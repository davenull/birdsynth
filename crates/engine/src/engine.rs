//! The engine: command intake, voice allocation and the render loop.
//!
//! Rendering runs on a fixed 16-frame control grid (`SUB_BLOCK`). Note-ons
//! start at their exact frame; every other command lands on the first grid
//! boundary at or after its frame. Nothing in `render` allocates.

use wt_dsp::{math, saw};

use crate::env::EnvTimes;
use crate::events::{Event, EventQueue};
use crate::spec::params as p;
use crate::spec::protocol::{self as proto, Command, HEADER_BYTES, MAX_BLOCK, MAX_VOICES, SUB_BLOCK, VOICE_SLOTS, tap, tel};
use crate::params::ParamStore;
use crate::voice::{Voice, VoiceCtx};

const N: usize = SUB_BLOCK;
const QUEUE_CAPACITY: usize = 4096;
/// Fade used when a voice is stolen or all sound is stopped.
const KILL_SECONDS: f32 = 0.003;

const _: () = assert!(tel::VOICE_NOTE_LEN == VOICE_SLOTS && tel::VOICE_LEVEL_LEN == VOICE_SLOTS);
const _: () = assert!(MAX_BLOCK.is_multiple_of(SUB_BLOCK));

#[derive(Debug, PartialEq, Eq)]
pub enum RenderError {
    /// Frames must be a positive multiple of SUB_BLOCK, at most MAX_BLOCK.
    BadFrames,
}

/// One-pole smoothing stepped once per sub-block; users ramp linearly from
/// `prev` to `cur` across the sub-block.
#[derive(Clone, Copy, Debug)]
struct Smooth {
    prev: f32,
    cur: f32,
    coef: f32,
}

impl Smooth {
    fn new(v: f32, sr: f32, ms: f32) -> Self {
        let coef = 1.0 - (-(N as f32) / (ms * 0.001 * sr)).exp();
        Smooth { prev: v, cur: v, coef }
    }

    fn step(&mut self, target: f32) {
        self.prev = self.cur;
        let next = self.cur + (target - self.cur) * self.coef;
        self.cur = if (target - next).abs() < 1e-6 { target } else { next };
    }

    fn snap(&mut self, v: f32) {
        self.prev = v;
        self.cur = v;
    }
}

pub struct Engine {
    sr: f32,
    /// Absolute frame of the next sample to render.
    frame: u64,
    params: ParamStore,
    queue: EventQueue,
    voices: Vec<Voice>,
    age: u64,
    focus: Option<usize>,
    table: Vec<f32>,
    out: [Vec<f32>; 2],
    taps: Vec<Vec<f32>>,
    tap_mask: u32,
    tel: [f32; tel::LEN],
    master: Smooth,
    osc_level: Smooth,
    unknown_cmds: u32,
    mix: [[f32; N]; 2],
    focus_osc: [f32; N],
    focus_out: [f32; N],
    scratch_osc: [f32; N],
    scratch_out: [f32; N],
}

