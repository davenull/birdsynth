//! The engine: command intake, voice allocation and the render loop.
//!
//! Rendering runs on a fixed 16-frame control grid (`SUB_BLOCK`). Note-ons
//! start at their exact frame; every other timed command lands on the first
//! grid boundary at or after its frame. Structural commands (tables,
//! samples, the matrix, LFO shapes, taps) take effect immediately. Nothing
//! in `render` allocates.
//!
//! Voices render at 1, 2 or 4 times the output rate (the Quality setting,
//! used only when a patch has warps that need it); the buses are decimated
//! back with halfband filters before the master stage.
//!
//! Modulation policy: per-voice destinations use each voice's own sources.
//! Global destinations (master, global, mix) follow the newest voice that's
//! still sounding and hold its values after it ends. Non-poly (and
//! free-running) LFOs are shared by every voice, as are macros and
//! controllers.

use wt_dsp::math;
use wt_dsp::oversample::{self, Halfband};
use wt_dsp::rng::Rng;
use wt_dsp::saw;
use wt_dsp::warp::{REMAP_POINTS, Remap};

use crate::events::{Event, EventQueue};
use crate::filter::MAX_N;
use crate::fx::{self, Fx};
use crate::lfo::{self, LfoSettings, LfoState, Point, Shape};
use crate::modmatrix::{Matrix, NONE, Slot};
use crate::osc::{self, unison::MAX_LANES};
use crate::params::ParamStore;
use crate::samples::{self, Samples};
use crate::spec::params as p;
use crate::spec::protocol::{self as proto, Command, HEADER_BYTES, MAX_BLOCK, MAX_MOD_SLOTS, MAX_VOICES, OSC_COUNT, SUB_BLOCK, VOICE_SLOTS, tap, tel};
use crate::tables::{AssetBuf, Tables};
use crate::voice::{self, Buses, LFOS, MACROS, Scratch, Start, Voice, VoiceCtx, VoiceTaps};

const N: usize = SUB_BLOCK;
const QUEUE_CAPACITY: usize = 4096;
/// Fade used when a voice is stolen or all sound is stopped.
const KILL_SECONDS: f32 = 0.003;
/// Smoothing time for parameters flagged `smooth`.
const SMOOTH_MS: f32 = 12.0;
/// Pitch-bend smoothing: fast enough to feel direct, slow enough not to zipper.
const BEND_MS: f32 = 6.0;
const MAX_HELD: usize = 128;
/// Bus channels that are decimated: main, direct, bus 1, bus 2 (L and R each).
const DEC_CH: usize = 8;

const _: () = assert!(tel::VOICE_NOTE_LEN == VOICE_SLOTS && tel::VOICE_LEVEL_LEN == VOICE_SLOTS);
const _: () = assert!(MAX_BLOCK.is_multiple_of(SUB_BLOCK));
const _: () = assert!(tel::OSC_WT_POS_LEN == OSC_COUNT && tel::FOCUS_ENV_LEN == 4);
const _: () = assert!(tel::LFO_VALUE_LEN == LFOS && tel::MACRO_VALUE_LEN == MACROS && tel::MOD_VALUE_LEN == MAX_MOD_SLOTS);

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

