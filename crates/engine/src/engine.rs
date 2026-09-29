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
use crate::osc::multi::Multi;
use crate::osc::sample::Recording;
use crate::osc::spectral::{self, Analysis};
use crate::samples::{self, Samples};
use crate::seq::{ClipNote, Ev, Events, Seq, SeqParams};
use crate::sources::{OscAssets, OscShared};
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
/// Host keys tracked for their key-ups.
const KEYS_DOWN: usize = 64;
/// An `auto_base` entry not in use.
const NO_AUTO: u16 = u16::MAX;
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
    /// Recordings, multisamples and analyses for the oscillator types, and what their voices share.
    assets: Box<OscAssets>,
    shared: Box<OscShared>,
    /// The transport, arpeggiator and clips, and a buffer for their events.
    seq: Box<Seq>,
    sev: Box<Events>,
    /// Keys down from the host: (key, note id, the note it became after the
    /// transpose and scale), so each key-up finds its own note however the
    /// keyboard settings changed meanwhile.
    keys: Vec<(u8, u32, u8)>,
    /// The parameters the playing clip automates, each with the value the
    /// host last set it to, which it returns to when the automation stops.
    auto_base: [(u16, f32); proto::CLIP_LANES],
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
            assets: Box::default(),
            shared: Box::default(),
            seq: Box::default(),
            sev: Box::default(),
            keys: Vec::with_capacity(KEYS_DOWN),
            auto_base: [(NO_AUTO, 0.0); proto::CLIP_LANES],
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
        for (i, v) in e.voices.iter_mut().enumerate() {
            v.slot = i as u16;
        }
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

    /// A command that carries a tail (tests; the host's go through `apply`).
    #[cfg(test)]
    pub fn command_with_tail(&mut self, cmd: Command, tail: &[u8]) {
        self.command_tail(cmd, tail)
    }

    /// The recording types' assets (native tests load them here: commands carry wasm32 pointers).
    /// A parameter's normalized value now (tests).
    #[cfg(test)]
    pub fn param(&self, id: u16) -> f32 {
        self.params.norm(id)
    }

    #[cfg(test)]
    pub fn assets_mut(&mut self) -> &mut OscAssets {
        &mut self.assets
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
            Command::SetArpPattern { bank, count } => {
                let n = (count as usize).min(proto::ARP_LANES * proto::ARP_STEPS).min(tail.len() / 4);
                let mut v = [0.0f32; proto::ARP_LANES * proto::ARP_STEPS];
                for (i, x) in v.iter_mut().enumerate().take(n) {
                    let f = rd(i * 4);
                    *x = if f.is_finite() { f } else { 0.0 };
                }
                self.seq.set_pattern(bank as usize, &v[..n]);
            }
            Command::SetClip { slot, count, length } => {
                let n = (count as usize).min(proto::CLIP_NOTES).min(tail.len() / 24);
                let mut notes = [ClipNote::default(); proto::CLIP_NOTES];
                for (i, c) in notes.iter_mut().enumerate().take(n) {
                    let f = |j: usize| {
                        let v = rd((i * 6 + j) * 4);
                        if v.is_finite() { v } else { 0.0 }
                    };
                    *c = ClipNote { start: f(0).max(0.0), len: f(1).max(0.0), key: f(2).clamp(0.0, 127.0) as u8, velocity: f(3).clamp(0.0, 1.0), chance: f(4).clamp(0.0, 1.0), bend: f(5).clamp(-48.0, 48.0) };
                }
                self.seq.set_clip(slot as usize, &notes[..n], length);
            }
            Command::SetClipLane { slot, lane, param, count } => {
                let n = (count as usize).min(proto::CLIP_POINTS).min(tail.len() / 8);
                let mut pts = [(0.0f32, 0.0f32); proto::CLIP_POINTS];
                for (i, pt) in pts.iter_mut().enumerate().take(n) {
                    *pt = (rd(i * 8), rd(i * 8 + 4).clamp(0.0, 1.0));
                }
                let param = if (param as usize) < p::COUNT { param } else { u16::MAX };
                self.seq.set_lane(slot as usize, lane as usize, param, &pts[..n]);
            }
            Command::SetSpectralFilter { osc, count } => {
                let n = (count as usize).min(proto::SPEC_FILTER_POINTS).min(tail.len() / 4);
                let mut pts = [1.0f32; proto::SPEC_FILTER_POINTS];
                for (i, x) in pts.iter_mut().enumerate().take(n) {
                    let v = rd(i * 4);
                    *x = if v.is_finite() { v.clamp(0.0, 1.0) } else { 1.0 };
                }
                if let Some(f) = self.assets.spec_filter.get_mut(osc as usize) {
                    spectral::filter_table(&pts[..n.max(1)], self.sr, f);
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
            Command::Transport { play } => {
                self.sev.n = 0;
                self.seq.transport(play != 0, &mut self.sev);
                self.play_seq_events();
            }
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
            Command::LoadOscSample { osc, channels, levels, slices, frames, rate, ptr, bytes } => {
                let Some(slot) = self.assets.recs.get_mut(osc as usize) else {
                    self.table_errors += 1;
                    return;
                };
                if ptr == 0 {
                    *slot = None;
                    return;
                }
                let asset = unsafe { AssetBuf::from_raw(ptr as usize as *mut u8, bytes as usize) };
                match Recording::new(asset, frames as usize, channels as usize, levels as usize, rate, slices as usize) {
                    Ok(r) => *slot = Some(r),
                    Err(_) => self.table_errors += 1,
                }
            }
            Command::LoadMulti { osc, zones, ptr, bytes } => {
                let Some(slot) = self.assets.multis.get_mut(osc as usize) else {
                    self.table_errors += 1;
                    return;
                };
                if ptr == 0 {
                    *slot = None;
                    return;
                }
                let asset = unsafe { AssetBuf::from_raw(ptr as usize as *mut u8, bytes as usize) };
                match Multi::new(asset, zones as usize) {
                    Ok(m) => *slot = Some(m),
                    Err(_) => self.table_errors += 1,
                }
            }
            Command::LoadSpectral { osc, frames, rate, ptr, bytes } => {
                let Some(slot) = self.assets.analyses.get_mut(osc as usize) else {
                    self.table_errors += 1;
                    return;
                };
                if ptr == 0 {
                    *slot = None;
                    return;
                }
                let asset = unsafe { AssetBuf::from_raw(ptr as usize as *mut u8, bytes as usize) };
                match Analysis::new(asset, frames as usize, rate) {
                    Ok(a) => *slot = Some(a),
                    Err(_) => self.table_errors += 1,
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
            Command::SetLfoShape { .. }
            | Command::SetOscCurve { .. }
            | Command::SetChain { .. }
            | Command::SetTuning { .. }
            | Command::SetSpectralFilter { .. }
            | Command::SetArpPattern { .. }
            | Command::SetClip { .. }
            | Command::SetClipLane { .. } => {} // need their tails: see `apply`
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
        for (i, v) in self.voices.iter_mut().enumerate() {
            // each keeps its slot: the grain pool and spectral voices are found by it
            *v = Voice::default();
            v.slot = i as u16;
        }
        self.shared.pool = crate::osc::granular::GrainPool::default();
        self.focus = None;
        self.held.clear();
        self.last_pitch = None;
        self.sustain = false;
        self.bend = self.bend_target;
        for b in self.auto_base.iter_mut() {
            if b.0 != NO_AUTO {
                self.params.set(b.0 as usize, b.1);
                b.0 = NO_AUTO;
            }
        }
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
            self.sequence();
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
                } else if let Some(b) = self.auto_base.iter_mut().find(|b| b.0 == id) {
                    // automated: the clip keeps it, and it goes to this value after
                    b.1 = value;
                }
            }
            Command::NoteOn { note, channel, velocity, note_id } => self.key_down(note, channel, velocity, note_id, offset),
            Command::NoteOff { note, channel, velocity, note_id } => self.key_up(note, channel, velocity, note_id),
            Command::AllNotesOff => {
                self.seq.clear_keys();
                self.keys.clear();
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

    /// The settings the sequencer reads this sub-block.
    fn seq_params(&self) -> SeqParams {
        let pr = &self.params;
        let f = |id: u16| pr.plain(id);
        SeqParams {
            bpm: f(p::GLOBAL_BPM),
            swing: f(p::GLOBAL_SWING),
            arp_on: f(p::ARP_ENABLE) >= 0.5,
            shape: f(p::ARP_SHAPE) as u8,
            rate: f(p::ARP_RATE) as u8,
            octaves: f(p::ARP_OCTAVES) as u8,
            shift: f(p::ARP_SHIFT) as i8,
            gate: f(p::ARP_GATE),
            chance: f(p::ARP_CHANCE),
            repeats: f(p::ARP_REPEATS) as u8,
            vel_ramp: f(p::ARP_VEL_RAMP),
            offset: f(p::ARP_OFFSET),
            retrigger: f(p::ARP_RETRIGGER) as u8,
            hold: f(p::ARP_HOLD) >= 0.5,
            steps: f(p::ARP_STEPS) as u8,
            bank: f(p::ARP_BANK) as u8,
            clip_on: f(p::CLIP_ENABLE) >= 0.5,
            clip_slot: f(p::CLIP_SLOT) as u8,
            quantize: f(p::CLIP_QUANTIZE) as u8,
            clip_transpose: f(p::CLIP_TRANSPOSE) as i8,
            clip_loop: f(p::CLIP_LOOP) >= 0.5,
        }
    }

    /// The sequencer's work for the next sub-block: its notes, then the playing clip's automation.
    fn sequence(&mut self) {
        let sp = self.seq_params();
        self.sev.n = 0;
        self.seq.advance(&sp, N, self.sr, &mut self.sev);
        self.play_seq_events();
        let mut auto = [(0u16, 0.0f32); proto::CLIP_LANES];
        let n = self.seq.automation(&mut auto);
        let auto = &auto[..n];
        // parameters no longer automated go back to where the host set them
        for b in self.auto_base.iter_mut() {
            if b.0 != NO_AUTO && !auto.iter().any(|a| a.0 == b.0) {
                self.params.set(b.0 as usize, b.1);
                b.0 = NO_AUTO;
            }
        }
        for &(id, v) in auto {
            if !self.auto_base.iter().any(|b| b.0 == id)
                && let Some(b) = self.auto_base.iter_mut().find(|b| b.0 == NO_AUTO)
            {
                *b = (id, self.params.target(id));
            }
            self.params.set(id as usize, v);
        }
    }

    fn play_seq_events(&mut self) {
        let n = self.sev.n;
        for i in 0..n {
            let (at, e) = self.sev.list[i];
            match e {
                Ev::On { note, velocity, id } => self.note_on(note, 0, velocity, id, at),
                Ev::Off { note, id } => self.note_off(note, 0, 0.0, id),
            }
        }
        self.sev.n = 0;
    }

    /// A played key, after the keyboard's transpose and scale.
    fn map_key(&self, note: u8) -> u8 {
        const SCALES: [u16; 14] = [
            0xFFF,
            0b1010_1011_0101,
            0b0101_1010_1101,
            0b0110_1010_1101,
            0b0101_1010_1011,
            0b1010_1101_0101,
            0b0110_1011_0101,
            0b0101_0110_1011,
            0b1001_1010_1101,
            0b1010_1010_1101,
            0b0010_1001_0101,
            0b0100_1010_1001,
            0b0100_1110_1001,
            0b0101_0101_0101,
        ];
        let t = (note as i32 + self.params.plain(p::KEYS_TRANSPOSE) as i32).clamp(0, 127);
        let scale = SCALES[(self.params.plain(p::KEYS_SCALE) as usize).min(SCALES.len() - 1)];
        if scale == 0xFFF {
            return t as u8;
        }
        let root = self.params.plain(p::KEYS_ROOT) as i32;
        // the nearest note of the scale, the lower one on a tie
        for d in 0..12 {
            for c in [t - d, t + d] {
                if (0..128).contains(&c) && scale & (1 << (c - root).rem_euclid(12)) != 0 {
                    return c as u8;
                }
            }
        }
        t as u8
    }

    /// A key from the host (or MIDI): trigger keys launch clips; with the arp on it joins the held keys.
    fn key_down(&mut self, note: u8, channel: u8, velocity: f32, note_id: u32, offset: usize) {
        if velocity <= 0.0 {
            return self.key_up(note, channel, 0.0, note_id);
        }
        if self.params.plain(p::CLIP_ENABLE) >= 0.5 && self.params.plain(p::CLIP_TRIGGER_KEYS) >= 0.5 && (24..36).contains(&note) {
            // C1..B1 pick clips 1..12 (the slot parameter runs 1..12)
            self.params.set(p::CLIP_SLOT as usize, (note - 24) as f32 / 11.0);
            return;
        }
        let mapped = self.map_key(note);
        if self.keys.len() >= KEYS_DOWN {
            self.keys.remove(0);
        }
        self.keys.push((note, note_id, mapped));
        if self.params.plain(p::ARP_ENABLE) >= 0.5 {
            let sp = self.seq_params();
            let beat = self.seq.beat_at(offset);
            let v = self.curve_velocity(velocity);
            self.seq.key_down(mapped, v, beat, &sp);
        } else {
            self.note_on(mapped, channel, velocity, note_id, offset);
        }
    }

    fn key_up(&mut self, note: u8, channel: u8, velocity: f32, note_id: u32) {
        // the key's own entry: by note id when there is one, else the newest press of that key
        let at = if note_id != 0 { self.keys.iter().rposition(|k| k.1 == note_id) } else { None }.or_else(|| self.keys.iter().rposition(|k| k.0 == note));
        let mapped = match at {
            Some(i) => self.keys.remove(i).2,
            None => self.map_key(note),
        };
        let sp = self.seq_params();
        // the arp lets go of the note once no other key still holds it
        if !self.keys.iter().any(|k| k.2 == mapped) {
            self.seq.key_up(mapped, &sp);
        }
        // a voice started before the arp came on still gets its note-off
        self.note_off(mapped, channel, velocity, note_id);
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
            assets: &self.assets,
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
                v.render(&cx, &mut self.buses, &mut self.scratch, &mut self.shared, Some(&mut self.vtaps));
                focus_rendered = true;
            } else {
                v.render(&cx, &mut self.buses, &mut self.scratch, &mut self.shared, None);
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
        for o in 0..OSC_COUNT {
            t[tel::OSC_PLAY + o] = match self.focus {
                Some(i) if self.voices[i].active => self.voices[i].play_pos(o),
                _ => -1.0,
            };
            let a = &self.assets;
            t[tel::OSC_ASSETS + o * 3] = a.recs[o].as_ref().map_or(0, |r| r.frames) as f32;
            t[tel::OSC_ASSETS + o * 3 + 1] = a.multis[o].as_ref().map_or(0, |m| m.count) as f32;
            t[tel::OSC_ASSETS + o * 3 + 2] = a.analyses[o].as_ref().map_or(0, |x| x.frames) as f32;
        }
        t[tel::GRAINS] = self.shared.pool.active() as f32;
        t[tel::PLAYING] = if self.seq.playing { 1.0 } else { 0.0 };
        t[tel::BEAT] = self.seq.beat as f32;
        t[tel::ARP_STEP] = self.seq.step as f32;
        t[tel::CLIP_PLAYING] = self.seq.clip_playing.map_or(-1.0, |s| s as f32);
        t[tel::CLIP_POS] = self.seq.clip_pos() as f32;
        t[tel::SEQ_NOTES] = self.seq.sounding() as f32;
        t[tel::GRAINS_STOLEN] = self.shared.pool.stolen as f32;
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
