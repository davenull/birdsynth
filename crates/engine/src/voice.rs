//! One voice: three wavetable oscillators, a sub, noise, two filters and
//! four envelopes, with every modulatable parameter resolved through the
//! matrix, and ten LFOs.
//!
//! The voice renders at `os` times the output rate (oversampling). Sources
//! are routed per sample into the filter inputs, the main bus, the direct
//! bus and the two FX-bus sends; the amp envelope then scales every bus.
//!
//! Cross-modulating oscillators (FM, PD, AM, RM warps) render sample by
//! sample in a fixed order: sources before the oscillators they modulate
//! where possible, the rest (and self-modulation) from the previous sample.
//! When a filter is a source, routing and filtering move into that loop too.

use wt_dsp::math;
use wt_dsp::rng::Rng;
use wt_dsp::warp::Remap;

use crate::env::{Env, EnvTimes, Stage};
use crate::filter::{Coeffs, FilterParams, FilterState, MAX_N};
use crate::lfo::{self, LfoSettings, LfoState, Shape};
use crate::modmatrix::{Matrix, NONE, Sources};
use crate::osc::kernel::{self, Kernel, XIn};
use crate::osc::sub::SubOsc;
use crate::osc::unison::{MAX_LANES, UniParams};
use crate::osc::{self, OscSettings, OscVoice, PITCH_HARMONICS, PITCH_RATIO, XKind, XMod, harmonic};
use crate::params::ParamStore;
use crate::samples::{self, Player, Samples};
use crate::sources::{OscAssets, OscShared, SrcIn, SrcState, TYPE_WAVETABLE};
use crate::spec::params as p;
use crate::spec::protocol::{MAX_MOD_SLOTS, OSC_COUNT, SUB_BLOCK as N, source};
use crate::tables::Tables;

pub const LFOS: usize = 10;
pub const MACROS: usize = 8;
/// Oscillators A-C, then the sub and the noise.
const SOURCES: usize = OSC_COUNT + 2;
const SUB: usize = OSC_COUNT;

pub const ROUTE_FILTERS: u8 = 0;
pub const ROUTE_MAIN: u8 = 1;
pub const ROUTE_DIRECT: u8 = 2;
pub const ROUTE_NONE: u8 = 3;

/// Values shared by every voice for one sub-block.
pub struct VoiceCtx<'a> {
    pub sr: f32,
    /// Oversampling factor for the voice path (1, 2 or 4).
    pub os: usize,
    pub params: &'a ParamStore,
    pub matrix: &'a Matrix,
    pub tables: &'a Tables,
    /// Each oscillator's remap curve.
    pub remaps: &'a [Remap; OSC_COUNT],
    pub samples: &'a Samples,
    /// Recordings, multisamples and spectral analyses for the oscillator types that play them.
    pub assets: &'a OscAssets,
    pub saw: &'a [f32],
    pub triangle: &'a [f32],
    pub lfo_shapes: &'a [Shape; LFOS],
    pub lfo_paths: &'a [Shape; LFOS],
    /// Global (shared) LFO outputs this sub-block, before rise and delay.
    pub lfo_global: [(f32, f32); LFOS],
    /// Seconds per sub-block, and the absolute beat position.
    pub dt: f32,
    pub beat: f64,
    /// Pitch bend in semitones, and the raw wheel position (-1..1).
    pub bend: f32,
    pub bend_raw: f32,
    pub modwheel: f32,
    pub aftertouch: f32,
    pub active_voices: f32,
    pub scalar: bool,
    /// 120 / tempo (BPM-synced envelopes scale their times by this).
    pub bpm_scale: f32,
    pub env_rate: f32,
    pub lfo_rate: f32,
    pub bpm: f32,
    pub serial: bool,
    /// Master tune, in semitones from A = 440 Hz.
    pub tune: f32,
    /// False for the aliasing demo: every table read at full detail.
    pub bandlimit: bool,
}

/// The buses a voice mixes into, at the oversampled rate.
pub struct Buses {
    pub main: [[f32; MAX_N]; 2],
    pub direct: [[f32; MAX_N]; 2],
    pub bus1: [[f32; MAX_N]; 2],
    pub bus2: [[f32; MAX_N]; 2],
}

impl Default for Buses {
    fn default() -> Self {
        Buses { main: [[0.0; MAX_N]; 2], direct: [[0.0; MAX_N]; 2], bus1: [[0.0; MAX_N]; 2], bus2: [[0.0; MAX_N]; 2] }
    }
}

impl Buses {
    pub fn clear(&mut self, len: usize) {
        for b in [&mut self.main, &mut self.direct, &mut self.bus1, &mut self.bus2] {
            for ch in b.iter_mut() {
                ch[..len].fill(0.0);
            }
        }
    }
}

/// Working buffers for one voice render, shared by all voices (they render
/// one at a time). Each render clears only the ranges it accumulates into.
pub struct Scratch {
    obuf: [Stereo; OSC_COUNT],
    f_in: [Stereo; 2],
    main: Stereo,
    direct: Stereo,
    b1: Stereo,
    b2: Stereo,
    fmono: [[f32; MAX_N]; 2],
    sub: [f32; MAX_N],
    noise: [f32; MAX_N],
    sum_tap: [f32; MAX_N],
}

impl Default for Scratch {
    fn default() -> Self {
        let z = [[0.0; MAX_N]; 2];
        Scratch { obuf: [z; OSC_COUNT], f_in: [z; 2], main: z, direct: z, b1: z, b2: z, fmono: z, sub: [0.0; MAX_N], noise: [0.0; MAX_N], sum_tap: [0.0; MAX_N] }
    }
}

/// Per-stage signals of the focused voice, mono, at the output rate.
pub struct VoiceTaps {
    pub osc: [[f32; N]; OSC_COUNT],
    pub sub: [f32; N],
    pub noise: [f32; N],
    pub sum: [f32; N],
    pub filter: [f32; N],
    pub filter2: [f32; N],
    pub out: [f32; N],
}

impl Default for VoiceTaps {
    fn default() -> Self {
        VoiceTaps { osc: [[0.0; N]; OSC_COUNT], sub: [0.0; N], noise: [0.0; N], sum: [0.0; N], filter: [0.0; N], filter2: [0.0; N], out: [0.0; N] }
    }
}

