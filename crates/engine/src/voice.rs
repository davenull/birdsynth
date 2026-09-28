//! One voice: three wavetable oscillators, Filter 1 and four envelopes,
//! with every modulatable parameter resolved through the matrix.

use wt_dsp::math;
use wt_dsp::rng::Rng;

use crate::env::{Env, EnvTimes, Stage};
use crate::filter::{FilterParams, FilterState};
use crate::modmatrix::{Matrix, NONE, Sources};
use crate::osc::unison::{MAX_LANES, UniParams};
use crate::osc::{OscSettings, OscVoice, PITCH_HARMONICS, PITCH_RATIO, harmonic};
use crate::params::ParamStore;
use crate::spec::params as p;
use crate::spec::protocol::{MAX_MOD_SLOTS, OSC_COUNT, SUB_BLOCK as N, source};
use crate::tables::Tables;

/// Values shared by every voice for one sub-block.
pub struct VoiceCtx<'a> {
    pub sr: f32,
    pub params: &'a ParamStore,
    pub matrix: &'a Matrix,
    pub tables: &'a Tables,
    pub env: [EnvTimes; 4],
    /// Pitch bend in semitones, and the raw wheel position (-1..1).
    pub bend: f32,
    pub bend_raw: f32,
    pub modwheel: f32,
    pub aftertouch: f32,
    pub active_voices: f32,
    pub scalar: bool,
}

/// Per-stage signals of the focused voice, mono.
pub struct VoiceTaps {
    pub osc: [[f32; N]; OSC_COUNT],
    pub sum: [f32; N],
    pub filter: [f32; N],
    pub out: [f32; N],
}

impl Default for VoiceTaps {
    fn default() -> Self {
        VoiceTaps { osc: [[0.0; N]; OSC_COUNT], sum: [0.0; N], filter: [0.0; N], out: [0.0; N] }
    }
}

/// How a note starts.
pub struct Start<'a> {
    pub note: u8,
    pub channel: u8,
    pub velocity: f32,
    pub note_id: u32,
    pub offset: usize,
    pub age: u64,
    pub index: u8,
    /// Pitch to glide from, and the glide length in samples.
    pub glide_from: Option<f32>,
    pub glide_len: f32,
    pub glide_curve: f32,
    /// Start phase per oscillator (fraction of a cycle), random share, and phase memory.
    pub phase: [f32; OSC_COUNT],
    pub rand_phase: [f32; OSC_COUNT],
    pub memory: [Option<&'a [u32; MAX_LANES]>; OSC_COUNT],
    pub rng: &'a mut Rng,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Voice {
    pub active: bool,
    pub note: u8,
    pub channel: u8,
    pub note_id: u32,
    pub velocity: f32,
    pub release_vel: f32,
    /// Start order; higher is newer.
    pub age: u64,
    pub released: bool,
    /// Key is up but the sustain pedal holds the note.
    pub sustained: bool,
    pub env: [Env; 4],
    pub osc: [OscVoice; OSC_COUNT],
    filt: FilterState,
    index: u8,
    rand: [f32; 3],
    pitch: f32,
    glide_from: f32,
    glide_to: f32,
    glide_pos: f32,
    glide_len: f32,
    glide_curve: f32,
    mod_off: ModOffsets,
    amp: f32,
    start: usize,
    kill_gain: f32,
    kill_step: f32,
    level_prev: [f32; OSC_COUNT],
    pan_prev: [(f32, f32); OSC_COUNT],
    osc_primed: [bool; OSC_COUNT],
    /// Last resolved values, reported for the focused voice.
    pub tel_wt_pos: [f32; OSC_COUNT],
    pub tel_cutoff: f32,
}

#[derive(Clone, Copy, Debug)]
struct ModOffsets([f32; MAX_MOD_SLOTS]);

impl Default for ModOffsets {
    fn default() -> Self {
        ModOffsets([0.0; MAX_MOD_SLOTS])
    }
}

impl Voice {
    pub fn start(&mut self, s: Start) {
        let mut v = Voice {
            active: true,
            note: s.note,
            channel: s.channel,
            note_id: s.note_id,
            velocity: s.velocity,
            age: s.age,
            index: s.index,
            kill_gain: 1.0,
            start: s.offset.min(N - 1),
            ..Voice::default()
        };
        v.rand = [s.rng.next_f32(), s.rng.next_f32(), (s.rng.next_u32() % 8) as f32 / 7.0];
        for o in 0..OSC_COUNT {
            v.osc[o].start(s.phase[o], s.rand_phase[o], s.memory[o], s.rng);
        }
        for e in v.env.iter_mut() {
            e.trigger();
        }
        let target = s.note as f32;
        v.pitch = s.glide_from.unwrap_or(target);
        v.glide_to = target;
        v.glide_from = v.pitch;
        v.glide_curve = s.glide_curve;
        v.glide_len = if s.glide_from.is_some() { s.glide_len } else { 0.0 };
        v.glide_pos = if v.glide_len > 0.0 { 0.0 } else { 1.0 };
        *self = v;
    }