impl Engine {
    pub fn new(sample_rate: f32) -> Box<Engine> {
        let params = ParamStore::default();
        let master = Smooth::new(math::db_to_gain(params.plain(p::MASTER_VOLUME)), sample_rate, 20.0);
        let osc_level = Smooth::new(params.plain(p::OSC_LEVEL[0]), sample_rate, 10.0);
        let mut e = Box::new(Engine {
            sr: sample_rate,
            frame: 0,
            params,
            queue: EventQueue::with_capacity(QUEUE_CAPACITY),
            voices: vec![Voice::default(); VOICE_SLOTS],
            age: 0,
            focus: None,
            table: saw::saw_frame(),
            out: [vec![0.0; MAX_BLOCK], vec![0.0; MAX_BLOCK]],
            taps: (0..tap::COUNT).map(|_| vec![0.0; MAX_BLOCK]).collect(),
            tap_mask: 0,
            tel: [0.0; tel::LEN],
            master,
            osc_level,
            unknown_cmds: 0,
            mix: [[0.0; N]; 2],
            focus_osc: [0.0; N],
            focus_out: [0.0; N],
            scratch_osc: [0.0; N],
            scratch_out: [0.0; N],
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

    /// Accept one decoded command. Taps, reset and debug take effect now;
    /// everything else is queued for its frame.
    pub fn command(&mut self, cmd: Command, frame: f64) {
        match cmd {
            Command::SetTaps { mask } => self.tap_mask = mask,
            Command::Reset => self.reset(),
            Command::Debug { code, .. } => {
                if code == proto::debug::TRAP {
                    panic!("debug trap requested by the host");
                }
            }
            _ => {
                let frame = if frame > 0.0 { frame as u64 } else { 0 };
                self.queue.push(Event { frame, cmd });
            }
        }
    }

    /// Clear voices, pending events and smoothing; parameters are kept.
    pub fn reset(&mut self) {
        self.queue.clear();
        for v in &mut self.voices {
            *v = Voice::default();
        }
        self.focus = None;
        self.master.snap(math::db_to_gain(self.params.plain(p::MASTER_VOLUME)));
        self.osc_level.snap(self.params.plain(p::OSC_LEVEL[0]));
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
            Command::NoteOff { note, channel, note_id, .. } => self.note_off(note, channel, note_id),
            Command::AllNotesOff => {
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.release();
                }
            }
            Command::AllSoundOff => {
                let k = self.kill_samples();
                for v in self.voices.iter_mut().filter(|v| v.active) {
                    v.kill(k);
                }
            }
            // Wheels and pressure arrive with the modulation matrix.
            Command::PitchBend { .. } | Command::Controller { .. } | Command::ChannelPressure { .. } | Command::PolyPressure { .. } => {}
            Command::SetTaps { .. } | Command::Reset | Command::Debug { .. } => {}
        }
    }

    fn kill_samples(&self) -> f32 {
        KILL_SECONDS * self.sr
    }

    fn note_on(&mut self, note: u8, channel: u8, velocity: f32, note_id: u32, offset: usize) {
        if velocity <= 0.0 {
            return self.note_off(note, channel, note_id);
        }
        let poly = (self.params.plain(p::VOICE_POLYPHONY) as usize).clamp(1, MAX_VOICES);
        let sounding = self.voices.iter().filter(|v| v.active && !v.killing()).count();
        if sounding >= poly {
            // Steal the oldest released voice, else the oldest voice.
            let victim = self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.active && !v.killing())
                .min_by_key(|(_, v)| (!v.released, v.age))
                .map(|(i, _)| i);
            if let Some(i) = victim {
                let k = self.kill_samples();
                self.voices[i].kill(k);
            }
        }
        let slot = match self.voices.iter().position(|v| !v.active) {
            Some(i) => i,
            // Every slot is still fading: reuse the oldest outright.
            None => (0..VOICE_SLOTS).min_by_key(|&i| self.voices[i].age).unwrap_or(0),
        };
        self.age += 1;
        self.voices[slot].start(note, channel, velocity, note_id, offset, self.age);
        self.focus = Some(slot);
    }

    fn note_off(&mut self, note: u8, channel: u8, note_id: u32) {
        for v in self.voices.iter_mut().filter(|v| v.active && !v.released) {
            let hit = if note_id != 0 { v.note_id == note_id } else { v.note == note && v.channel == channel };
            if hit {
                v.release();
            }
        }
    }

