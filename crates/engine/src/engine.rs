//! The engine: command intake, voice allocation and the render loop.
//!
//! Rendering runs on a fixed 16-frame control grid (`SUB_BLOCK`). Note-ons
//! start at their exact frame; every other timed command lands on the first
//! grid boundary at or after its frame. Structural commands (tables, the
//! matrix, taps) take effect immediately. Nothing in `render` allocates.

use wt_dsp::math;
use wt_dsp::rng::Rng;

use crate::env::EnvTimes;
use crate::events::{Event, EventQueue};
use crate::modmatrix::{Matrix, Slot};
use crate::osc::unison::MAX_LANES;
use crate::params::ParamStore;
use crate::spec::params as p;
use crate::spec::protocol::{self as proto, Command, HEADER_BYTES, MAX_BLOCK, MAX_VOICES, OSC_COUNT, SUB_BLOCK, VOICE_SLOTS, tap, tel};
use crate::tables::{AssetBuf, Tables};
use crate::voice::{Start, Voice, VoiceCtx, VoiceTaps};

const N: usize = SUB_BLOCK;
const QUEUE_CAPACITY: usize = 4096;
/// Fade used when a voice is stolen or all sound is stopped.
const KILL_SECONDS: f32 = 0.003;
/// Smoothing time for parameters flagged `smooth`.
const SMOOTH_MS: f32 = 12.0;
/// Pitch-bend smoothing: fast enough to feel direct, slow enough not to zipper.
const BEND_MS: f32 = 6.0;
const MAX_HELD: usize = 128;

const _: () = assert!(tel::VOICE_NOTE_LEN == VOICE_SLOTS && tel::VOICE_LEVEL_LEN == VOICE_SLOTS);
const _: () = assert!(MAX_BLOCK.is_multiple_of(SUB_BLOCK));
const _: () = assert!(tel::OSC_WT_POS_LEN == OSC_COUNT && tel::FOCUS_ENV_LEN == 4);

#[derive(Debug, PartialEq, Eq)]
pub enum RenderError {
    /// Frames must be a positive multiple of SUB_BLOCK, at most MAX_BLOCK.
    BadFrames,
}

#[derive(Clone, Copy, Debug)]
struct Held {
    note: u8,
    channel: u8,
    velocity: f32,
    note_id: u32,
}

pub struct Engine {
    sr: f32,
    /// Absolute frame of the next sample to render.
    frame: u64,
    params: ParamStore,
    queue: EventQueue,
    voices: Vec<Voice>,
    age: u64,
    note_count: u32,
    focus: Option<usize>,
    tables: Tables,
    matrix: Matrix,
    rng: Rng,
    held: Vec<Held>,
    last_pitch: Option<f32>,
    last_phase: [[u32; MAX_LANES]; OSC_COUNT],
    bend_target: f32,
    bend: f32,
    bend_prev: f32,
    bend_coef: f32,
    modwheel: f32,
    aftertouch: f32,
    sustain: bool,
    scalar: bool,
    out: [Vec<f32>; 2],
    taps: Vec<Vec<f32>>,
    tap_mask: u32,
    tel: [f32; tel::LEN],
    master_prev: f32,
    unknown_cmds: u32,
    table_errors: u32,
    mix: [[f32; N]; 2],
    vtaps: VoiceTaps,
}