/// How a note starts.
pub struct Start<'a> {
    pub note: u8,
    /// The note's pitch (fractional note number) under the current tuning.
    pub pitch: f32,
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
    /// Noise start and random start.
    pub noise_start: (f32, f32),
    pub noise_frames: usize,
    /// Per-LFO settings for LFOs that restart with the note.
    pub lfo: [Option<LfoSettings>; LFOS],
    /// Envelopes that restart from zero rather than their current level.
    pub env_from_zero: [bool; 4],
    pub rng: &'a mut Rng,
}

/// Start and end values of one routing line across a sub-block.
#[derive(Clone, Copy, Debug, Default)]
struct Line {
    gl: (f32, f32),
    gr: (f32, f32),
    bal: (f32, f32),
    s1: (f32, f32),
    s2: (f32, f32),
    route: u8,
    /// The source is playing (off sources are skipped).
    on: bool,
}

#[derive(Clone, Copy, Debug, Default)]
struct LineState {
    gl: f32,
    gr: f32,
    bal: f32,
    s1: f32,
    s2: f32,
    primed: bool,
}

#[derive(Clone, Copy, Debug)]
struct Arr64([f32; MAX_MOD_SLOTS]);

impl Default for Arr64 {
    fn default() -> Self {
        Arr64([0.0; MAX_MOD_SLOTS])
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Voice {
    /// Which slot of the engine's voice list this is (fixed).
    pub slot: u16,
    pub active: bool,
    pub note: u8,
    pub channel: u8,
    pub note_id: u32,
    pub velocity: f32,
    pub release_vel: f32,
    pub poly_at: f32,
    /// Start order; higher is newer.
    pub age: u64,
    pub released: bool,
    /// Key is up but the sustain pedal holds the note.
    pub sustained: bool,
    pub env: [Env; 4],
    pub osc: [OscVoice; OSC_COUNT],
    /// The recording types' state, per oscillator.
    src: [SrcState; OSC_COUNT],
    sub: SubOsc,
    noise: Player,
    lfo: [LfoState; LFOS],
    /// Seconds since the note started.
    time: f32,
    filt: [FilterState; 2],
    index: u8,
    rand: [f32; 3],
    pitch: f32,
    glide_from: f32,
    glide_to: f32,
    glide_pos: f32,
    glide_len: f32,
    glide_curve: f32,
    mod_off: Arr64,
    amp: f32,
    start: usize,
    kill_gain: f32,
    kill_step: f32,
    lines: [LineState; SOURCES],
    fline: [(f32, f32, bool); 2],
    /// Cross-mod memory: each oscillator's lane outputs and each filter's
    /// mono output from the previous sample.
    xm_lanes: [[f32; MAX_LANES]; OSC_COUNT],
    xm_filter: [f32; 2],
    /// Last sub and noise samples (audio-rate matrix sources).
    xm_sub_noise: [f32; 2],
    /// Last resolved values, reported for the focused voice.
    pub tel_wt_pos: [f32; OSC_COUNT],
    pub tel_cutoff: f32,
    pub tel_lfo: [(f32, f32, f32); LFOS],
    pub tel_macro: [f32; MACROS],
    pub tel_mod: Arr64Pub,
}

/// The focused voice's resolved value of each matrix destination.
#[derive(Clone, Copy, Debug)]
pub struct Arr64Pub(pub [f32; MAX_MOD_SLOTS]);

impl Default for Arr64Pub {
    fn default() -> Self {
        Arr64Pub([0.0; MAX_MOD_SLOTS])
    }
}

impl Voice {
    pub fn start(&mut self, s: Start) {
        let mut v = Voice {
            slot: self.slot,
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
            v.src[o].fresh = true;
        }
        v.sub.start(0);
        v.noise.start(s.noise_frames.max(1), s.noise_start.0, s.noise_start.1, s.rng);
        for (i, set) in s.lfo.iter().enumerate() {
            if let Some(set) = set {
                v.lfo[i].trigger(set, s.rng.next_u32());
            }
        }
        for (e, env) in v.env.iter_mut().enumerate() {
            if s.env_from_zero[e] {
                env.trigger_from_zero();
            } else {
                env.trigger();
            }
        }
        let target = s.pitch;
        v.pitch = s.glide_from.unwrap_or(target);
        v.glide_to = target;
        v.glide_from = v.pitch;
        v.glide_curve = s.glide_curve;
        v.glide_len = if s.glide_from.is_some() { s.glide_len } else { 0.0 };
        v.glide_pos = if v.glide_len > 0.0 { 0.0 } else { 1.0 };
        *self = v;
    }

    /// Move to a new note without restarting the voice (mono and legato).
    /// `retrigger[e]` restarts envelope e (from its level, or from zero).
    #[allow(clippy::too_many_arguments)]
    pub fn retarget(&mut self, note: u8, pitch: f32, note_id: u32, velocity: f32, glide_len: f32, retrigger: [bool; 4], from_zero: [bool; 4]) {
        self.glide_from = self.pitch;
        self.glide_to = pitch;
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
        for e in 0..4 {
            if retrigger[e] {
                if from_zero[e] {
                    self.env[e].trigger_from_zero();
                } else {
                    self.env[e].trigger();
                }
            }
        }
    }

    /// Restart the LFOs that retrigger with notes (mono legato re-strikes).
    pub fn retrigger_lfos(&mut self, lfo: &[Option<LfoSettings>; LFOS], rng: &mut Rng) {
        for (i, set) in lfo.iter().enumerate() {
            if let Some(set) = set {
                self.lfo[i].trigger(set, rng.next_u32());
            }
        }
        self.time = 0.0;
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

    /// Normalized offset the matrix gives destination index `d` in this voice.
    pub fn mod_offset(&self, d: usize) -> f32 {
        self.mod_off.0[d]
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

    /// Render one sub-block into `buses`. `taps` is filled for the focused voice.
    /// Where oscillator `o` is in its recording, 0..1 (-1: it isn't playing one).
    pub fn play_pos(&self, o: usize) -> f32 {
        self.src[o].play
    }

    pub fn render(&mut self, cx: &VoiceCtx, buses: &mut Buses, scratch: &mut Scratch, shared: &mut OscShared, taps: Option<&mut VoiceTaps>) {
        let Scratch { obuf, f_in, main, direct, b1, b2, fmono, sub: sub_buf, noise: noise_buf, sum_tap } = scratch;
        let os = cx.os;
        let start = self.start;
        self.start = 0;
        let n = N - start;
        let len = N * os;
        let st = start * os;
        let sr = cx.sr * os as f32;
        self.time += n as f32 / cx.sr;

        // --- sources for the matrix (modulator values at the sub-block's end)
        let m = cx.matrix;
        let prm = cx.params;
        let base = |id: u16| prm.plain(id);

        // envelopes use their own (possibly modulated) times, so they're
        // advanced after the matrix; the matrix sees last sub-block's values
        let mut src: Sources = [0.0; source::COUNT];
        for e in 0..4 {
            src[source::ENV_1 as usize + e] = self.env[e].level;
        }
        src[source::VELOCITY as usize] = self.velocity;
        src[source::NOTE as usize] = ((self.pitch - 60.0) / 60.0).clamp(-1.0, 1.0);
        src[source::MOD_WHEEL as usize] = cx.modwheel;
        src[source::PITCH_BEND as usize] = cx.bend_raw;
        src[source::AFTERTOUCH as usize] = cx.aftertouch;
        src[source::POLY_AT as usize] = self.poly_at;
        src[source::RELEASE_VEL as usize] = self.release_vel;
        src[source::RAND_1 as usize] = self.rand[0];
        src[source::RAND_2 as usize] = self.rand[1];
        src[source::RAND_DISCRETE as usize] = self.rand[2];
        src[source::VOICE_INDEX as usize] = self.index as f32 / 31.0;
        src[source::ACTIVE_VOICES as usize] = cx.active_voices / 32.0;
        src[source::FIXED as usize] = 1.0;
        let (vc1, vc2) = voice_control(&base, self.time, cx.bpm);
        src[source::VOICE_MOD_1 as usize] = vc1;
        src[source::VOICE_MOD_2 as usize] = vc2;
        for i in 0..MACROS {
            src[source::MACRO_1 as usize + i] = prm.norm(p::MACRO_VALUE[i]);
        }
        src[source::OSC_A as usize..=source::FILTER_2 as usize].copy_from_slice(&[
            self.xm_lanes[0][0],
            self.xm_lanes[1][0],
            self.xm_lanes[2][0],
            self.xm_sub_noise[0],
            self.xm_sub_noise[1],
            self.xm_filter[0],
            self.xm_filter[1],
        ]);

        // LFOs: settings resolve through the matrix too, so do a first pass
        // for everything, then refine with the LFO values.
        if m.live() > 0 {
            m.eval(&src, &mut self.mod_off.0);
        }
        let mut off = self.mod_off.0;
        let res_off = |id: u16, off: &[f32; MAX_MOD_SLOTS]| -> f32 {
            let d = m.dest_of[id as usize];
            if d == NONE { prm.plain(id) } else { prm.modded(id, off[d as usize]) }
        };
        // unrouted LFOs are skipped, except in the focused voice (its LFOs are displayed)
        let focused = taps.is_some();
        for i in 0..LFOS {
            let used = m.uses(source::LFO_1 + i as u8) || m.uses(source::LFO_1_Y + i as u8);
            if !used && !focused {
                continue;
            }
            let set = lfo_settings(cx, |id| res_off(id, &off), i);
            let (x, y) = if base(p::LFO_POLY[i]) >= 0.5 && set.mode != lfo::MODE_FREE {
                self.lfo[i].advance(&set, &cx.lfo_shapes[i], &cx.lfo_paths[i], cx.dt, cx.beat)
            } else {
                cx.lfo_global[i]
            };
            // rise and delay count from this voice's note
            let amp = if self.time < set.delay { 0.0 } else if set.rise > 0.0 { ((self.time - set.delay) / set.rise).min(1.0) } else { 1.0 };
            src[source::LFO_1 as usize + i] = x * amp;
            src[source::LFO_1_Y as usize + i] = y * amp;
            self.tel_lfo[i] = (x * amp, y * amp, if base(p::LFO_POLY[i]) >= 0.5 && set.mode != lfo::MODE_FREE { self.lfo[i].phase() } else { -1.0 });
        }
        if m.live() > 0 {
            // macros first (they can be modulated, then act as sources)
            if (0..MACROS).any(|i| m.dest_of[p::MACRO_VALUE[i] as usize] != NONE) {
                m.eval(&src, &mut off);
                for i in 0..MACROS {
                    let d = m.dest_of[p::MACRO_VALUE[i] as usize];
                    if d != NONE {
                        src[source::MACRO_1 as usize + i] = (prm.norm(p::MACRO_VALUE[i]) + off[d as usize]).clamp(0.0, 1.0);
                    }
                }
            }
            m.eval(&src, &mut self.mod_off.0);
        }
        for i in 0..MACROS {
            self.tel_macro[i] = src[source::MACRO_1 as usize + i];
        }
        let off = self.mod_off.0;
        let res = |id: u16| -> f32 {
            let d = m.dest_of[id as usize];
            if d == NONE { prm.plain(id) } else { prm.modded(id, off[d as usize]) }
        };
        for d in 0..m.dests {
            let id = m.dest_param[d];
            self.tel_mod.0[d] = (prm.norm(id) + off[d]).clamp(0.0, 1.0);
        }

        // --- envelopes and pitch
        let mut envs = [0.0f32; 4];
        for (e, lv) in envs.iter_mut().enumerate() {
            let t = env_times(cx, &res, e);
            *lv = self.env[e].advance(n as f32, &t);
        }
        let pitch = self.advance_glide(n) + cx.bend + cx.tune;

        // --- sub and noise (block)
        let sub_on = res(p::SUB_ENABLE) >= 0.5;
        if sub_on {
            let hz = math::note_to_hz((pitch + 12.0 * res(p::SUB_OCTAVE)) as f64) as f32;
            self.sub.render(res(p::SUB_SHAPE) as u8, hz, sr, cx.saw, cx.triangle, st, len, sub_buf);
        }
        let noise_on = res(p::NOISE_ENABLE) >= 0.5;
        if noise_on {
            let s = cx.samples.get(samples::NOISE);
            let key = if res(p::NOISE_KEYTRACK) >= 0.5 { (self.pitch - 60.0) / 12.0 } else { 0.0 };
            let step = s.map_or(1.0, |s| s.rate as f64 / sr as f64) * ((res(p::NOISE_PITCH) / 12.0 + key) as f64).exp2();
            self.noise.render(s, step, res(p::NOISE_ONESHOT) >= 0.5, st, len, noise_buf);
        }

        self.xm_sub_noise = [if sub_on { sub_buf[len - 1] } else { 0.0 }, if noise_on { noise_buf[len - 1] } else { 0.0 }];

        // --- oscillator settings and cross-modulation
        let filter_on = [res(p::FILTER_ENABLE[0]) >= 0.5, res(p::FILTER_ENABLE[1]) >= 0.5];
        let osc_a_semis = res(p::OSC_OCTAVE[0]) * 12.0 + res(p::OSC_FINE[0]) * 0.01 + res(p::OSC_COARSE[0]);
        let mut enabled = [false; OSC_COUNT];
        let mut kinds = [TYPE_WAVETABLE; OSC_COUNT];
        let mut opitch = [pitch; OSC_COUNT];
        let mut tuning = [0.0f32; OSC_COUNT];
        let mut uni = [UniParams::default(); OSC_COUNT];
        let mut settings: [Option<OscSettings>; OSC_COUNT] = [None; OSC_COUNT];
        let mut xm: [[Option<XMod>; 2]; OSC_COUNT] = [[None; 2]; OSC_COUNT];
        for o in 0..OSC_COUNT {
            // which keys and velocities it plays (outside them: silent, folded or warped in)
            let key_shift = key_range(&res, o, self.note, self.velocity);
            if res(p::OSC_ENABLE[o]) < 0.5 || key_shift.is_none() {
                self.osc[o].primed = false;
                self.lines[o].primed = false;
                continue;
            }
            let pitch = pitch + key_shift.unwrap_or(0.0);
            opitch[o] = pitch;
            enabled[o] = true;
            let mode = res(p::OSC_PITCH_MODE[o]) as u8;
            let semi = res(p::OSC_SEMI[o]);
            let base_st = res(p::OSC_OCTAVE[o]) * 12.0 + res(p::OSC_FINE[o]) * 0.01 + res(p::OSC_COARSE[o]);
            kinds[o] = res(p::OSC_TYPE[o]) as u8;
            if kinds[o] != TYPE_WAVETABLE {
                // a recording: tuned in semitones; its warps and wavetable settings don't apply
                tuning[o] = base_st + semi;
                uni[o] = UniParams {
                    voices: res(p::OSC_UNISON[o]) as usize,
                    detune: res(p::OSC_DETUNE[o]),
                    blend: res(p::OSC_BLEND[o]),
                    width: res(p::OSC_WIDTH[o]),
                    range: res(p::OSC_UNI_RANGE[o]),
                    mode: res(p::OSC_UNI_MODE[o]) as u8,
                    stack: res(p::OSC_UNI_STACK[o]) as u8,
                };
                self.osc[o].primed = false;
                continue;
            }
            let hz = match mode {
                PITCH_HARMONICS => math::note_to_hz((pitch + base_st) as f64) as f32 * harmonic(semi),
                PITCH_RATIO => math::note_to_hz((pitch + osc_a_semis) as f64) as f32 * harmonic(semi),
                _ => math::note_to_hz((pitch + base_st + semi) as f64) as f32,
            };
            let w1 = (res(p::OSC_WARP1_MODE[o]) as u8, res(p::OSC_WARP1_AMOUNT[o]));
            let w2 = (res(p::OSC_WARP2_MODE[o]) as u8, res(p::OSC_WARP2_AMOUNT[o]));
            xm[o] = [osc::xmod(w1.0, w1.1), osc::xmod(w2.0, w2.1)];
            let xs = xm[o].iter().flatten().map(osc::xstretch).product::<f32>();
            settings[o] = Some(OscSettings {
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
                w1,
                w2,
                xstretch: xs,
                bandlimit: cx.bandlimit,
            });
            self.tel_wt_pos[o] = res(p::OSC_WT_POS[o]);
        }
        // a cross-mod needs its source oscillator switched on
        for o in 0..OSC_COUNT {
            for x in xm[o].iter_mut() {
                if let Some(v) = x {
                    let ok = match v.src {
                        osc::SRC_A | osc::SRC_B | osc::SRC_C => enabled[v.src as usize] || v.kind == XKind::PdSelf,
                        osc::SRC_SUB => sub_on,
                        osc::SRC_NOISE => noise_on,
                        _ => true,
                    };
                    if !ok {
                        *x = None;
                    }
                }
            }
        }
        let fused = xm.iter().flatten().any(|x| x.is_some());
        let filter_src = xm.iter().flatten().flatten().any(|x| x.src == osc::SRC_F1 || x.src == osc::SRC_F2);

        // --- routing lines (start and end gains for each source)
        let mut lines = [Line::default(); SOURCES];
        let src_keys: [(u16, u16, u16, u16, u16, u16); SOURCES] = [
            (p::OSC_LEVEL[0], p::OSC_PAN[0], p::OSC_BALANCE[0], p::OSC_SEND1[0], p::OSC_SEND2[0], p::OSC_ROUTE[0]),
            (p::OSC_LEVEL[1], p::OSC_PAN[1], p::OSC_BALANCE[1], p::OSC_SEND1[1], p::OSC_SEND2[1], p::OSC_ROUTE[1]),
            (p::OSC_LEVEL[2], p::OSC_PAN[2], p::OSC_BALANCE[2], p::OSC_SEND1[2], p::OSC_SEND2[2], p::OSC_ROUTE[2]),
            (p::SUB_LEVEL, p::SUB_PAN, p::SUB_BALANCE, p::SUB_SEND1, p::SUB_SEND2, p::SUB_ROUTE),
            (p::NOISE_LEVEL, p::NOISE_PAN, p::NOISE_BALANCE, p::NOISE_SEND1, p::NOISE_SEND2, p::NOISE_ROUTE),
        ];
        let live = [enabled[0], enabled[1], enabled[2], sub_on, noise_on];
        for s in 0..SOURCES {
            if !live[s] {
                self.lines[s].primed = false;
                continue;
            }
            let (kl, kp, kb, k1, k2, kr) = src_keys[s];
            let lv = res(kl);
            let (pl, pr) = math::balance(res(kp));
            let now = LineState { gl: lv * pl, gr: lv * pr, bal: res(kb), s1: res(k1), s2: res(k2), primed: true };
            let was = if self.lines[s].primed { self.lines[s] } else { now };
            self.lines[s] = now;
            let mut route = res(kr) as u8;
            if route == ROUTE_FILTERS && !filter_on[0] && !filter_on[1] {
                route = ROUTE_MAIN; // nothing to filter: go straight to the main bus
            }
            lines[s] = Line { gl: (was.gl, now.gl), gr: (was.gr, now.gr), bal: (was.bal, now.bal), s1: (was.s1, now.s1), s2: (was.s2, now.s2), route, on: true };
        }
        let on = || lines.iter().filter(|l| l.on);
        let uses_direct = on().any(|l| l.route == ROUTE_DIRECT);
        let uses_b1 = on().any(|l| l.s1 != (0.0, 0.0));
        let uses_b2 = on().any(|l| l.s2 != (0.0, 0.0));

        // --- filters: coefficients for this block
        let mut fcoef: [Option<Coeffs>; 2] = [None, None];
        let mut fout_gain = [((0.0f32, 0.0f32), (0.0f32, 0.0f32)); 2];
        for f in 0..2 {
            if !filter_on[f] {
                self.filt[f].reset();
                self.fline[f].2 = false;
                continue;
            }
            let kt = res(p::FILTER_KEYTRACK[f]);
            let cutoff = res(p::FILTER_CUTOFF[f]) * math::semis_to_ratio(kt * (self.pitch - 60.0));
            if f == 0 {
                self.tel_cutoff = cutoff;
            }
            let fp = FilterParams {
                kind: res(p::FILTER_TYPE[f]) as u8,
                cutoff,
                res: res(p::FILTER_RES[f]),
                drive: res(p::FILTER_DRIVE[f]),
                mix: res(p::FILTER_MIX[f]),
                var: res(p::FILTER_VAR[f]),
                stereo: res(p::FILTER_STEREO[f]),
            };
            fcoef[f] = Some(self.filt[f].prepare(&fp, sr, len));
            let lv = res(p::FILTER_LEVEL[f]);
            let (pl, pr) = math::balance(res(p::FILTER_PAN[f]));
            let now = (lv * pl, lv * pr);
            let was = if self.fline[f].2 { (self.fline[f].0, self.fline[f].1) } else { now };
            self.fline[f] = (now.0, now.1, true);
            fout_gain[f] = (was, now);
        }

        // --- render the oscillators
        let inv = 1.0 / len as f32;
        let clear = |b: &mut Stereo| {
            b[0][st..len].fill(0.0);
            b[1][st..len].fill(0.0);
        };
        if !fused {
            for o in 0..OSC_COUNT {
                if enabled[o] && kinds[o] == TYPE_WAVETABLE {
                    clear(&mut obuf[o]); // the block kernels add their lanes in
                }
            }
        }
        // filter outputs are valid when a filter runs (or feeds a cross-mod)
        let filters_run = filter_on[0] || filter_on[1] || filter_src;
        if filters_run {
            clear(&mut f_in[0]);
            clear(&mut f_in[1]);
        }
        clear(main);
        if uses_direct {
            clear(direct);
        }
        if uses_b1 {
            clear(b1);
        }
        if uses_b2 {
            clear(b2);
        }
        sum_tap[st..len].fill(0.0);

        // the recording types render as a block, whichever way the wavetables go
        for o in 0..OSC_COUNT {
            if !enabled[o] || kinds[o] == TYPE_WAVETABLE {
                self.src[o].play = -1.0;
            }
            if enabled[o] && kinds[o] != TYPE_WAVETABLE {
                let x = SrcIn {
                    kind: kinds[o],
                    osc: o,
                    slot: self.slot as usize,
                    note: self.note,
                    velocity: self.velocity,
                    held: !self.released,
                    pitch: opitch[o],
                    tuning: tuning[o],
                    bend: cx.bend,
                    sr,
                    layout: self.osc[o].layout(&uni[o]),
                    res: &res,
                };
                self.src[o].render(&x, cx.assets, shared, st, len, &mut obuf[o]);
                self.xm_lanes[o][0] = 0.5 * (obuf[o][0][len - 1] + obuf[o][1][len - 1]);
            }
        }
        if !fused {
            for o in 0..OSC_COUNT {
                if let Some(s) = &settings[o] {
                    let (table, frames) = cx.tables.get(o);
                    self.osc[o].render(s, table, frames, &cx.remaps[o], sr, st, len, cx.scalar, &mut obuf[o]);
                    // the matrix's audio-rate source: this block's last sample
                    self.xm_lanes[o][0] = 0.5 * (obuf[o][0][len - 1] + obuf[o][1][len - 1]);
                }
            }
        } else {
            // prepare kernels, then run sample by sample
            let mut kernels: [Option<Kernel>; OSC_COUNT] = [None, None, None];
            for o in 0..OSC_COUNT {
                if let Some(s) = &settings[o] {
                    let (table, frames) = cx.tables.get(o);
                    kernels[o] = Some(self.osc[o].prepare(s, table, frames, &cx.remaps[o], sr, len));
                }
            }
            let order = osc_order(&xm, &enabled);
            let mut inc_base = [0.0f32; OSC_COUNT];
            for o in 0..OSC_COUNT {
                if let Some(k) = &kernels[o] {
                    inc_base[o] = k.inc0;
                    for _ in 0..st {
                        inc_base[o] *= k.inc_step;
                    }
                }
            }
            let mut done = [false; OSC_COUNT];
            let mut cur = self.xm_lanes;
            for i in st..len {
                done.fill(false);
                // recordings are already rendered: this sample of theirs is the source value
                for o in 0..OSC_COUNT {
                    if enabled[o] && kinds[o] != TYPE_WAVETABLE {
                        cur[o][0] = 0.5 * (obuf[o][0][i] + obuf[o][1][i]);
                        done[o] = true;
                    }
                }
                for &o in order.iter().take(OSC_COUNT) {
                    let Some(k) = &kernels[o] else { continue };
                    let mut x = XIn::default();
                    for xmod in xm[o].iter().flatten() {
                        for l in 0..k.lanes {
                            let v = match xmod.src {
                                s @ (osc::SRC_A | osc::SRC_B | osc::SRC_C) if xmod.kind != XKind::PdSelf => {
                                    let s = s as usize;
                                    let lanes = settings[s].map_or(1, |q| q.uni.voices.clamp(1, MAX_LANES));
                                    let lane = l % lanes;
                                    if done[s] { cur[s][lane] } else { self.xm_lanes[s][lane] }
                                }
                                osc::SRC_SUB => sub_buf[i],
                                osc::SRC_NOISE => noise_buf[i],
                                osc::SRC_F1 => self.xm_filter[0],
                                osc::SRC_F2 => self.xm_filter[1],
                                _ => self.xm_lanes[o][l], // PD Self
                            };
                            let a = xmod.amount;
                            match xmod.kind {
                                XKind::Fm => x.fm[l] += osc::FM_DEPTH * a * v,
                                XKind::Pd => x.pd[l] += osc::PD_DEPTH * a * v,
                                XKind::PdSelf => x.pd[l] += a * v,
                                XKind::Am => x.gain[l] *= 1.0 - a + a * (v + 1.0) * 0.5,
                                XKind::Rm => x.gain[l] *= 1.0 - a + a * v,
                            }
                        }
                    }
                    let (l, r) = kernel::step(k, &mut self.osc[o].phase, i, inc_base[o], &x, &mut cur[o]);
                    obuf[o][0][i] = l;
                    obuf[o][1][i] = r;
                    inc_base[o] *= k.inc_step;
                    done[o] = true;
                }
                // everything computed this sample becomes "previous" for the next
                self.xm_lanes = cur;
                if filter_src {
                    let t = (i + 1) as f32 * inv;
                    let (osc_lr, sn) = sample_sources(obuf, sub_buf, noise_buf, i);
                    route_sample(&lines, t, &osc_lr, &sn, i, f_in, main, direct, b1, b2, sum_tap);
                    filter_sample(&mut self.filt, &fcoef, &fout_gain, cx.serial, t, i, f_in, main, fmono);
                    self.xm_filter = [fmono[0][i], fmono[1][i]];
                }
            }
        }

        // --- routing and filters in block form (unless done per sample above)
        if !filter_src {
            for s in 0..SOURCES {
                let (xl, xr): (&[f32; MAX_N], &[f32; MAX_N]) = match s {
                    SUB => (sub_buf, sub_buf),
                    s if s < OSC_COUNT => (&obuf[s][0], &obuf[s][1]),
                    _ => (noise_buf, noise_buf),
                };
                route_block(&lines[s], xl, xr, st, len, inv, f_in, main, direct, b1, b2, sum_tap);
            }
            // with both filters off nothing is routed to them (see the routing lines)
            if filter_on[0] || filter_on[1] {
                for i in st..len {
                    let t = (i + 1) as f32 * inv;
                    filter_sample(&mut self.filt, &fcoef, &fout_gain, cx.serial, t, i, f_in, main, fmono);
                }
            }
            self.xm_filter = if filters_run && len > st { [fmono[0][len - 1], fmono[1][len - 1]] } else { [0.0; 2] };
        }
        for f in 0..2 {
            if fcoef[f].is_some() {
                self.filt[f].finish();
            }
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
        let damp = (amp1 - amp0) / (len - st) as f32;
        let mix_in = |dst: &mut Stereo, src: &Stereo| {
            for ch in 0..2 {
                let mut a = amp0;
                for i in st..len {
                    a += damp;
                    dst[ch][i] += src[ch][i] * a;
                }
            }
        };
        mix_in(&mut buses.main, main);
        if uses_direct {
            mix_in(&mut buses.direct, direct);
        }
        if uses_b1 {
            mix_in(&mut buses.bus1, b1);
        }
        if uses_b2 {
            mix_in(&mut buses.bus2, b2);
        }
        if let Some(t) = taps {
            let mut a = amp0;
            for i in st..len {
                a += damp;
                if i % os != 0 {
                    continue;
                }
                let j = i / os;
                let d = if uses_direct { direct[0][i] + direct[1][i] } else { 0.0 };
                t.out[j] = 0.5 * (main[0][i] + main[1][i] + d) * a;
                t.sum[j] = sum_tap[i];
                if filters_run {
                    t.filter[j] = fmono[0][i];
                    t.filter2[j] = fmono[1][i];
                }
                if sub_on {
                    t.sub[j] = sub_buf[i];
                }
                if noise_on {
                    t.noise[j] = noise_buf[i];
                }
                for o in 0..OSC_COUNT {
                    if enabled[o] {
                        t.osc[o][j] = 0.5 * (obuf[o][0][i] + obuf[o][1][i]);
                    }
                }
            }
        }
        self.amp = envs[0];
        if self.killing() {
            self.kill_gain = kill_end;
        }
        if kill_end <= 0.0 || (self.env[0].stage == Stage::Idle && amp1 == 0.0) {
            self.active = false;
            for o in 0..OSC_COUNT {
                self.src[o].stop(self.slot as usize, o, shared);
            }
        }
    }
}

/// Voice Control: its two lanes' values `t` seconds into the note.
fn voice_control(base: &impl Fn(u16) -> f32, t: f32, bpm: f32) -> (f32, f32) {
    const BEATS: [f32; 9] = [4.0, 2.0, 1.0, 0.5, 0.25, 0.125, 2.0 / 3.0, 1.0 / 3.0, 1.0 / 6.0];
    const A: [u16; 8] = [p::VC_A1, p::VC_A2, p::VC_A3, p::VC_A4, p::VC_A5, p::VC_A6, p::VC_A7, p::VC_A8];
    const B: [u16; 8] = [p::VC_B1, p::VC_B2, p::VC_B3, p::VC_B4, p::VC_B5, p::VC_B6, p::VC_B7, p::VC_B8];
    let step = BEATS[(base(p::VC_RATE) as usize).min(BEATS.len() - 1)] * 60.0 / bpm.max(1.0);
    let steps = (base(p::VC_STEPS) as usize).clamp(1, 8);
    let x = t / step;
    let i = x.floor() as usize;
    let frac = x - x.floor();
    let looped = base(p::VC_LOOP) >= 0.5;
    let at = |k: usize| if looped { k % steps } else { k.min(steps - 1) };
    let (cur, prev) = (at(i), if i == 0 { at(0) } else { at(i - 1) });
    let smooth = base(p::VC_SMOOTH);
    let g = if smooth > 0.0 && i > 0 {
        let u = (frac / smooth).min(1.0);
        u * u * (3.0 - 2.0 * u)
    } else {
        1.0
    };
    let lane = |l: &[u16; 8]| base(l[prev]) + (base(l[cur]) - base(l[prev])) * g;
    (lane(&A), lane(&B))
}

/// How far oscillator `o`'s pitch moves for this note under its key range,
/// or None when the note (or its velocity) is outside and the mode is Range.
fn key_range(res: &impl Fn(u16) -> f32, o: usize, note: u8, velocity: f32) -> Option<f32> {
    let (lo, hi) = (res(p::OSC_KEY_LO[o]), res(p::OSC_KEY_HI[o]).max(res(p::OSC_KEY_LO[o])));
    let vel = velocity * 127.0;
    if vel < res(p::OSC_VEL_LO[o]) - 0.5 || vel > res(p::OSC_VEL_HI[o]) + 0.5 {
        return None;
    }
    let k = note as f32;
    match res(p::OSC_KEY_MODE[o]) as u8 {
        // Fold: octaves toward the range until the key is inside it
        1 => {
            let mut f = k;
            while f < lo && f + 12.0 <= 127.0 {
                f += 12.0;
            }
            while f > hi && f - 12.0 >= 0.0 {
                f -= 12.0;
            }
            Some(f - k)
        }
        // Warp: the whole keyboard squeezed into the range
        2 => Some(lo + (k / 127.0) * (hi - lo) - k),
        _ => (k >= lo && k <= hi).then_some(0.0),
    }
}

/// Resolved envelope settings for envelope `e`.
fn env_times(cx: &VoiceCtx, res: &impl Fn(u16) -> f32, e: usize) -> EnvTimes {
    let bpm = if res(p::ENV_BPM[e]) >= 0.5 { cx.bpm_scale } else { 1.0 };
    let ms = cx.sr * 0.001 * bpm / cx.env_rate;
    EnvTimes {
        attack: res(p::ENV_ATTACK[e]) * ms,
        hold: res(p::ENV_HOLD[e]) * ms,
        decay: res(p::ENV_DECAY[e]) * ms,
        sustain: res(p::ENV_SUSTAIN[e]),
        release: res(p::ENV_RELEASE[e]) * ms,
        attack_curve: res(p::ENV_ATTACK_CURVE[e]),
        decay_curve: res(p::ENV_DECAY_CURVE[e]),
        release_curve: res(p::ENV_RELEASE_CURVE[e]),
    }
}

/// Resolved settings for LFO `i`.
pub fn lfo_settings(cx: &VoiceCtx, res: impl Fn(u16) -> f32, i: usize) -> LfoSettings {
    lfo_settings_from(res, i, cx.bpm, cx.lfo_rate)
}

pub fn lfo_settings_from(res: impl Fn(u16) -> f32, i: usize, bpm: f32, rate_scale: f32) -> LfoSettings {
    let x10 = if res(p::LFO_X10[i]) >= 0.5 { 10.0 } else { 1.0 };
    let (hz, sync_beats) = if res(p::LFO_BPM[i]) >= 0.5 {
        let beats = lfo::SYNC_BEATS[(res(p::LFO_SYNC_RATE[i]) as usize).min(9)] * lfo::SYNC_MOD[(res(p::LFO_SYNC_MOD[i]) as usize).min(2)] / (x10 as f64 * rate_scale as f64);
        ((bpm as f64 / 60.0 / beats) as f32, Some(beats))
    } else {
        (res(p::LFO_RATE[i]) * x10 * rate_scale, None)
    };
    LfoSettings {
        kind: res(p::LFO_TYPE[i]) as u8,
        mode: res(p::LFO_MODE[i]) as u8,
        hz,
        sync_beats,
        rise: res(p::LFO_RISE[i]) * 0.001,
        delay: res(p::LFO_DELAY[i]) * 0.001,
        smooth: res(p::LFO_SMOOTH[i]),
        phase0: res(p::LFO_PHASE[i]) / 360.0,
        reverse: res(p::LFO_DIRECTION[i]) >= 0.5,
    }
}

/// Render order for cross-modulating oscillators: each after the
/// oscillators it listens to where possible (lowest index first on ties);
/// cycles fall back to index order, and those edges read the previous sample.
fn osc_order(xm: &[[Option<XMod>; 2]; OSC_COUNT], enabled: &[bool; OSC_COUNT]) -> [usize; OSC_COUNT] {
    let needs = |o: usize, s: usize| xm[o].iter().flatten().any(|x| x.src as usize == s && x.kind != XKind::PdSelf && s != o);
    let mut order = [0usize; OSC_COUNT];
    let mut placed = [false; OSC_COUNT];
    for slot in order.iter_mut() {
        let ready = (0..OSC_COUNT).find(|&o| !placed[o] && (0..OSC_COUNT).all(|s| placed[s] || !enabled[s] || !needs(o, s)));
        let pick = ready.unwrap_or_else(|| (0..OSC_COUNT).find(|&o| !placed[o]).unwrap());
        placed[pick] = true;
        *slot = pick;
    }
    order
}

type Stereo = [[f32; MAX_N]; 2];
/// Left/right gain at the start and end of a sub-block.
type GainRamp = ((f32, f32), (f32, f32));

#[inline(always)]
fn sample_sources(obuf: &[Stereo; OSC_COUNT], sub: &[f32; MAX_N], noise: &[f32; MAX_N], i: usize) -> ([(f32, f32); OSC_COUNT], [f32; 2]) {
    ([(obuf[0][0][i], obuf[0][1][i]), (obuf[1][0][i], obuf[1][1][i]), (obuf[2][0][i], obuf[2][1][i])], [sub[i], noise[i]])
}

/// Route one sample of every source to the filter inputs, buses and sends.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn route_sample(
    lines: &[Line; SOURCES],
    t: f32,
    osc_lr: &[(f32, f32); OSC_COUNT],
    sn: &[f32; 2],
    i: usize,
    f_in: &mut [Stereo; 2],
    main: &mut Stereo,
    direct: &mut Stereo,
    b1: &mut Stereo,
    b2: &mut Stereo,
    sum_tap: &mut [f32; MAX_N],
) {
    let lerp = |(a, b): (f32, f32)| a + (b - a) * t;
    for s in 0..SOURCES {
        let ln = &lines[s];
        if !ln.on || (ln.route == ROUTE_NONE && ln.s1 == (0.0, 0.0) && ln.s2 == (0.0, 0.0)) {
            continue;
        }
        let (xl, xr) = if s < OSC_COUNT { osc_lr[s] } else if s == SUB { (sn[0], sn[0]) } else { (sn[1], sn[1]) };
        let (l, r) = (xl * lerp(ln.gl), xr * lerp(ln.gr));
        match ln.route {
            ROUTE_FILTERS => {
                let bal = lerp(ln.bal);
                f_in[0][0][i] += l * (1.0 - bal);
                f_in[0][1][i] += r * (1.0 - bal);
                f_in[1][0][i] += l * bal;
                f_in[1][1][i] += r * bal;
            }
            ROUTE_MAIN => {
                main[0][i] += l;
                main[1][i] += r;
            }
            ROUTE_DIRECT => {
                direct[0][i] += l;
                direct[1][i] += r;
            }
            _ => {}
        }
        if ln.s1 != (0.0, 0.0) {
            let s1 = lerp(ln.s1);
            b1[0][i] += l * s1;
            b1[1][i] += r * s1;
        }
        if ln.s2 != (0.0, 0.0) {
            let s2 = lerp(ln.s2);
            b2[0][i] += l * s2;
            b2[1][i] += r * s2;
        }
        if ln.route != ROUTE_NONE {
            sum_tap[i] += 0.5 * (l + r);
        }
    }
}

/// Route one source's samples `st..len`: exactly `route_sample`'s arithmetic,
/// a source at a time (so each loop is simple enough to vectorize).
#[allow(clippy::too_many_arguments)]
fn route_block(
    ln: &Line,
    xl: &[f32; MAX_N],
    xr: &[f32; MAX_N],
    st: usize,
    len: usize,
    inv: f32,
    f_in: &mut [Stereo; 2],
    main: &mut Stereo,
    direct: &mut Stereo,
    b1: &mut Stereo,
    b2: &mut Stereo,
    sum_tap: &mut [f32; MAX_N],
) {
    if !ln.on || (ln.route == ROUTE_NONE && ln.s1 == (0.0, 0.0) && ln.s2 == (0.0, 0.0)) {
        return;
    }
    let lerp = |(a, b): (f32, f32), i: usize| a + (b - a) * ((i + 1) as f32 * inv);
    let l = |i: usize| xl[i] * lerp(ln.gl, i);
    let r = |i: usize| xr[i] * lerp(ln.gr, i);
    match ln.route {
        ROUTE_FILTERS => {
            for i in st..len {
                let (l, r, bal) = (l(i), r(i), lerp(ln.bal, i));
                f_in[0][0][i] += l * (1.0 - bal);
                f_in[0][1][i] += r * (1.0 - bal);
                f_in[1][0][i] += l * bal;
                f_in[1][1][i] += r * bal;
            }
        }
        ROUTE_MAIN | ROUTE_DIRECT => {
            let bus = if ln.route == ROUTE_MAIN { main } else { direct };
            for i in st..len {
                bus[0][i] += l(i);
                bus[1][i] += r(i);
            }
        }
        _ => {}
    }
    for (on, s, bus) in [(ln.s1 != (0.0, 0.0), ln.s1, b1), (ln.s2 != (0.0, 0.0), ln.s2, b2)] {
        if on {
            for i in st..len {
                let g = lerp(s, i);
                bus[0][i] += l(i) * g;
                bus[1][i] += r(i) * g;
            }
        }
    }
    if ln.route != ROUTE_NONE {
        for i in st..len {
            sum_tap[i] += 0.5 * (l(i) + r(i));
        }
    }
}

/// Run both filters for one sample (serial: 1 feeds 2) into the main bus.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn filter_sample(
    filt: &mut [FilterState; 2],
    coef: &[Option<Coeffs>; 2],
    gain: &[GainRamp; 2],
    serial: bool,
    t: f32,
    i: usize,
    f_in: &mut [Stereo; 2],
    main: &mut Stereo,
    mono: &mut [[f32; MAX_N]; 2],
) {
    let g = |f: usize| {
        let (a, b) = gain[f];
        (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
    };
    let x1 = [f_in[0][0][i], f_in[0][1][i]];
    let y1 = match &coef[0] {
        Some(c) => {
            let y = filt[0].tick(c, i, x1);
            let (gl, gr) = g(0);
            [y[0] * gl, y[1] * gr]
        }
        None => x1,
    };
    mono[0][i] = 0.5 * (y1[0] + y1[1]);
    let x2 = if serial { [f_in[1][0][i] + y1[0], f_in[1][1][i] + y1[1]] } else { [f_in[1][0][i], f_in[1][1][i]] };
    let y2 = match &coef[1] {
        Some(c) => {
            let y = filt[1].tick(c, i, x2);
            let (gl, gr) = g(1);
            [y[0] * gl, y[1] * gr]
        }
        None => x2,
    };
    mono[1][i] = 0.5 * (y2[0] + y2[1]);
    if serial {
        main[0][i] += y2[0];
        main[1][i] += y2[1];
    } else {
        main[0][i] += y1[0] + y2[0];
        main[1][i] += y1[1] + y2[1];
    }
}