    /// Move to a new note without restarting the voice (mono and legato).
    /// `retrigger` restarts the envelopes from their current levels.
    pub fn retarget(&mut self, note: u8, note_id: u32, velocity: f32, glide_len: f32, retrigger: bool) {
        self.glide_from = self.pitch;
        self.glide_to = note as f32;
        self.glide_len = glide_len;
        self.glide_pos = if glide_len > 0.0 { 0.0 } else { 1.0 };
        if glide_len <= 0.0 {
            self.pitch = self.glide_to;
        }
        self.note = note;
        self.note_id = note_id;
        self.velocity = velocity;
        self.released = false;
        self.sustained = false;
        if retrigger {
            for e in self.env.iter_mut() {
                e.trigger();
            }
        }
    }

    pub fn release(&mut self, velocity: f32) {
        self.released = true;
        self.sustained = false;
        self.release_vel = velocity;
        for e in self.env.iter_mut() {
            e.release();
        }
    }

    /// Fade out over `samples`, then free the slot (stealing, all-sound-off).
    pub fn kill(&mut self, samples: f32) {
        if self.kill_step == 0.0 {
            self.kill_step = 1.0 / samples.max(1.0);
        }
    }

    #[inline]
    pub fn killing(&self) -> bool {
        self.kill_step > 0.0
    }

    /// Current pitch in semitones (glide included).
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    fn advance_glide(&mut self, n: usize) -> f32 {
        if self.glide_pos < 1.0 {
            self.glide_pos = (self.glide_pos + n as f32 / self.glide_len.max(1.0)).min(1.0);
            let t = self.glide_pos;
            let c = self.glide_curve;
            let e = 1.0 + 3.0 * c.abs();
            let shape = if c < 0.0 {
                1.0 - (1.0 - t).powf(e)
            } else if c > 0.0 {
                t.powf(e)
            } else {
                t
            };
            self.pitch = self.glide_from + (self.glide_to - self.glide_from) * shape;
        } else {
            self.pitch = self.glide_to;
        }
        self.pitch
    }