/// Is a parameter a global destination (one value for the whole synth)?
pub fn is_global(id: u16) -> bool {
    let k = p::INFO[id as usize].key;
    ["master.", "global.", "mix.", "fx.", "rack."].iter().any(|pre| k.starts_with(pre))
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
    remaps: [Remap; OSC_COUNT],
    /// Pitch of each MIDI note (fractional note number).
    tuning: [f32; 128],
    bandlimit: bool,
    samples: Samples,
    triangle: Vec<f32>,
    matrix: Matrix,
    rng: Rng,
    held: Vec<Held>,
    last_pitch: Option<f32>,
    last_phase: [[u32; MAX_LANES]; OSC_COUNT],
    lfo_shapes: [Shape; LFOS],
    lfo_paths: [Shape; LFOS],
    /// Shared LFO states (non-poly and free-running LFOs), and their outputs.
    lfo_global: [LfoState; LFOS],
    lfo_out: [(f32, f32); LFOS],
    /// Held modulation offsets for global destinations (by destination index).
    global_off: [f32; MAX_MOD_SLOTS],
    bend_target: f32,
    bend: f32,
    bend_coef: f32,
    modwheel: f32,
    aftertouch: f32,
    sustain: bool,
    scalar: bool,
    os: usize,
    hb: [f32; oversample::TAPS],
    /// Two decimation stages per bus channel (4x uses both).
    dec: [[Halfband; 2]; DEC_CH],
    buses: Box<Buses>,
    scratch: Box<Scratch>,
    fx: Box<Fx>,
    out: [Vec<f32>; 2],
    taps: Vec<Vec<f32>>,
    tap_mask: u32,
    tel: [f32; tel::LEN],
    master_prev: f32,
    unknown_cmds: u32,
    table_errors: u32,
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
            remaps: [Remap::default(); OSC_COUNT],
            tuning: std::array::from_fn(|n| n as f32),
            bandlimit: true,
            samples: Samples::default(),
            triangle: saw::triangle_frame(),
            matrix: Matrix::default(),
            rng: Rng::new(0x5EED),
            held: Vec::with_capacity(MAX_HELD),
            last_pitch: None,
            last_phase: [[0; MAX_LANES]; OSC_COUNT],
            lfo_shapes: [Shape::default_curve(); LFOS],
            lfo_paths: [Shape::default_path(); LFOS],
            lfo_global: [LfoState::default(); LFOS],
            lfo_out: [(0.0, 0.0); LFOS],
            global_off: [0.0; MAX_MOD_SLOTS],
            bend_target: 0.0,
            bend: 0.0,
            bend_coef: 1.0 - (-(N as f32) / (BEND_MS * 0.001 * sample_rate)).exp(),
            modwheel: 0.0,
            aftertouch: 0.0,
            sustain: false,
            scalar: false,
            os: 1,
            hb: oversample::design(),
            dec: [[Halfband::default(); 2]; DEC_CH],
            buses: Box::default(),
            scratch: Box::default(),
            fx: Fx::new(sample_rate),
            out: [vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]],
            taps: (0..tap::COUNT).map(|_| vec![0.0; MAX_BLOCK]).collect(),
            tap_mask: 0,
            tel: [0.0; tel::LEN],
            master_prev,
            unknown_cmds: 0,
            table_errors: 0,
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

    pub fn samples_mut(&mut self) -> &mut Samples {
        &mut self.samples
    }

    pub fn fx_mut(&mut self) -> &mut Fx {
        &mut self.fx
    }

    #[cfg(test)]
    pub(crate) fn voices(&self) -> &[Voice] {
        &self.voices
    }

    /// Use the scalar reference oscillator kernel (tests and debugging).
    pub fn set_scalar(&mut self, on: bool) {
        self.scalar = on;
    }

    /// Set an LFO's drawn shape, or its XY path.
    pub fn set_lfo_shape(&mut self, lfo: usize, path: bool, pts: &[Point]) {
        if lfo >= LFOS || pts.is_empty() {
            return;
        }
        if path {
            self.lfo_paths[lfo].set(pts, true);
        } else {
            self.lfo_shapes[lfo].set(pts, false);
        }
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
            let payload = &buf[start..end];
            match proto::decode(op, payload) {
                Some(cmd) => {
                    match proto::tail_offset(op) {
                        Some(at) if at <= payload.len() => self.command_tail(cmd, &payload[at..]),
                        Some(_) => self.unknown_cmds += 1,
                        None => self.command(cmd, frame),
                    }
                    count += 1;
                }
                None => self.unknown_cmds += 1,
            }
            pos = end;
        }
        count
    }

    /// Commands with a variable-length tail (applied immediately).
    fn command_tail(&mut self, cmd: Command, tail: &[u8]) {
        let rd = |i: usize| f32::from_le_bytes([tail[i], tail[i + 1], tail[i + 2], tail[i + 3]]);
        match cmd {
            Command::SetLfoShape { lfo, kind, count } => {
                let n = (count as usize).min(lfo::MAX_POINTS).min(tail.len() / 12);
                let mut pts = [Point::default(); lfo::MAX_POINTS];
                for (i, pt) in pts.iter_mut().enumerate().take(n) {
                    *pt = Point { x: rd(i * 12), y: rd(i * 12 + 4), c: rd(i * 12 + 8) };
                }
                self.set_lfo_shape(lfo as usize, kind == 1, &pts[..n]);
            }
            Command::SetChain { chain, count } => {
                let n = (count as usize).min(fx::MAX_CHAIN).min(tail.len() / 2);
                let mut refs = [0u16; fx::MAX_CHAIN];
                for (i, r) in refs.iter_mut().enumerate().take(n) {
                    *r = u16::from_le_bytes([tail[2 * i], tail[2 * i + 1]]);
                }
                if !self.fx.set_chain(chain as usize, &refs[..n]) {
                    self.unknown_cmds += 1;
                }
            }
            Command::SetTuning { count } => {
                let n = (count as usize).min(128).min(tail.len() / 4);
                for i in 0..128 {
                    self.tuning[i] = if i < n {
                        let v = rd(i * 4);
                        if v.is_finite() { v.clamp(-24.0, 160.0) } else { i as f32 }
                    } else {
                        i as f32
                    };
                }
            }
            Command::SetOscCurve { osc, count } => {
                let n = (count as usize).min(REMAP_POINTS).min(tail.len() / 4);
                let mut v = [0.0f32; REMAP_POINTS];
                for (i, x) in v.iter_mut().enumerate().take(n) {
                    *x = rd(i * 4);
                }
                if let Some(r) = self.remaps.get_mut(osc as usize) {
                    *r = Remap::from_values(&v[..n]);
                }
            }
            _ => {}
        }
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
                proto::debug::NO_BANDLIMIT => self.bandlimit = arg < 0.5,
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
            Command::UpdateFrame { osc, index, ptr, bytes } => {
                if ptr == 0 {
                    self.table_errors += 1;
                    return;
                }
                let asset = unsafe { AssetBuf::from_raw(ptr as usize as *mut u8, bytes as usize) };
                if self.tables.update_frame(osc as usize, index as usize, asset.as_f32()).is_err() {
                    self.table_errors += 1;
                }
            }
            Command::ResetTable { osc } => self.tables.reset(osc as usize),
            Command::LoadSample { slot, frames, rate, ptr, bytes } => {
                if ptr == 0 {
                    self.table_errors += 1;
                    return;
                }
                let asset = unsafe { AssetBuf::from_raw(ptr as usize as *mut u8, bytes as usize) };
                if self.samples.load(slot as usize, asset, frames as usize, rate).is_err() {
                    self.table_errors += 1;
                }
            }
            Command::SetModSlot { slot, source, aux, flags, dest, amount, curve, output } => {
                if !self.matrix.set(slot as usize, Slot { source, aux, flags, dest, amount, curve, output }) {
                    self.unknown_cmds += 1;
                }
            }
            Command::ClearMod => self.matrix.clear(),
            Command::LoadIr { inst, taps, ptr, bytes } => {
                let asset = if ptr == 0 { None } else { Some(unsafe { AssetBuf::from_raw(ptr as usize as *mut u8, bytes as usize) }) };
                if !self.fx.load_ir(inst as usize, asset, taps as usize) {
                    self.table_errors += 1;
                }
            }
            Command::SetLfoShape { .. } | Command::SetOscCurve { .. } | Command::SetChain { .. } | Command::SetTuning { .. } => {} // need their tails: see `apply`
            _ => {
                let frame = if frame > 0.0 { frame as u64 } else { 0 };
                self.queue.push(Event { frame, cmd });
            }
        }
    }

    /// Clear voices, pending events and smoothing; parameters, tables,
    /// samples, the matrix and LFO shapes are kept.
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
        self.params.snap();
        self.master_prev = math::db_to_gain(self.params.plain(p::MASTER_VOLUME));
        for d in self.dec.iter_mut().flatten() {
            d.reset();
        }
        self.lfo_global = [LfoState::default(); LFOS];
        self.global_off = [0.0; MAX_MOD_SLOTS];
        self.fx.reset();
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
            self.render_sub_block(sb * N, t0);
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
            Command::PolyPressure { note, channel, value, note_id } => {
                let hit = |v: &Voice| v.active && if note_id != 0 { v.note_id == note_id } else { v.note == note && v.channel == channel };
                for v in self.voices.iter_mut().filter(|v| hit(v)) {
                    v.poly_at = value.clamp(0.0, 1.0);
                }
            }
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

    /// Settings of the LFOs that restart with a note (Retrig or Env mode),
    /// for the poly LFOs (`poly`) or the shared ones.
    fn note_lfos(&self, poly: bool) -> [Option<LfoSettings>; LFOS] {
        let bpm = self.params.plain(p::GLOBAL_BPM);
        let rate = self.params.plain(p::GLOBAL_LFO_RATE);
        let mut out = [None; LFOS];
        for (i, o) in out.iter_mut().enumerate() {
            let set = voice::lfo_settings_from(|id| self.params.plain(id), i, bpm, rate);
            let is_poly = self.params.plain(p::LFO_POLY[i]) >= 0.5;
            if set.mode != lfo::MODE_FREE && is_poly == poly {
                *o = Some(set);
            }
        }
        out
    }

    fn env_from_zero(&self) -> [bool; 4] {
        [0, 1, 2, 3].map(|e| self.params.plain(p::ENV_RETRIG[e]) >= 0.5)
    }

    /// Which envelopes restart on a mono note change: all of them unless
    /// playing legato, each flipped by its Legato Invert switch.
    fn mono_retrigger(&self, base: bool) -> [bool; 4] {
        [0, 1, 2, 3].map(|e| base != (self.params.plain(p::ENV_LEGATO_INVERT[e]) >= 0.5))
    }

    /// Velocity through the Velocity Curve.
    fn curve_velocity(&self, v: f32) -> f32 {
        let c = self.params.plain(p::VOICE_VEL_CURVE);
        if c == 0.0 { v } else { v.clamp(0.0, 1.0).powf(4f32.powf(-c)) }
    }

    fn note_on(&mut self, note: u8, channel: u8, velocity: f32, note_id: u32, offset: usize) {
        if velocity <= 0.0 {
            return self.note_off(note, channel, 0.0, note_id);
        }
        let velocity = self.curve_velocity(velocity);
        let pitch = self.tuning[(note as usize).min(127)];
        let mono = self.params.plain(p::VOICE_MONO) >= 0.5;
        let legato = self.params.plain(p::VOICE_LEGATO) >= 0.5;
        let glide_on = self.params.plain(p::VOICE_GLIDE) > 0.0;
        let always = self.params.plain(p::VOICE_GLIDE_ALWAYS) >= 0.5 || (mono && legato);
        let overlapping = !self.held.is_empty();
        if self.held.len() < MAX_HELD {
            self.held.push(Held { note, channel, velocity, note_id });
        }
        let glide_from = if glide_on && (always || overlapping) { self.last_pitch } else { None };
        self.last_pitch = Some(pitch);

        // shared LFOs in Retrig/Env mode restart with every note
        let shared = self.note_lfos(false);
        for (i, set) in shared.iter().enumerate() {
            if let Some(set) = set {
                let seed = self.rng.next_u32();
                self.lfo_global[i].trigger(set, seed);
            }
        }
        let from_zero = self.env_from_zero();

        if mono {
            // reuse the newest voice that's still going
            let current = (0..VOICE_SLOTS).filter(|&i| self.voices[i].active && !self.voices[i].killing()).max_by_key(|&i| self.voices[i].age);
            if let Some(i) = current {
                let from = self.voices[i].pitch();
                let held_down = !self.voices[i].released;
                let glide = if glide_from.is_some() { self.glide_len(from, pitch) } else { 0.0 };
                let base = !(legato && held_down);
                let retrigger = self.mono_retrigger(base);
                self.voices[i].retarget(note, pitch, note_id, velocity, glide, retrigger, from_zero);
                if base {
                    let lfos = self.note_lfos(true);
                    self.voices[i].retrigger_lfos(&lfos, &mut self.rng);
                }
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

        let poly = if mono { 1 } else { (self.params.plain(p::VOICE_POLYPHONY) as usize).clamp(1, MAX_VOICES) };
        let sounding = self.voices.iter().filter(|v| v.active && !v.killing()).count();
        if sounding >= poly
            && let Some(i) = self.steal_victim()
        {
            let k = self.kill_samples();
            self.voices[i].kill(k);
        }
        let slot = match self.voices.iter().position(|v| !v.active) {
            Some(i) => i,
            // every slot is still fading: reuse the oldest outright
            None => (0..VOICE_SLOTS).min_by_key(|&i| self.voices[i].age).unwrap_or(0),
        };
        let glide_len = glide_from.map_or(0.0, |f| self.glide_len(f, pitch));
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
        let lfo = self.note_lfos(true);
        let noise_frames = self.samples.get(samples::NOISE).map_or(1, |s| s.frames);
        let start = Start {
            note,
            pitch,
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
            noise_start: (self.params.plain(p::NOISE_PHASE), self.params.plain(p::NOISE_RAND)),
            noise_frames,
            lfo,
            env_from_zero: from_zero,
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
                let to = self.tuning[(back.note as usize).min(127)];
                let glide = if glide_on { self.glide_len(from, to) } else { 0.0 };
                let retrigger = self.mono_retrigger(!legato);
                let from_zero = self.env_from_zero();
                self.voices[i].retarget(back.note, to, back.note_id, back.velocity, glide, retrigger, from_zero);
                self.last_pitch = Some(to);
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

    /// Oversampling for this sub-block: the Quality factor, when an enabled
    /// oscillator uses a warp that benefits.
    fn pick_os(&self) -> usize {
        let pr = &self.params;
        let factor = [1, 2, 4][(pr.plain(p::GLOBAL_QUALITY) as usize).min(2)];
        if factor == 1 {
            return 1;
        }
        let needs = (0..OSC_COUNT).any(|o| {
            pr.plain(p::OSC_ENABLE[o]) >= 0.5
                && (osc::wants_oversampling(pr.plain(p::OSC_WARP1_MODE[o]) as u8, pr.plain(p::OSC_WARP1_AMOUNT[o]))
                    || osc::wants_oversampling(pr.plain(p::OSC_WARP2_MODE[o]) as u8, pr.plain(p::OSC_WARP2_AMOUNT[o])))
        });
        if needs { factor } else { 1 }
    }

    fn render_sub_block(&mut self, off: usize, t0: u64) {
        self.params.step();
        self.bend += (self.bend_target - self.bend) * self.bend_coef;
        let bend_semis = if self.bend >= 0.0 { self.bend * self.params.plain(p::VOICE_BEND_UP) } else { self.bend * self.params.plain(p::VOICE_BEND_DOWN) };
        let active = self.voices.iter().filter(|v| v.active && !v.killing()).count() as f32;
        let bpm = self.params.plain(p::GLOBAL_BPM);
        let lfo_rate = self.params.plain(p::GLOBAL_LFO_RATE);
        let dt = N as f32 / self.sr;
        let beat = t0 as f64 / self.sr as f64 * bpm as f64 / 60.0;

        // shared LFOs
        for i in 0..LFOS {
            let set = voice::lfo_settings_from(|id| self.params.plain(id), i, bpm, lfo_rate);
            self.lfo_out[i] = self.lfo_global[i].advance(&set, &self.lfo_shapes[i], &self.lfo_paths[i], dt, beat);
        }

        let os = self.pick_os();
        if os != self.os {
            self.os = os;
            for d in self.dec.iter_mut().flatten() {
                d.reset();
            }
        }
        self.buses.clear(N * os);
        let cx = VoiceCtx {
            sr: self.sr,
            os,
            params: &self.params,
            matrix: &self.matrix,
            tables: &self.tables,
            remaps: &self.remaps,
            samples: &self.samples,
            saw: self.tables.saw(),
            triangle: &self.triangle,
            lfo_shapes: &self.lfo_shapes,
            lfo_paths: &self.lfo_paths,
            lfo_global: self.lfo_out,
            dt,
            beat,
            bend: bend_semis,
            bend_raw: self.bend,
            modwheel: self.modwheel,
            aftertouch: self.aftertouch,
            active_voices: active,
            scalar: self.scalar,
            bpm_scale: 120.0 / bpm.max(1.0),
            env_rate: self.params.plain(p::GLOBAL_ENV_RATE),
            lfo_rate,
            bpm,
            serial: self.params.plain(p::MIX_FILTER_ROUTING) < 0.5,
            tune: 12.0 * (self.params.plain(p::GLOBAL_TUNE) / 440.0).log2(),
            bandlimit: self.bandlimit,
        };

        let focus = self.focus;
        let mut focus_rendered = false;
        for (i, v) in self.voices.iter_mut().enumerate() {
            if !v.active {
                continue;
            }
            if focus == Some(i) {
                self.vtaps = VoiceTaps::default();
                v.render(&cx, &mut self.buses, &mut self.scratch, Some(&mut self.vtaps));
                focus_rendered = true;
            } else {
                v.render(&cx, &mut self.buses, &mut self.scratch, None);
            }
        }
        if !focus_rendered {
            self.vtaps = VoiceTaps::default();
        }

        // back to the output rate
        let mut bus = [[[0.0f32; N]; 2]; 4]; // main, direct, bus 1, bus 2
        let b = &*self.buses;
        let chans: [&[f32; MAX_N]; DEC_CH] = [&b.main[0], &b.main[1], &b.direct[0], &b.direct[1], &b.bus1[0], &b.bus1[1], &b.bus2[0], &b.bus2[1]];
        for (c, src) in chans.iter().enumerate() {
            let out = &mut bus[c / 2][c % 2];
            match os {
                1 => out.copy_from_slice(&src[..N]),
                2 => self.dec[c][0].process(&self.hb, &src[..2 * N], out),
                _ => {
                    let mut half = [0.0f32; 2 * N];
                    self.dec[c][0].process(&self.hb, &src[..4 * N], &mut half);
                    self.dec[c][1].process(&self.hb, &half, out);
                }
            }
        }

        // global destinations follow the focused voice (and hold after it ends)
        if let Some(i) = self.focus {
            for d in 0..self.matrix.dests {
                self.global_off[d] = self.voices[i].mod_offset(d);
            }
        }

        // the FX racks, then the direct bus joins at the master
        let note = self.focus.map_or(60.0, |i| self.voices[i].pitch());
        let fcx = fx::Ctx::new(self.sr, bpm, beat, note, &self.params, &self.matrix, &self.global_off);
        let [mut main, direct, mut bus1, mut bus2] = bus;
        self.fx.process(&fcx, &mut main, &mut bus1, &mut bus2);
        let mut mix = main;
        for ch in 0..2 {
            for i in 0..N {
                mix[ch][i] += direct[ch][i];
            }
        }
        let g1 = match self.matrix.dest_of[p::MASTER_VOLUME as usize] {
            NONE => math::db_to_gain(self.params.plain(p::MASTER_VOLUME)),
            d => math::db_to_gain(self.params.modded(p::MASTER_VOLUME, self.global_off[d as usize])),
        };
        let g0 = self.master_prev;
        self.master_prev = g1;
        let dg = (g1 - g0) / N as f32;
        for i in 0..N {
            let g = g0 + dg * (i + 1) as f32;
            self.out[0][off + i] = mix[0][i] * g;
            self.out[1][off + i] = mix[1][i] * g;
        }

        let m = self.tap_mask;
        for (t, ch) in [(tap::MASTER_L, 0), (tap::MASTER_R, 1)] {
            if m & (1 << t) != 0 {
                let src: [f32; N] = self.out[ch][off..off + N].try_into().unwrap();
                self.taps[t][off..off + N].copy_from_slice(&src);
            }
        }
        let vt = &self.vtaps;
        let voice_taps: [(usize, &[f32; N]); 9] = [
            (tap::FOCUS_OSC, &vt.sum),
            (tap::FOCUS_OUT, &vt.out),
            (tap::FOCUS_OSC_A, &vt.osc[0]),
            (tap::FOCUS_OSC_B, &vt.osc[1]),
            (tap::FOCUS_OSC_C, &vt.osc[2]),
            (tap::FOCUS_FILTER, &vt.filter),
            (tap::FOCUS_FILTER2, &vt.filter2),
            (tap::FOCUS_SUB, &vt.sub),
            (tap::FOCUS_NOISE, &vt.noise),
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
        t[tel::OVERSAMPLE] = self.os as f32;
        for inst in 0..fx::INSTANCES {
            for band in 0..3 {
                t[tel::FX_GR + inst * 3 + band] = self.fx.gain_reduction(inst, band);
            }
            t[tel::FX_IR + inst] = self.fx.ir_taps(inst) as f32;
        }
        let dests = self.matrix.dests;
        for d in 0..MAX_MOD_SLOTS {
            t[tel::MOD_DEST + d] = if d < dests { self.matrix.dest_param[d] as f32 } else { -1.0 };
        }
        let focus = self.focus.map(|i| &self.voices[i]);
        for i in 0..LFOS {
            let (x, y, ph) = match focus {
                Some(v) => v.tel_lfo[i],
                None => (self.lfo_out[i].0, self.lfo_out[i].1, -1.0),
            };
            t[tel::LFO_VALUE + i] = x;
            t[tel::LFO_Y + i] = y;
            t[tel::LFO_PHASE + i] = if ph >= 0.0 { ph } else { self.lfo_global[i].phase() };
        }
        match focus {
            Some(v) => {
                t[tel::OSC_WT_POS..tel::OSC_WT_POS + OSC_COUNT].copy_from_slice(&v.tel_wt_pos);
                t[tel::FOCUS_PITCH] = v.pitch();
                for e in 0..4 {
                    t[tel::FOCUS_ENV + e] = v.env[e].level;
                }
                t[tel::FOCUS_CUTOFF] = v.tel_cutoff;
                t[tel::MACRO_VALUE..tel::MACRO_VALUE + MACROS].copy_from_slice(&v.tel_macro);
                for d in 0..MAX_MOD_SLOTS {
                    t[tel::MOD_VALUE + d] = if d < dests { v.tel_mod.0[d] } else { 0.0 };
                }
            }
            None => {
                for o in 0..OSC_COUNT {
                    t[tel::OSC_WT_POS + o] = self.params.plain(p::OSC_WT_POS[o]);
                }
                for e in 0..4 {
                    t[tel::FOCUS_ENV + e] = 0.0;
                }
                for i in 0..MACROS {
                    t[tel::MACRO_VALUE + i] = self.params.norm(p::MACRO_VALUE[i]);
                }
                for d in 0..MAX_MOD_SLOTS {
                    t[tel::MOD_VALUE + d] = if d < dests {
                        let id = self.matrix.dest_param[d];
                        let held = if is_global(id) { self.global_off[d] } else { 0.0 };
                        (self.params.norm(id) + held).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                }
            }
        }
    }
}