    fn render_sub_block(&mut self, off: usize) {
        self.master.step(math::db_to_gain(self.params.plain(p::MASTER_VOLUME)));
        self.osc_level.step(self.params.plain(p::OSC_LEVEL[0]));
        let pr = &self.params;
        let cx = VoiceCtx {
            sr: self.sr,
            table: &self.table,
            env: EnvTimes::from_params(pr, 0, self.sr),
            osc_on: pr.plain(p::OSC_ENABLE[0]) >= 0.5,
            pitch: pr.plain(p::OSC_OCTAVE[0]) * 12.0 + pr.plain(p::OSC_SEMI[0]) + pr.plain(p::OSC_FINE[0]) * 0.01 + pr.plain(p::OSC_COARSE[0]),
            level: (self.osc_level.prev, self.osc_level.cur),
            pan: math::balance(pr.plain(p::OSC_PAN[0])),
        };

        self.mix = [[0.0; N]; 2];
        let mut focus_rendered = false;
        for (i, v) in self.voices.iter_mut().enumerate() {
            if !v.active {
                continue;
            }
            if self.focus == Some(i) {
                v.render(&cx, &mut self.mix, &mut self.focus_osc, &mut self.focus_out);
                focus_rendered = true;
            } else {
                v.render(&cx, &mut self.mix, &mut self.scratch_osc, &mut self.scratch_out);
            }
        }
        if !focus_rendered {
            self.focus_osc = [0.0; N];
            self.focus_out = [0.0; N];
        }

        let (g0, g1) = (self.master.prev, self.master.cur);
        let dg = (g1 - g0) / N as f32;
        for i in 0..N {
            let g = g0 + dg * (i + 1) as f32;
            self.out[0][off + i] = self.mix[0][i] * g;
            self.out[1][off + i] = self.mix[1][i] * g;
        }

        let m = self.tap_mask;
        if m & (1 << tap::MASTER_L) != 0 {
            self.taps[tap::MASTER_L][off..off + N].copy_from_slice(&self.out[0][off..off + N]);
        }
        if m & (1 << tap::MASTER_R) != 0 {
            self.taps[tap::MASTER_R][off..off + N].copy_from_slice(&self.out[1][off..off + N]);
        }
        if m & (1 << tap::FOCUS_OSC) != 0 {
            self.taps[tap::FOCUS_OSC][off..off + N].copy_from_slice(&self.focus_osc);
        }
        if m & (1 << tap::FOCUS_OUT) != 0 {
            self.taps[tap::FOCUS_OUT][off..off + N].copy_from_slice(&self.focus_out);
        }

        self.update_focus();
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
            t[tel::VOICE_LEVEL + i] = if v.active { v.env.level } else { 0.0 };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn norm(id: u16, plain: f32) -> f32 {
        let info = crate::spec::params::INFO[id as usize];
        (plain - info.min) / (info.max - info.min)
    }

    fn render(e: &mut Engine, frames: usize) -> Vec<f32> {
        let mut out = Vec::with_capacity(frames);
        let mut left = frames;
        while left > 0 {
            let n = left.min(128);
            e.render(n.div_ceil(N) * N, -1.0).unwrap();
            out.extend_from_slice(&e.out(0)[..n]);
            left -= n;
        }
        out
    }

    fn rising_crossings(x: &[f32]) -> Vec<f64> {
        let mut t = Vec::new();
        for i in 1..x.len() {
            if x[i - 1] < 0.0 && x[i] >= 0.0 {
                t.push((i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64);
            }
        }
        t
    }

    #[test]
    fn a4_is_440_hz() {
        for &sr in &[44_100.0f32, 48_000.0] {
            let mut e = Engine::new(sr);
            e.command(Command::NoteOn { note: 69, channel: 0, velocity: 1.0, note_id: 1 }, 0.0);
            let x = render(&mut e, (sr * 2.0) as usize);
            let z = rising_crossings(&x[(sr * 0.1) as usize..]);
            let hz = (z.len() - 1) as f64 * sr as f64 / (z[z.len() - 1] - z[0]);
            assert!((hz - 440.0).abs() < 0.01, "{sr}: {hz}");
        }
    }

    #[test]
    fn note_on_starts_on_its_exact_frame() {
        let mut e = Engine::new(SR);
        e.command(Command::NoteOn { note: 60, channel: 0, velocity: 1.0, note_id: 1 }, 100.0);
        // the saw starts at 0 rising, so the first sounding sample is small but nonzero
        let x = render(&mut e, 256);
        assert!(x[..100].iter().all(|&v| v == 0.0));
        assert!(x[101..110].iter().all(|&v| v != 0.0));
    }

    #[test]
    fn params_land_on_the_next_grid_boundary() {
        let mut e = Engine::new(SR);
        e.command(Command::SetParam { id: p::VOICE_POLYPHONY, value: norm(p::VOICE_POLYPHONY, 3.0) }, 5.0);
        e.render(16, 0.0).unwrap();
        assert_eq!(e.params().plain(p::VOICE_POLYPHONY), 8.0);
        e.render(16, -1.0).unwrap();
        assert_eq!(e.params().plain(p::VOICE_POLYPHONY), 3.0);
    }

    #[test]
    fn steals_the_oldest_voice_past_polyphony() {
        let mut e = Engine::new(SR);
        e.command(Command::SetParam { id: p::VOICE_POLYPHONY, value: norm(p::VOICE_POLYPHONY, 2.0) }, 0.0);
        for (i, n) in [60u8, 64, 67].iter().enumerate() {
            e.command(Command::NoteOn { note: *n, channel: 0, velocity: 1.0, note_id: i as u32 + 1 }, 0.0);
        }
        render(&mut e, 128 * 4); // past the 3 ms steal fade
        let t = e.telemetry();
        assert_eq!(t[tel::VOICES_ACTIVE], 2.0);
        let notes: Vec<f32> = (0..VOICE_SLOTS).map(|i| t[tel::VOICE_NOTE + i]).filter(|&n| n >= 0.0).collect();
        assert_eq!(notes.len(), 2);
        assert!(!notes.contains(&60.0), "{notes:?}");
    }

    #[test]
    fn note_off_by_id_releases_only_that_note() {
        let mut e = Engine::new(SR);
        e.command(Command::NoteOn { note: 60, channel: 0, velocity: 1.0, note_id: 7 }, 0.0);
        e.command(Command::NoteOn { note: 60, channel: 0, velocity: 1.0, note_id: 8 }, 0.0);
        render(&mut e, 128);
        e.command(Command::NoteOff { note: 60, channel: 0, velocity: 0.0, note_id: 7 }, 0.0);
        render(&mut e, 48_000 / 4); // release is 15 ms
        assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 1.0);
    }

    #[test]
    fn rejects_bad_block_sizes() {
        let mut e = Engine::new(SR);
        assert_eq!(e.render(15, 0.0), Err(RenderError::BadFrames));
        assert_eq!(e.render(MAX_BLOCK + 16, 0.0), Err(RenderError::BadFrames));
        assert!(e.render(MAX_BLOCK, 0.0).is_ok());
    }

    #[test]
    fn decodes_a_batch_and_counts_garbage() {
        let mut buf = Vec::new();
        // SetParam(id=VOICE_POLYPHONY, value) at frame 0
        buf.extend_from_slice(&proto::op::SET_PARAM.to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes());
        buf.extend_from_slice(&(proto::bytes::SET_PARAM as u32).to_le_bytes());
        buf.extend_from_slice(&0f64.to_le_bytes());
        buf.extend_from_slice(&p::VOICE_POLYPHONY.to_le_bytes());
        buf.extend_from_slice(&[0, 0]);
        buf.extend_from_slice(&norm(p::VOICE_POLYPHONY, 4.0).to_le_bytes());
        // an unknown op
        buf.extend_from_slice(&999u16.to_le_bytes());
        buf.extend_from_slice(&[0, 0]);
        buf.extend_from_slice(&0u32.to_le_bytes());
        buf.extend_from_slice(&0f64.to_le_bytes());
        let mut e = Engine::new(SR);
        assert_eq!(e.apply(&buf), 1);
        e.render(16, 0.0).unwrap();
        assert_eq!(e.params().plain(p::VOICE_POLYPHONY), 4.0);
        assert_eq!(e.telemetry()[tel::UNKNOWN_CMDS], 1.0);
    }
}