    /// Render one sub-block and add it into `out`. `taps` is filled for the focused voice.
    pub fn render(&mut self, cx: &VoiceCtx, out: &mut [[f32; N]; 2], mut taps: Option<&mut VoiceTaps>) {
        let start = self.start;
        self.start = 0;
        let n = N - start;

        let mut envs = [0.0f32; 4];
        for (e, lv) in envs.iter_mut().enumerate() {
            *lv = self.env[e].advance(n as f32, &cx.env[e]);
        }
        let pitch = self.advance_glide(n) + cx.bend;

        // --- modulation
        let m = cx.matrix;
        if m.live() > 0 {
            let mut src: Sources = [0.0; source::COUNT];
            src[source::ENV_1 as usize..=source::ENV_4 as usize].copy_from_slice(&envs);
            src[source::VELOCITY as usize] = self.velocity;
            src[source::NOTE as usize] = ((self.pitch - 60.0) / 60.0).clamp(-1.0, 1.0);
            src[source::MOD_WHEEL as usize] = cx.modwheel;
            src[source::PITCH_BEND as usize] = cx.bend_raw;
            src[source::AFTERTOUCH as usize] = cx.aftertouch;
            src[source::RELEASE_VEL as usize] = self.release_vel;
            src[source::RAND_1 as usize] = self.rand[0];
            src[source::RAND_2 as usize] = self.rand[1];
            src[source::RAND_DISCRETE as usize] = self.rand[2];
            src[source::VOICE_INDEX as usize] = self.index as f32 / 31.0;
            src[source::ACTIVE_VOICES as usize] = cx.active_voices / 32.0;
            src[source::FIXED as usize] = 1.0;
            m.eval(&src, &mut self.mod_off.0);
        }
        let off = self.mod_off.0;
        let prm = cx.params;
        let res = |id: u16| -> f32 {
            let d = m.dest_of[id as usize];
            if d == NONE { prm.plain(id) } else { prm.modded(id, off[d as usize]) }
        };

        // --- oscillators
        let filter_on = res(p::FILTER_ENABLE[0]) >= 0.5;
        let mut fbus = [[0.0f32; N]; 2];
        let mut dbus = [[0.0f32; N]; 2];
        let mut sum_tap = [0.0f32; N];
        let osc_a_semis = res(p::OSC_OCTAVE[0]) * 12.0 + res(p::OSC_FINE[0]) * 0.01 + res(p::OSC_COARSE[0]);
        for o in 0..OSC_COUNT {
            if res(p::OSC_ENABLE[o]) < 0.5 {
                self.osc_primed[o] = false;
                continue;
            }
            let mode = res(p::OSC_PITCH_MODE[o]) as u8;
            let semi = res(p::OSC_SEMI[o]);
            let base = res(p::OSC_OCTAVE[o]) * 12.0 + res(p::OSC_FINE[o]) * 0.01 + res(p::OSC_COARSE[o]);
            let hz = match mode {
                PITCH_HARMONICS => math::note_to_hz((pitch + base) as f64) as f32 * harmonic(semi),
                PITCH_RATIO => math::note_to_hz((pitch + osc_a_semis) as f64) as f32 * harmonic(semi),
                _ => math::note_to_hz((pitch + base + semi) as f64) as f32,
            };
            let settings = OscSettings {
                hz,
                pos: res(p::OSC_WT_POS[o]),
                smooth: res(p::OSC_WT_SMOOTH[o]) >= 0.5,
                uni: UniParams {
                    voices: res(p::OSC_UNISON[o]) as usize,
                    detune: res(p::OSC_DETUNE[o]),
                    blend: res(p::OSC_BLEND[o]),
                    width: res(p::OSC_WIDTH[o]),
                    range: res(p::OSC_UNI_RANGE[o]),
                    mode: res(p::OSC_UNI_MODE[o]) as u8,
                    stack: res(p::OSC_UNI_STACK[o]) as u8,
                },
                wt_spread: res(p::OSC_WT_SPREAD[o]),
                warp_spread: res(p::OSC_WARP_SPREAD[o]),
                w1: (res(p::OSC_WARP1_MODE[o]) as u8, res(p::OSC_WARP1_AMOUNT[o])),
                w2: (res(p::OSC_WARP2_MODE[o]) as u8, res(p::OSC_WARP2_AMOUNT[o])),
            };
            self.tel_wt_pos[o] = settings.pos;
            let (table, frames) = cx.tables.get(o);
            let mut obuf = [[0.0f32; N]; 2];
            self.osc[o].render(&settings, table, frames, cx.sr, start, cx.scalar, &mut obuf);

            // level and pan, ramped across the sub-block
            let lv1 = res(p::OSC_LEVEL[o]);
            let pan1 = math::balance(res(p::OSC_PAN[o]));
            let (lv0, pan0) = if self.osc_primed[o] { (self.level_prev[o], self.pan_prev[o]) } else { (lv1, pan1) };
            self.level_prev[o] = lv1;
            self.pan_prev[o] = pan1;
            self.osc_primed[o] = true;
            let to_filter = filter_on && res(p::OSC_FILTER[o]) >= 0.5;
            let bus = if to_filter { &mut fbus } else { &mut dbus };
            let inv = 1.0 / N as f32;
            for i in start..N {
                let t = (i + 1) as f32 * inv;
                let lv = lv0 + (lv1 - lv0) * t;
                let gl = (pan0.0 + (pan1.0 - pan0.0) * t) * lv;
                let gr = (pan0.1 + (pan1.1 - pan0.1) * t) * lv;
                let (l, r) = (obuf[0][i] * gl, obuf[1][i] * gr);
                bus[0][i] += l;
                bus[1][i] += r;
                let mono = 0.5 * (l + r);
                sum_tap[i] += mono;
                if let Some(t) = taps.as_deref_mut() {
                    t.osc[o][i] = mono;
                }
            }
        }

        // --- filter
        if filter_on {
            let kt = res(p::FILTER_KEYTRACK[0]);
            let cutoff = res(p::FILTER_CUTOFF[0]) * math::semis_to_ratio(kt * (self.pitch - 60.0));
            self.tel_cutoff = cutoff;
            let fp = FilterParams {
                kind: res(p::FILTER_TYPE[0]) as u8,
                cutoff,
                res: res(p::FILTER_RES[0]),
                drive: res(p::FILTER_DRIVE[0]),
                mix: res(p::FILTER_MIX[0]),
            };
            self.filt.process(&fp, cx.sr, &mut fbus, start);
            for ch in 0..2 {
                for i in start..N {
                    dbus[ch][i] += fbus[ch][i];
                }
            }
            if let Some(t) = taps.as_deref_mut() {
                for i in start..N {
                    t.filter[i] = 0.5 * (fbus[0][i] + fbus[1][i]);
                }
            }
        } else {
            self.filt.reset();
        }

        // --- amp: Env 1, with the kill fade while being stolen
        let kill_end = if self.killing() {
            let k = self.kill_gain - self.kill_step * n as f32;
            if k < 1e-4 { 0.0 } else { k }
        } else {
            1.0
        };
        let amp0 = self.amp * if self.killing() { self.kill_gain } else { 1.0 };
        let amp1 = envs[0] * kill_end;
        let damp = (amp1 - amp0) / n as f32;
        let mut a = amp0;
        for i in start..N {
            a += damp;
            let (l, r) = (dbus[0][i] * a, dbus[1][i] * a);
            out[0][i] += l;
            out[1][i] += r;
            if let Some(t) = taps.as_deref_mut() {
                t.out[i] = 0.5 * (l + r);
                t.sum[i] = sum_tap[i];
            }
        }
        self.amp = envs[0];
        if self.killing() {
            self.kill_gain = kill_end;
        }
        if kill_end <= 0.0 || (self.env[0].stage == Stage::Idle && amp1 == 0.0) {
            self.active = false;
        }
    }
}