impl Engine {
    pub fn new(sample_rate: f32) -> Box<Engine> {
        let mut params = ParamStore::default();
        params.set_smoothing(sample_rate, N, SMOOTH_MS);
        let master_prev = math::db_to_gain(params.plain(p::MASTER_VOLUME));
        let mut e = Box::new(Engine {
            sr: sample_rate,
            frame: 0,
            params,
            queue: EventQueue::with_capacity(QUEUE_CAPACITY),
            voices: vec![Voice::default(); VOICE_SLOTS],
            age: 0,
            note_count: 0,
            focus: None,
            tables: Tables::default(),
            matrix: Matrix::default(),
            rng: Rng::new(0x5EED),
            held: Vec::with_capacity(MAX_HELD),
            last_pitch: None,
            last_phase: [[0; MAX_LANES]; OSC_COUNT],
            bend_target: 0.0,
            bend: 0.0,
            bend_prev: 0.0,
            bend_coef: 1.0 - (-(N as f32) / (BEND_MS * 0.001 * sample_rate)).exp(),
            modwheel: 0.0,
            aftertouch: 0.0,
            sustain: false,
            scalar: false,
            out: [vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]],
            taps: (0..tap::COUNT).map(|_| vec![0.0; MAX_BLOCK]).collect(),
            tap_mask: 0,
            tel: [0.0; tel::LEN],
            master_prev,
            unknown_cmds: 0,
            table_errors: 0,
            mix: [[0.0; N]; 2],
            vtaps: VoiceTaps::default(),
        });
        e.write_telemetry([0.0, 0.0]);
        e
    }

    pub fn sample_rate(&self) -> f32 {
        self.sr
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn out(&self, ch: usize) -> &[f32] {
        &self.out[ch.min(1)]
    }

    pub fn tap(&self, i: usize) -> &[f32] {
        &self.taps[i.min(tap::COUNT - 1)]
    }

    pub fn telemetry(&self) -> &[f32; tel::LEN] {
        &self.tel
    }

    pub fn params(&self) -> &ParamStore {
        &self.params
    }

    pub fn tables_mut(&mut self) -> &mut Tables {
        &mut self.tables
    }

    /// Use the scalar reference oscillator kernel (tests and debugging).
    pub fn set_scalar(&mut self, on: bool) {
        self.scalar = on;
    }

    /// Decode and queue a command batch. Returns how many commands decoded.
    pub fn apply(&mut self, buf: &[u8]) -> u32 {
        let mut pos = 0;
        let mut count = 0;
        while pos + HEADER_BYTES <= buf.len() {
            let op = u16::from_le_bytes([buf[pos], buf[pos + 1]]);
            let len = u32::from_le_bytes([buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]]) as usize;
            let mut f = [0u8; 8];
            f.copy_from_slice(&buf[pos + 8..pos + 16]);
            let frame = f64::from_le_bytes(f);
            let start = pos + HEADER_BYTES;
            let Some(end) = start.checked_add(len).filter(|&e| e <= buf.len()) else {
                self.unknown_cmds += 1;
                break;
            };
            match proto::decode(op, &buf[start..end]) {
                Some(cmd) => {
                    self.command(cmd, frame);
                    count += 1;
                }
                None => self.unknown_cmds += 1,
            }
            pos = end;
        }
        count
    }

    /// Accept one decoded command. Structural ones take effect now;
    /// everything else is queued for its frame.
    pub fn command(&mut self, cmd: Command, frame: f64) {
        match cmd {
            Command::SetTaps { mask } => self.tap_mask = mask,
            Command::Reset => self.reset(),
            Command::Debug { code, arg } => match code {
                proto::debug::TRAP => panic!("debug trap requested by the host"),
                proto::debug::SCALAR => self.scalar = arg >= 0.5,
                _ => {}
            },
            Command::LoadTable { osc, frames, ptr, bytes } => {
                if ptr == 0 {
                    self.table_errors += 1;
                    return;
                }
                // SAFETY: the host got ptr/bytes from wt_asset_alloc and hands them over here
                let asset = unsafe { AssetBuf::from_raw(ptr as usize as *mut u8, bytes as usize) };
                if self.tables.load(osc as usize, asset, frames as usize).is_err() {
                    self.table_errors += 1;
                }
            }
            Command::UpdateFrame { osc, index: fr, ptr, bytes } => {
                if ptr == 0 {
                    self.table_errors += 1;
                    return;
                }
                let asset = unsafe { AssetBuf::from_raw(ptr as usize as *mut u8, bytes as usize) };
                if self.tables.update_frame(osc as usize, fr as usize, asset.as_f32()).is_err() {
                    self.table_errors += 1;
                }
            }
            Command::ResetTable { osc } => self.tables.reset(osc as usize),
            Command::SetModSlot { slot, source, aux, flags, dest, amount } => {
                let ok = self.matrix.set(slot as usize, Slot { source, aux, flags, dest, amount });
                if !ok {
                    self.unknown_cmds += 1;
                }
            }
            Command::ClearMod => self.matrix.clear(),
            _ => {
                let frame = if frame > 0.0 { frame as u64 } else { 0 };
                self.queue.push(Event { frame, cmd });
            }
        }
    }

    /// Clear voices, pending events and smoothing; parameters and tables are kept.
    pub fn reset(&mut self) {
        self.queue.clear();
        for v in &mut self.voices {
            *v = Voice::default();
        }
        self.focus = None;
        self.held.clear();
        self.last_pitch = None;
        self.sustain = false;
        self.bend = self.bend_target;
        self.bend_prev = self.bend;
        self.params.snap();
        self.master_prev = math::db_to_gain(self.params.plain(p::MASTER_VOLUME));
    }

    /// Render `frames` samples starting at absolute frame `start`. A negative
    /// `start` continues from the engine's own clock.
    pub fn render(&mut self, frames: usize, start: f64) -> Result<(), RenderError> {
        if frames == 0 || !frames.is_multiple_of(N) || frames > MAX_BLOCK {
            return Err(RenderError::BadFrames);
        }
        if start >= 0.0 {
            self.frame = start as u64; // follow the host's clock
        }
        for sb in 0..frames / N {
            let t0 = self.frame + (sb * N) as u64;
            self.dispatch(t0);
            self.render_sub_block(sb * N);
        }
        self.frame += frames as u64;
        let mut peak = [0.0f32; 2];
        for (ch, pk) in peak.iter_mut().enumerate() {
            *pk = self.out[ch][..frames].iter().fold(0.0, |m, v| m.max(v.abs()));
        }
        self.write_telemetry(peak);
        Ok(())
    }

    /// Apply everything due by frame `t0`, then start any note-ons that fall
    /// inside this sub-block at their exact offset.
    fn dispatch(&mut self, t0: u64) {
        while let Some(ev) = self.queue.front() {
            if ev.frame > t0 {
                break;
            }
            let ev = self.queue.pop_front().unwrap();
            self.event(ev.cmd, 0);
        }
        let t1 = t0 + N as u64;
        let mut i = 0;
        while let Some(ev) = self.queue.get(i) {
            if ev.frame >= t1 {
                break;
            }
            if matches!(ev.cmd, Command::NoteOn { .. }) {
                let ev = self.queue.remove(i).unwrap();
                self.event(ev.cmd, (ev.frame - t0) as usize);
            } else {
                i += 1;
            }
        }
    }

    fn event(&mut self, cmd: Command, offset: usize) {
        match cmd {
            Command::SetParam { id, value } => {
                if !self.params.set(id as usize, value) {
                    self.unknown_cmds += 1;
                }
            }
            Command::NoteOn { note, channel, velocity, note_id } => self.note_on(note, channel, velocity, note_id, offset),
            Command::NoteOff { note, channel, velocity, note_id } => self.note_off(note, channel, velocity, note_id),
            Command::AllNotesOff => {
                self.held.clear();
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.release(0.0);
                }
            }
            Command::AllSoundOff => {
                let k = self.kill_samples();
                self.held.clear();
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.kill(k);
                }
            }
            Command::PitchBend { value, .. } => self.bend_target = value.clamp(-1.0, 1.0),
            Command::Controller { cc, value, .. } => match cc {
                1 => self.modwheel = value.clamp(0.0, 1.0),
                64 => self.set_sustain(value >= 0.5),
                _ => {}
            },
            Command::ChannelPressure { value, .. } => self.aftertouch = value.clamp(0.0, 1.0),
            Command::PolyPressure { .. } => {} // a per-voice source arrives with P2
            _ => {}
        }
    }

    fn kill_samples(&self) -> f32 {
        KILL_SECONDS * self.sr
    }

    fn glide_len(&self, from: f32, to: f32) -> f32 {
        let ms = self.params.plain(p::VOICE_GLIDE);
        let scaled = self.params.plain(p::VOICE_GLIDE_SCALED) >= 0.5;
        let len = ms * 0.001 * self.sr;
        if scaled { len * (to - from).abs() / 12.0 } else { len }
    }

    fn set_sustain(&mut self, down: bool) {
        self.sustain = down;
        if !down {
            for v in self.voices.iter_mut().filter(|v| v.active && v.sustained) {
                v.release(0.0);
            }
        }
    }

    fn note_on(&mut self, note: u8, channel: u8, velocity: f32, note_id: u32, offset: usize) {
        if velocity <= 0.0 {
            return self.note_off(note, channel, 0.0, note_id);
        }
        let pr = &self.params;
        let mono = pr.plain(p::VOICE_MONO) >= 0.5;
        let legato = pr.plain(p::VOICE_LEGATO) >= 0.5;
        let glide_on = pr.plain(p::VOICE_GLIDE) > 0.0;
        let always = pr.plain(p::VOICE_GLIDE_ALWAYS) >= 0.5 || (mono && legato);
        let overlapping = !self.held.is_empty();
        if self.held.len() < MAX_HELD {
            self.held.push(Held { note, channel, velocity, note_id });
        }
        let glide_from = if glide_on && (always || overlapping) { self.last_pitch } else { None };
        self.last_pitch = Some(note as f32);

        if mono {
            // reuse the newest voice that's still going
            let current = (0..VOICE_SLOTS).filter(|&i| self.voices[i].active && !self.voices[i].killing()).max_by_key(|&i| self.voices[i].age);
            if let Some(i) = current {
                let v = &self.voices[i];
                let from = v.pitch();
                let held_down = !v.released;
                let glide = if glide_from.is_some() { self.glide_len(from, note as f32) } else { 0.0 };
                let retrigger = !(legato && held_down);
                self.voices[i].retarget(note, note_id, velocity, glide, retrigger);
                self.focus = Some(i);
                // mono keeps one voice: fade any others
                let k = self.kill_samples();
                for (j, v) in self.voices.iter_mut().enumerate() {
                    if j != i && v.active {
                        v.kill(k);
                    }
                }
                return;
            }
        }

        let poly = if mono { 1 } else { (pr.plain(p::VOICE_POLYPHONY) as usize).clamp(1, MAX_VOICES) };
        let sounding = self.voices.iter().filter(|v| v.active && !v.killing()).count();
        if sounding >= poly {
            if let Some(i) = self.steal_victim() {
                let k = self.kill_samples();
                self.voices[i].kill(k);
            }
        }
        let slot = match self.voices.iter().position(|v| !v.active) {
            Some(i) => i,
            // every slot is still fading: reuse the oldest outright
            None => (0..VOICE_SLOTS).min_by_key(|&i| self.voices[i].age).unwrap_or(0),
        };
        let glide_len = glide_from.map_or(0.0, |f| self.glide_len(f, note as f32));
        let mut phase = [0.0f32; OSC_COUNT];
        let mut rand_phase = [0.0f32; OSC_COUNT];
        let mut memory = [false; OSC_COUNT];
        for o in 0..OSC_COUNT {
            phase[o] = self.params.plain(p::OSC_PHASE[o]) / 360.0;
            rand_phase[o] = self.params.plain(p::OSC_RAND_PHASE[o]);
            memory[o] = self.params.plain(p::OSC_PHASE_MEM[o]) >= 0.5 && self.age > 0;
        }
        self.age += 1;
        self.note_count = self.note_count.wrapping_add(1);
        let last = self.last_phase;
        let start = Start {
            note,
            channel,
            velocity,
            note_id,
            offset,
            age: self.age,
            index: (self.note_count % 32) as u8,
            glide_from,
            glide_len,
            glide_curve: self.params.plain(p::VOICE_GLIDE_CURVE),
            phase,
            rand_phase,
            memory: [memory[0].then_some(&last[0]), memory[1].then_some(&last[1]), memory[2].then_some(&last[2])],
            rng: &mut self.rng,
        };
        self.voices[slot].start(start);
        self.focus = Some(slot);
    }

    /// Which voice to steal: released voices go first, then the steal priority decides.
    fn steal_victim(&self) -> Option<usize> {
        let policy = self.params.plain(p::VOICE_STEAL) as u8;
        let candidates = || (0..VOICE_SLOTS).filter(|&i| self.voices[i].active && !self.voices[i].killing());
        let any_released = candidates().any(|i| self.voices[i].released);
        let pool = |i: &usize| !any_released || self.voices[*i].released;
        let v = &self.voices;
        match policy {
            1 => candidates().filter(pool).max_by_key(|&i| v[i].age),
            2 => candidates().filter(pool).min_by_key(|&i| (v[i].note, v[i].age)),
            3 => candidates().filter(pool).max_by_key(|&i| (v[i].note, u64::MAX - v[i].age)),
            4 => candidates().filter(pool).min_by(|&a, &b| v[a].velocity.total_cmp(&v[b].velocity).then(v[a].age.cmp(&v[b].age))),
            _ => candidates().filter(pool).min_by_key(|&i| v[i].age),
        }
    }

    fn note_off(&mut self, note: u8, channel: u8, velocity: f32, note_id: u32) {
        if let Some(k) = self.held.iter().position(|h| if note_id != 0 { h.note_id == note_id } else { h.note == note && h.channel == channel }) {
            self.held.remove(k);
        }
        let mono = self.params.plain(p::VOICE_MONO) >= 0.5;
        let matches = |v: &Voice| v.active && !v.released && (if note_id != 0 { v.note_id == note_id } else { v.note == note && v.channel == channel });

        if mono {
            let Some(i) = (0..VOICE_SLOTS).find(|&i| matches(&self.voices[i])) else { return };
            if let Some(&back) = self.held.last() {
                // return to the newest key still held
                let legato = self.params.plain(p::VOICE_LEGATO) >= 0.5;
                let glide_on = self.params.plain(p::VOICE_GLIDE) > 0.0;
                let from = self.voices[i].pitch();
                let glide = if glide_on { self.glide_len(from, back.note as f32) } else { 0.0 };
                self.voices[i].retarget(back.note, back.note_id, back.velocity, glide, !legato);
                self.last_pitch = Some(back.note as f32);
                return;
            }
            if self.sustain {
                self.voices[i].sustained = true;
            } else {
                self.voices[i].release(velocity);
            }
            return;
        }
        let sustain = self.sustain;
        for v in self.voices.iter_mut().filter(|v| matches(v)) {
            if sustain {
                v.sustained = true;
            } else {
                v.release(velocity);
            }
        }
    }

    fn render_sub_block(&mut self, off: usize) {
        self.params.step();
        self.bend_prev = self.bend;
        self.bend += (self.bend_target - self.bend) * self.bend_coef;
        let bend_semis = if self.bend >= 0.0 { self.bend * self.params.plain(p::VOICE_BEND_UP) } else { self.bend * self.params.plain(p::VOICE_BEND_DOWN) };
        let active = self.voices.iter().filter(|v| v.active && !v.killing()).count() as f32;
        let cx = VoiceCtx {
            sr: self.sr,
            params: &self.params,
            matrix: &self.matrix,
            tables: &self.tables,
            env: [
                EnvTimes::from_params(&self.params, 0, self.sr),
                EnvTimes::from_params(&self.params, 1, self.sr),
                EnvTimes::from_params(&self.params, 2, self.sr),
                EnvTimes::from_params(&self.params, 3, self.sr),
            ],
            bend: bend_semis,
            bend_raw: self.bend,
            modwheel: self.modwheel,
            aftertouch: self.aftertouch,
            active_voices: active,
            scalar: self.scalar,
        };

        self.mix = [[0.0; N]; 2];
        let focus = self.focus;
        let mut focus_rendered = false;
        for (i, v) in self.voices.iter_mut().enumerate() {
            if !v.active {
                continue;
            }
            if focus == Some(i) {
                self.vtaps = VoiceTaps::default();
                v.render(&cx, &mut self.mix, Some(&mut self.vtaps));
                focus_rendered = true;
            } else {
                v.render(&cx, &mut self.mix, None);
            }
        }
        if !focus_rendered {
            self.vtaps = VoiceTaps::default();
        }

        let g1 = math::db_to_gain(self.params.plain(p::MASTER_VOLUME));
        let g0 = self.master_prev;
        self.master_prev = g1;
        let dg = (g1 - g0) / N as f32;
        for i in 0..N {
            let g = g0 + dg * (i + 1) as f32;
            self.out[0][off + i] = self.mix[0][i] * g;
            self.out[1][off + i] = self.mix[1][i] * g;
        }

        let m = self.tap_mask;
        let copy = |dst: &mut Vec<f32>, src: &[f32]| dst[off..off + N].copy_from_slice(src);
        if m & (1 << tap::MASTER_L) != 0 {
            let src: [f32; N] = self.out[0][off..off + N].try_into().unwrap();
            copy(&mut self.taps[tap::MASTER_L], &src);
        }
        if m & (1 << tap::MASTER_R) != 0 {
            let src: [f32; N] = self.out[1][off..off + N].try_into().unwrap();
            copy(&mut self.taps[tap::MASTER_R], &src);
        }
        let vt = &self.vtaps;
        let voice_taps: [(usize, &[f32; N]); 6] = [
            (tap::FOCUS_OSC, &vt.sum),
            (tap::FOCUS_OUT, &vt.out),
            (tap::FOCUS_OSC_A, &vt.osc[0]),
            (tap::FOCUS_OSC_B, &vt.osc[1]),
            (tap::FOCUS_OSC_C, &vt.osc[2]),
            (tap::FOCUS_FILTER, &vt.filter),
        ];
        for (t, src) in voice_taps {
            if m & (1 << t) != 0 {
                self.taps[t][off..off + N].copy_from_slice(src);
            }
        }

        self.update_focus();
        if let Some(i) = self.focus {
            for o in 0..OSC_COUNT {
                self.last_phase[o] = self.voices[i].osc[o].phase;
            }
        }
    }

    /// Focus follows the newest sounding voice; a voice being stolen only
    /// keeps it when nothing else is playing.
    fn update_focus(&mut self) {
        if let Some(i) = self.focus {
            let v = &self.voices[i];
            if v.active && !v.killing() {
                return;
            }
        }
        let newest = |killing: bool| {
            self.voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.active && v.killing() == killing)
                .max_by_key(|(_, v)| v.age)
                .map(|(i, _)| i)
        };
        self.focus = newest(false).or_else(|| newest(true));
    }

    fn write_telemetry(&mut self, peak: [f32; 2]) {
        let t = &mut self.tel;
        t[tel::VOICES_ACTIVE] = self.voices.iter().filter(|v| v.active && !v.killing()).count() as f32;
        t[tel::PEAK_L] = peak[0];
        t[tel::PEAK_R] = peak[1];
        t[tel::FOCUS_VOICE] = self.focus.map_or(-1.0, |i| i as f32);
        t[tel::QUEUE_DROPS] = self.queue.drops as f32;
        t[tel::UNKNOWN_CMDS] = self.unknown_cmds as f32;
        for (i, v) in self.voices.iter().enumerate() {
            t[tel::VOICE_NOTE + i] = if v.active { v.note as f32 } else { -1.0 };
            t[tel::VOICE_LEVEL + i] = if v.active { v.env[0].level } else { 0.0 };
        }
        for o in 0..OSC_COUNT {
            t[tel::OSC_FRAMES + o] = self.tables.frames(o) as f32;
        }
        t[tel::TABLE_ERRORS] = self.table_errors as f32;
        t[tel::MOD_SLOTS] = self.matrix.live() as f32;
        if let Some(i) = self.focus {
            let v = &self.voices[i];
            for o in 0..OSC_COUNT {
                t[tel::OSC_WT_POS + o] = v.tel_wt_pos[o];
            }
            t[tel::FOCUS_PITCH] = v.pitch();
            for e in 0..4 {
                t[tel::FOCUS_ENV + e] = v.env[e].level;
            }
            t[tel::FOCUS_CUTOFF] = v.tel_cutoff;
        } else {
            for o in 0..OSC_COUNT {
                t[tel::OSC_WT_POS + o] = self.params.plain(p::OSC_WT_POS[o]);
            }
            for e in 0..4 {
                t[tel::FOCUS_ENV + e] = 0.0;
            }
        }
    }
}
