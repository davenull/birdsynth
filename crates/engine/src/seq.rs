//! The sequencer: a transport clock, the arpeggiator and the clips, working
//! a sub-block at a time and handing the engine note events at exact frame
//! offsets. Every note it starts is kept in a ledger with the beat it ends
//! on, so edits, clip switches and stops can never strand a note: whatever
//! happens, each started note gets its note-off.
//!
//! Time is in beats. While the transport runs its position restarts at 0 on
//! each start; while it's stopped the clock still runs (from the engine's
//! start), so the arpeggiator's Beat retrigger has a grid to keep to.

use wt_dsp::rng::Rng;

use crate::spec::protocol::{ARP_BANKS, ARP_LANES, ARP_STEPS, CLIP_LANES, CLIP_NOTES, CLIP_POINTS, CLIP_SLOTS};

/// Note ids the sequencer uses (the host's are below).
pub const SEQ_ID: u32 = 0x4000_0000;
const MAX_HELD: usize = 32;
const MAX_SEQ: usize = 128;
/// Strummed notes waiting: two steps' chords (a swung step's strum can reach past the next step).
const STRUM_QUEUE: usize = 2 * MAX_HELD;
const LEDGER: usize = 128;
/// Room for every ledgered note-off, a full chord and a sub-block's clip notes at once: none is ever dropped.
pub const MAX_EVENTS: usize = 512;

pub const SHAPE_DOWN: u8 = 1;
pub const SHAPE_UPDOWN: u8 = 2;
pub const SHAPE_UPANDDOWN: u8 = 3;
pub const SHAPE_CONVERGE: u8 = 4;
pub const SHAPE_DIVERGE: u8 = 5;
pub const SHAPE_THUMB: u8 = 6;
pub const SHAPE_PLAYED: u8 = 7;
pub const SHAPE_CHORD: u8 = 8;
pub const SHAPE_RANDOM: u8 = 9;
pub const SHAPE_PATTERN: u8 = 10;

pub const RETRIG_BEAT: u8 = 0;
pub const RETRIG_LAUNCH: u8 = 1;
pub const RETRIG_NOTE: u8 = 2;

/// Lanes of a pattern step.
pub const L_ON: usize = 0;
pub const L_GATE: usize = 1;
pub const L_VEL: usize = 2;
pub const L_CHANCE: usize = 3;
pub const L_BEND: usize = 4;
pub const L_STRUM: usize = 5;
pub const L_DEGREE: usize = 6;

/// Step lengths of the arp's Rate options, in beats.
pub const ARP_RATES: [f64; 16] = [4.0, 2.0, 1.0, 0.5, 0.25, 0.125, 0.0625, 3.0, 1.5, 0.75, 0.375, 4.0 / 3.0, 2.0 / 3.0, 1.0 / 3.0, 1.0 / 6.0, 1.0 / 12.0];
/// Launch quantize options, in beats (0: now).
pub const QUANTIZE: [f64; 6] = [0.0, 0.25, 1.0, 4.0, 8.0, 16.0];

/// One sub-block's clock: frames [f0, f1), and beats to frames.
struct Clock {
    f0: u64,
    f1: u64,
    base_frame: u64,
    base_beat: f64,
    spb: f64,
}

impl Clock {
    /// The frame a beat falls on (rounded: exact for beats on the tempo's grid).
    fn frame(&self, beat: f64) -> i64 {
        (self.base_frame as f64 + (beat - self.base_beat) * self.spb).round() as i64
    }

    fn beat(&self, frame: u64) -> f64 {
        self.base_beat + (frame as i64 - self.base_frame as i64) as f64 / self.spb
    }

    /// A beat's offset into this sub-block.
    fn at(&self, beat: f64) -> usize {
        (self.frame(beat) - self.f0 as i64).clamp(0, (self.f1 - self.f0) as i64 - 1) as usize
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ev {
    On { note: u8, velocity: f32, id: u32 },
    Off { note: u8, id: u32 },
}

/// Events for one sub-block, at offsets within it.
pub struct Events {
    pub list: [(usize, Ev); MAX_EVENTS],
    pub n: usize,
}

impl Default for Events {
    fn default() -> Self {
        Events { list: [(0, Ev::Off { note: 0, id: 0 }); MAX_EVENTS], n: 0 }
    }
}

impl Events {
    fn push(&mut self, at: usize, e: Ev) {
        if self.n < MAX_EVENTS {
            self.list[self.n] = (at, e);
            self.n += 1;
        }
    }

    #[cfg(test)]
    pub fn as_slice(&self) -> &[(usize, Ev)] {
        &self.list[..self.n]
    }

    /// In order of offset, note-offs first at the same offset (so a repeated note restarts cleanly).
    fn sort(&mut self) {
        self.list[..self.n].sort_unstable_by_key(|(at, e)| (*at, matches!(e, Ev::On { .. })));
    }
}

/// Who started a sounding note.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owner {
    Arp,
    Clip,
}

#[derive(Clone, Copy, Debug)]
struct Sounding {
    id: u32,
    note: u8,
    off: f64,
    owner: Owner,
}

/// Settings the sequencer reads each sub-block (resolved by the engine).
#[derive(Clone, Copy, Debug, Default)]
pub struct SeqParams {
    pub bpm: f32,
    pub swing: f32,
    pub arp_on: bool,
    pub shape: u8,
    pub rate: u8,
    pub octaves: u8,
    pub shift: i8,
    pub gate: f32,
    pub chance: f32,
    pub repeats: u8,
    pub vel_ramp: f32,
    pub offset: f32,
    pub retrigger: u8,
    pub hold: bool,
    pub steps: u8,
    pub bank: u8,
    pub clip_on: bool,
    pub clip_slot: u8,
    pub quantize: u8,
    pub clip_transpose: i8,
    pub clip_loop: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClipNote {
    pub start: f32,
    pub len: f32,
    pub key: u8,
    pub velocity: f32,
    pub chance: f32,
    pub bend: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct Lane {
    pub param: u16,
    pub n: usize,
    pub points: [(f32, f32); CLIP_POINTS],
}

impl Default for Lane {
    fn default() -> Self {
        Lane { param: u16::MAX, n: 0, points: [(0.0, 0.0); CLIP_POINTS] }
    }
}

impl Lane {
    /// The lane's value at beat `b` (holding the ends).
    pub fn at(&self, b: f32) -> Option<f32> {
        if self.param == u16::MAX || self.n == 0 {
            return None;
        }
        let p = &self.points[..self.n];
        if b <= p[0].0 {
            return Some(p[0].1);
        }
        for w in p.windows(2) {
            if b < w[1].0 {
                let t = (b - w[0].0) / (w[1].0 - w[0].0).max(1e-9);
                return Some(w[0].1 + (w[1].1 - w[0].1) * t);
            }
        }
        Some(p[self.n - 1].1)
    }
}

pub struct Clip {
    pub notes: [ClipNote; CLIP_NOTES],
    pub n: usize,
    pub length: f32,
    pub lanes: [Lane; CLIP_LANES],
}

impl Default for Clip {
    fn default() -> Self {
        Clip { notes: [ClipNote::default(); CLIP_NOTES], n: 0, length: 4.0, lanes: [Lane::default(); CLIP_LANES] }
    }
}

pub struct Seq {
    pub playing: bool,
    /// The clock, in beats (at the end of the last sub-block).
    pub beat: f64,
    /// Frames since the engine started, and the clock's anchor: `base_beat`
    /// at frame `base_frame`, `spb` frames per beat (rebased when the tempo moves).
    frame: u64,
    base_frame: u64,
    base_beat: f64,
    spb: f64,
    // arp
    held: [(u8, f32, u32); MAX_HELD],
    nheld: usize,
    /// Played-order counter.
    order: u32,
    /// Held notes are latched (Hold, keys all up).
    latched: bool,
    seq: [(u8, f32); MAX_SEQ],
    seq_len: usize,
    seq_dirty: bool,
    /// Beat of step 0, and the next step's index from there.
    origin: f64,
    next: u64,
    running: bool,
    pub step: i32,
    rng: Rng,
    pub patterns: Box<[[f32; ARP_LANES * ARP_STEPS]; ARP_BANKS]>,
    // clips
    pub clips: Box<[Clip]>,
    pub clip_playing: Option<usize>,
    clip_start: f64,
    pending: Option<(usize, f64)>,
    /// The beat the transport just started at: the clip starts with it (at the song position,
    /// if that's mid-way), rather than waiting for the launch quantize.
    song_start: Option<f64>,
    /// No clip note before this beat plays (a start mid-way through a sub-block).
    play_from: f64,
    /// The clock's rate against the tempo (1 but for the small trim that keeps a linked instance locked).
    rate: f64,
    /// A strummed chord's later notes, until their frames come: (beat, note, velocity, off beat).
    strum: [(f64, u8, f32, f64); STRUM_QUEUE],
    nstrum: usize,
    // the ledger
    ledger: [Sounding; LEDGER],
    nledger: usize,
    next_id: u32,
}

impl Default for Seq {
    fn default() -> Self {
        let mut patterns = Box::new([[0.0f32; ARP_LANES * ARP_STEPS]; ARP_BANKS]);
        for p in patterns.iter_mut() {
            *p = default_pattern();
        }
        Seq {
            playing: false,
            beat: 0.0,
            frame: 0,
            base_frame: 0,
            base_beat: 0.0,
            spb: 0.0,
            held: [(0, 0.0, 0); MAX_HELD],
            nheld: 0,
            order: 0,
            latched: false,
            seq: [(0, 0.0); MAX_SEQ],
            seq_len: 0,
            seq_dirty: false,
            strum: [(0.0, 0, 0.0, 0.0); STRUM_QUEUE],
            nstrum: 0,
            origin: 0.0,
            next: 0,
            running: false,
            step: -1,
            rng: Rng::new(0x5eed),
            patterns,
            clips: (0..CLIP_SLOTS).map(|_| Clip::default()).collect(),
            clip_playing: None,
            clip_start: 0.0,
            pending: None,
            song_start: None,
            play_from: f64::NEG_INFINITY,
            rate: 1.0,
            ledger: [Sounding { id: 0, note: 0, off: 0.0, owner: Owner::Arp }; LEDGER],
            nledger: 0,
            next_id: 1,
        }
    }
}

/// How late full swing makes a clip's off-beat sixteenths: half a sixteenth, in beats.
const SWING_MAX: f64 = 0.125;

/// A clip position with swing: time within each eighth is bent so its second
/// sixteenth lands `delay` beats late (the eighths themselves stay put, so
/// notes keep their order).
fn swung(pos: f64, delay: f64) -> f64 {
    if delay <= 0.0 {
        return pos;
    }
    let base = (pos / 0.5).floor() * 0.5;
    let u = pos - base;
    let u = if u < 0.25 { u * (0.25 + delay) / 0.25 } else { 0.25 + delay + (u - 0.25) * (0.25 - delay) / 0.25 };
    base + u
}

/// Every step on, full gate multiplier 1 (lane value 0.5), full velocity and chance, no bend or strum, degrees in order.
pub fn default_pattern() -> [f32; ARP_LANES * ARP_STEPS] {
    let mut p = [0.0f32; ARP_LANES * ARP_STEPS];
    for s in 0..ARP_STEPS {
        p[L_ON * ARP_STEPS + s] = 1.0;
        p[L_GATE * ARP_STEPS + s] = 0.5;
        p[L_VEL * ARP_STEPS + s] = 1.0;
        p[L_CHANCE * ARP_STEPS + s] = 1.0;
        p[L_DEGREE * ARP_STEPS + s] = s as f32;
    }
    p
}

/// The order the held notes play in, before octaves (and repeats) are added.
fn order(shape: u8, held: &[(u8, f32, u32)], out: &mut [(u8, f32); MAX_SEQ]) -> usize {
    let mut by_pitch: [(u8, f32, u32); MAX_HELD] = [(0, 0.0, 0); MAX_HELD];
    let n = held.len().min(MAX_HELD);
    if n == 0 {
        return 0;
    }
    let held = &held[..n];
    by_pitch[..n].copy_from_slice(held);
    let sorted = &mut by_pitch[..n];
    sorted.sort_unstable_by_key(|h| (h.0, h.2));
    let mut k = 0;
    let mut put = |h: &(u8, f32, u32)| {
        if k < MAX_SEQ {
            out[k] = (h.0, h.1);
            k += 1;
        }
    };
    match shape {
        SHAPE_DOWN => sorted.iter().rev().for_each(&mut put),
        SHAPE_UPDOWN => {
            sorted.iter().for_each(&mut put);
            if n > 2 {
                sorted[1..n - 1].iter().rev().for_each(&mut put);
            }
        }
        SHAPE_UPANDDOWN => {
            sorted.iter().for_each(&mut put);
            sorted.iter().rev().for_each(&mut put);
        }
        SHAPE_CONVERGE => {
            let (mut a, mut b) = (0usize, n);
            while a < b {
                put(&sorted[a]);
                a += 1;
                if a < b {
                    b -= 1;
                    put(&sorted[b]);
                }
            }
        }
        SHAPE_DIVERGE => {
            // the middle first, then outwards, alternating up and down
            let mid = (n - 1) / 2;
            put(&sorted[mid]);
            for d in 1..n {
                let up = mid + d;
                if up < n {
                    put(&sorted[up]);
                }
                if d <= mid {
                    put(&sorted[mid - d]);
                }
            }
        }
        SHAPE_THUMB => {
            // the lowest between each of the others: C E C G C B ...
            if n == 1 {
                put(&sorted[0]);
            }
            for h in sorted.iter().skip(1) {
                put(&sorted[0]);
                put(h);
            }
        }
        SHAPE_PLAYED => {
            let mut played: [(u8, f32, u32); MAX_HELD] = [(0, 0.0, 0); MAX_HELD];
            played[..n].copy_from_slice(held);
            played[..n].sort_unstable_by_key(|h| h.2);
            played[..n].iter().for_each(&mut put);
        }
        _ => sorted.iter().for_each(&mut put), // Up (0), Chord, Random, Pattern use the notes by pitch
    }
    k
}

impl Seq {
    pub fn set_pattern(&mut self, bank: usize, vals: &[f32]) {
        if let Some(p) = self.patterns.get_mut(bank) {
            *p = default_pattern();
            let n = vals.len().min(p.len());
            p[..n].copy_from_slice(&vals[..n]);
        }
    }

    pub fn set_clip(&mut self, slot: usize, notes: &[ClipNote], length: f32) {
        if let Some(c) = self.clips.get_mut(slot) {
            let n = notes.len().min(CLIP_NOTES);
            c.notes[..n].copy_from_slice(&notes[..n]);
            c.notes[..n].sort_unstable_by(|a, b| a.start.total_cmp(&b.start));
            c.n = n;
            c.length = if length.is_finite() && length > 0.0 { length.min(4096.0) } else { 4.0 };
        }
    }

    pub fn set_lane(&mut self, slot: usize, lane: usize, param: u16, points: &[(f32, f32)]) {
        if let Some(l) = self.clips.get_mut(slot).and_then(|c| c.lanes.get_mut(lane)) {
            let n = points.len().min(CLIP_POINTS);
            l.param = param;
            l.points[..n].copy_from_slice(&points[..n]);
            l.points[..n].sort_unstable_by(|a, b| a.0.total_cmp(&b.0));
            l.n = n;
        }
    }

    /// The clock at `offset` frames into the coming sub-block.
    pub fn beat_at(&self, offset: usize) -> f64 {
        if self.spb <= 0.0 {
            return self.beat;
        }
        self.base_beat + ((self.frame + offset as u64) as i64 - self.base_frame as i64) as f64 / self.spb
    }

    /// Frames the sequencer has run (the start of the coming sub-block).
    pub fn frame(&self) -> u64 {
        self.frame
    }

    /// Follow a shared timeline: at `frame` the beat is `beat`, running at
    /// `rate` × the tempo, and the transport plays or not. Joining one that's
    /// already playing starts the clip at once, at the song position, as if it
    /// had been playing from the top.
    pub fn set_timeline(&mut self, frame: u64, beat: f64, rate: f64, playing: bool, out: &mut Events) {
        let rate = if rate.is_finite() { rate.clamp(0.5, 2.0) } else { 1.0 };
        let beat = if beat.is_finite() { beat } else { 0.0 };
        if playing && !self.playing {
            self.playing = true;
            self.clip_playing = None;
            self.pending = None;
            self.song_start = Some(beat);
            if self.running {
                // the arp's grid stays on the song's beats
                self.origin = 0.0;
                self.next = 0;
            }
        } else if !playing && self.playing {
            self.playing = false;
            self.release(out, 0, Some(Owner::Clip));
            self.clip_playing = None;
            self.pending = None;
        }
        // re-anchor (a tempo change re-anchors again at the next sub-block, from here)
        if self.spb > 0.0 {
            self.spb *= self.rate / rate;
        }
        self.rate = rate;
        self.base_frame = frame;
        self.base_beat = beat;
        self.beat = self.beat_at(0);
    }

    /// Sequencer notes sounding now.
    pub fn sounding(&self) -> usize {
        self.nledger
    }

    fn id(&mut self) -> u32 {
        let id = SEQ_ID + self.next_id;
        self.next_id = if self.next_id >= 0x3FFF_FFFE { 1 } else { self.next_id + 1 };
        id
    }

    /// Start a note (ledger it): its note-on goes out now, its note-off at `off`.
    fn start(&mut self, out: &mut Events, at: usize, note: u8, velocity: f32, off: f64, owner: Owner) {
        if self.nledger >= LEDGER {
            // full: end the note that ends soonest now, to make room
            let (i, _) = self.ledger[..self.nledger].iter().enumerate().fold((0, f64::MAX), |b, (i, s)| if s.off < b.1 { (i, s.off) } else { b });
            let s = self.ledger[i];
            out.push(at, Ev::Off { note: s.note, id: s.id });
            self.ledger[i] = self.ledger[self.nledger - 1];
            self.nledger -= 1;
        }
        let id = self.id();
        out.push(at, Ev::On { note, velocity, id });
        self.ledger[self.nledger] = Sounding { id, note, off, owner };
        self.nledger += 1;
    }

    /// End every sequencer note now (or only one owner's).
    fn release(&mut self, out: &mut Events, at: usize, owner: Option<Owner>) {
        let mut i = 0;
        while i < self.nledger {
            let s = self.ledger[i];
            if owner.is_none_or(|o| o == s.owner) {
                out.push(at, Ev::Off { note: s.note, id: s.id });
                self.ledger[i] = self.ledger[self.nledger - 1];
                self.nledger -= 1;
            } else {
                i += 1;
            }
        }
    }

    /// Start (or stop) the transport. Stopping ends every clip note.
    pub fn transport(&mut self, play: bool, out: &mut Events) {
        if play {
            self.playing = true;
            self.beat = 0.0;
            self.base_beat = 0.0;
            self.base_frame = self.frame;
            self.song_start = Some(0.0);
            self.clip_playing = None;
            self.pending = None;
            if self.running {
                // the arp's grid restarts with the transport
                self.origin = 0.0;
                self.next = 0;
            }
        } else if self.playing {
            self.playing = false;
            self.release(out, 0, Some(Owner::Clip));
            self.clip_playing = None;
            self.pending = None;
        }
    }

    /// A key went down (already mapped by the keyboard settings) at `beat`.
    pub fn key_down(&mut self, note: u8, velocity: f32, beat: f64, p: &SeqParams) {
        if self.latched {
            // a new chord after letting go replaces the held one
            self.nheld = 0;
            self.latched = false;
        }
        let first = self.nheld == 0;
        if let Some(h) = self.held[..self.nheld].iter_mut().find(|h| h.0 == note) {
            h.1 = velocity;
        } else if self.nheld < MAX_HELD {
            self.order += 1;
            self.held[self.nheld] = (note, velocity, self.order);
            self.nheld += 1;
        }
        self.seq_dirty = true;
        let restart = match p.retrigger {
            RETRIG_NOTE => true,
            RETRIG_LAUNCH => first || !self.running,
            _ => !self.running,
        };
        if restart {
            let len = ARP_RATES[(p.rate as usize).min(ARP_RATES.len() - 1)];
            if p.retrigger == RETRIG_BEAT {
                // the next point of the grid (from beat 0)
                self.origin = 0.0;
                self.next = (beat / len - 1e-9).ceil().max(0.0) as u64;
            } else {
                self.origin = beat;
                self.next = 0;
            }
            self.running = true;
        }
    }

    pub fn key_up(&mut self, note: u8, p: &SeqParams) {
        if let Some(i) = self.held[..self.nheld].iter().position(|h| h.0 == note) {
            if p.hold && self.nheld == 1 {
                // keep the chord going: it's replaced by the next one played
                self.latched = true;
                return;
            }
            self.held.copy_within(i + 1..self.nheld, i);
            self.nheld -= 1;
            self.seq_dirty = true;
        }
    }

    /// Let go of everything held (all notes off, or the arp switched off).
    pub fn clear_keys(&mut self) {
        self.nheld = 0;
        self.latched = false;
        self.seq_dirty = true;
    }

    /// Everything due in the next sub-block of `frames` frames at `sr`, into `out` (offsets in frames).
    pub fn advance(&mut self, p: &SeqParams, frames: usize, sr: f32, out: &mut Events) {
        let spb = 60.0 * sr as f64 / (p.bpm.max(1.0) as f64 * self.rate);
        let f0 = self.frame;
        if spb != self.spb {
            // a new tempo: carry on from where the clock is now
            if self.spb > 0.0 {
                self.base_beat += (f0 as i64 - self.base_frame as i64) as f64 / self.spb;
            }
            self.base_frame = f0;
            self.spb = spb;
        }
        let f1 = f0 + frames as u64;
        let clock = Clock { f0, f1, base_frame: self.base_frame, base_beat: self.base_beat, spb };
        // note-offs due
        let mut i = 0;
        while i < self.nledger {
            let s = self.ledger[i];
            if clock.frame(s.off) < f1 as i64 {
                out.push(clock.at(s.off), Ev::Off { note: s.note, id: s.id });
                self.ledger[i] = self.ledger[self.nledger - 1];
                self.nledger -= 1;
            } else {
                i += 1;
            }
        }
        // strummed notes whose frames have come
        let mut i = 0;
        while i < self.nstrum {
            let (when, note, vel, off) = self.strum[i];
            if clock.frame(when) < f1 as i64 {
                self.start(out, clock.at(when), note, vel, off, Owner::Arp);
                self.nstrum -= 1;
                self.strum[i] = self.strum[self.nstrum];
            } else {
                i += 1;
            }
        }
        self.arp(p, &clock, out);
        self.clip(p, &clock, out);
        self.frame = f1;
        self.beat = clock.beat(f1);
        out.sort();
    }

    fn arp(&mut self, p: &SeqParams, c: &Clock, out: &mut Events) {
        if !p.arp_on {
            if self.running {
                self.running = false;
                self.step = -1;
                self.release(out, 0, Some(Owner::Arp));
            }
            self.nstrum = 0;
            return;
        }
        if self.nheld == 0 {
            self.running = false;
            self.step = -1;
            self.nstrum = 0;
            return;
        }
        if self.seq_dirty {
            self.seq_len = order(p.shape, &self.held[..self.nheld], &mut self.seq);
            self.seq_dirty = false;
        }
        let len = ARP_RATES[(p.rate as usize).min(ARP_RATES.len() - 1)];
        let steps = (p.steps as u64).clamp(1, ARP_STEPS as u64);
        let pat = self.patterns[(p.bank as usize).clamp(1, ARP_BANKS) - 1];
        let lane = |l: usize, s: usize| pat[l * ARP_STEPS + s];
        let reps = (p.repeats as u64).max(1);
        let octaves = (p.octaves as u64).clamp(1, 4);
        let cycle = self.seq_len as u64 * octaves;
        loop {
            let k = self.next;
            let swing = if k % 2 == 1 { p.swing as f64 * 0.5 } else { 0.0 };
            let beat = self.origin + (k as f64 + p.offset as f64 + swing) * len;
            let f = c.frame(beat);
            if f >= c.f1 as i64 {
                break;
            }
            self.next += 1;
            if f < c.f0 as i64 {
                continue;
            }
            let pos = (k % steps) as usize;
            self.step = pos as i32;
            if lane(L_ON, pos) < 0.5 || cycle == 0 {
                continue;
            }
            if self.rng.next_f32() >= (p.chance * lane(L_CHANCE, pos)).clamp(0.0, 1.0) {
                continue;
            }
            // which note (and octave) this step plays
            let n = k / reps;
            let (idx, oct) = match p.shape {
                SHAPE_RANDOM => ((self.rng.next_u32() as u64) % self.seq_len as u64, (self.rng.next_u32() as u64) % octaves),
                SHAPE_PATTERN => ((lane(L_DEGREE, pos).max(0.0) as u64) % self.seq_len as u64, (n / self.seq_len as u64) % octaves),
                SHAPE_CHORD => (0, n % octaves),
                _ => ((n % cycle) % self.seq_len as u64, (n % cycle) / self.seq_len as u64),
            };
            let ramp = if cycle > 1 { ((n % cycle) as f32 / (cycle - 1) as f32) * 2.0 - 1.0 } else { 0.0 };
            let vel_scale = (lane(L_VEL, pos) * (1.0 + p.vel_ramp * ramp * 0.5)).clamp(0.02, 1.0);
            let off = beat + len * (p.gate as f64) * (lane(L_GATE, pos) as f64 * 2.0);
            let shift = oct as i32 * p.shift as i32 + lane(L_BEND, pos).round() as i32;
            let key = |note: u8| (note as i32 + shift).clamp(0, 127) as u8;
            if p.shape == SHAPE_CHORD {
                let strum = lane(L_STRUM, pos) as f64 * len;
                let n = self.seq_len;
                for j in 0..n {
                    let (note, vel) = self.seq[j];
                    let when = beat + strum * j as f64 / n as f64;
                    let (note, vel, end) = (key(note), vel * vel_scale, off.max(when + 1e-6));
                    if c.frame(when) >= c.f1 as i64 && self.nstrum < STRUM_QUEUE {
                        // later than this sub-block: it waits for its frame
                        self.strum[self.nstrum] = (when, note, vel, end);
                        self.nstrum += 1;
                    } else {
                        self.start(out, c.at(when), note, vel, end, Owner::Arp);
                    }
                }
            } else {
                let (note, vel) = self.seq[idx as usize];
                self.start(out, c.at(beat), key(note), vel * vel_scale, off, Owner::Arp);
            }
        }
    }

    fn clip(&mut self, p: &SeqParams, c: &Clock, out: &mut Events) {
        let (b0, b1) = (c.beat(c.f0), c.beat(c.f1));
        if !self.playing || !p.clip_on {
            if self.clip_playing.is_some() {
                self.release(out, 0, Some(Owner::Clip));
                self.clip_playing = None;
            }
            self.pending = None;
            // a clip switched on later waits for the launch quantize as usual
            self.song_start = None;
            return;
        }
        let want = (p.clip_slot as usize).clamp(1, CLIP_SLOTS) - 1;
        let song_start = self.song_start.take();
        if self.clip_playing != Some(want) && self.pending.is_none_or(|(s, _)| s != want) {
            let q = QUANTIZE[(p.quantize as usize).min(QUANTIZE.len() - 1)];
            let start = match song_start {
                Some(beat) => beat,
                None if q == 0.0 => b0,
                None => (b0 / q - 1e-9).ceil() * q,
            };
            self.pending = Some((want, start));
        }
        if let Some((s, start)) = self.pending
            && c.frame(start) < c.f1 as i64
        {
            // the old clip's notes end where the new one starts
            if self.clip_playing.is_some() {
                self.release(out, c.at(start), Some(Owner::Clip));
            }
            self.clip_playing = Some(s);
            self.clip_start = start;
            self.play_from = start;
            if song_start.is_some() {
                // with the transport: where it would be had it played from the top (the song's
                // beat 0), so instances that join mid-way play in step
                let len = (self.clips[s].length as f64).max(1e-3);
                self.clip_start = (start / len + 1e-9).floor() * len;
            }
            self.pending = None;
        }
        let Some(slot) = self.clip_playing else { return };
        let clip = &self.clips[slot];
        let length = clip.length as f64;
        // this sub-block, in the clip's own time (maybe across its end), a hair
        // wider in beats (and earlier by the most swing can delay a note): each
        // note is then kept if its frame falls in the block
        let eps = 0.5 / c.spb;
        let swing = p.swing.clamp(0.0, 1.0) as f64 * SWING_MAX;
        let (p0, p1) = ((b0 - eps - swing).max(self.clip_start).max(self.play_from) - self.clip_start, b1 + eps - self.clip_start);
        if p1 <= 0.0 {
            return;
        }
        let mut starts: [(f64, usize); 128] = [(0.0, 0); 128];
        let mut ns = 0;
        let mut add_range = |a: f64, b: f64, base: f64, clip: &Clip| {
            // notes whose start lies in [a, b) of one pass through the clip
            let lo = clip.notes[..clip.n].partition_point(|n| (n.start as f64) < a);
            for (i, n) in clip.notes[lo..clip.n].iter().enumerate() {
                if n.start as f64 >= b || ns >= starts.len() {
                    break;
                }
                starts[ns] = (base + n.start as f64, lo + i);
                ns += 1;
            }
        };
        if p.clip_loop {
            let pass0 = (p0 / length).floor();
            let pass1 = (p1 / length).floor();
            let mut pass = pass0;
            while pass <= pass1 {
                let a = (p0 - pass * length).max(0.0);
                let b = (p1 - pass * length).min(length);
                if b > a {
                    add_range(a, b, self.clip_start + pass * length, clip);
                }
                pass += 1.0;
            }
        } else if p0 < length {
            add_range(p0, p1.min(length), self.clip_start, clip);
        }
        for &(beat, i) in &starts[..ns] {
            let beat = swung(beat - self.clip_start, swing) + self.clip_start;
            let f = c.frame(beat);
            if f < c.f0 as i64 || f >= c.f1 as i64 {
                continue;
            }
            let n = self.clips[slot].notes[i];
            if n.chance < 1.0 && self.rng.next_f32() >= n.chance {
                continue;
            }
            let key = (n.key as i32 + p.clip_transpose as i32 + n.bend.round() as i32).clamp(0, 127) as u8;
            self.start(out, c.at(beat), key, n.velocity, beat + (n.len.max(0.001) as f64), Owner::Clip);
        }
    }

    /// The playing clip's automation: (param, normalized value) for each lane now.
    pub fn automation(&self, out: &mut [(u16, f32); CLIP_LANES]) -> usize {
        let Some(slot) = self.clip_playing else { return 0 };
        let clip = &self.clips[slot];
        let mut pos = (self.beat - self.clip_start).max(0.0);
        if clip.length > 0.0 {
            pos %= clip.length as f64;
        }
        let mut n = 0;
        for l in &clip.lanes {
            if let Some(v) = l.at(pos as f32) {
                out[n] = (l.param, v.clamp(0.0, 1.0));
                n += 1;
            }
        }
        n
    }

    /// Where the playing clip is, in beats.
    pub fn clip_pos(&self) -> f64 {
        match self.clip_playing {
            Some(s) => {
                let len = self.clips[s].length.max(1e-3) as f64;
                (self.beat - self.clip_start).max(0.0) % len
            }
            None => 0.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> SeqParams {
        SeqParams { bpm: 120.0, arp_on: true, rate: 4, octaves: 1, shift: 12, gate: 0.5, chance: 1.0, repeats: 1, retrigger: RETRIG_LAUNCH, steps: 16, bank: 1, clip_loop: true, clip_slot: 1, quantize: 3, ..SeqParams::default() }
    }

    /// Run for `frames` (16 at a time); the note-ons with their absolute frames.
    fn run(s: &mut Seq, p: &SeqParams, frames: usize) -> Vec<(usize, u8)> {
        let mut t = 0;
        let mut ons = Vec::new();
        while t < frames {
            let mut ev = Events::default();
            s.advance(p, 16, 48_000.0, &mut ev);
            for &(at, e) in ev.as_slice() {
                if let Ev::On { note, .. } = e {
                    ons.push((t + at, note));
                }
            }
            t += 16;
        }
        ons
    }

    fn order_of(shape: u8, notes: &[u8]) -> Vec<u8> {
        let held: Vec<(u8, f32, u32)> = notes.iter().enumerate().map(|(i, &n)| (n, 1.0, i as u32)).collect();
        let mut out = [(0u8, 0.0f32); MAX_SEQ];
        let n = order(shape, &held, &mut out);
        out[..n].iter().map(|x| x.0).collect()
    }

    #[test]
    fn shapes_play_in_their_order() {
        let notes = [64, 60, 67, 72]; // played in this order: E C G C'
        assert_eq!(order_of(0, &notes), vec![60, 64, 67, 72]);
        assert_eq!(order_of(SHAPE_DOWN, &notes), vec![72, 67, 64, 60]);
        assert_eq!(order_of(SHAPE_UPDOWN, &notes), vec![60, 64, 67, 72, 67, 64]);
        assert_eq!(order_of(SHAPE_UPANDDOWN, &notes), vec![60, 64, 67, 72, 72, 67, 64, 60]);
        assert_eq!(order_of(SHAPE_CONVERGE, &notes), vec![60, 72, 64, 67]);
        assert_eq!(order_of(SHAPE_DIVERGE, &notes), vec![64, 67, 60, 72]);
        assert_eq!(order_of(SHAPE_THUMB, &notes), vec![60, 64, 60, 67, 60, 72]);
        assert_eq!(order_of(SHAPE_PLAYED, &notes), vec![64, 60, 67, 72]);
        assert_eq!(order_of(SHAPE_DIVERGE, &[60, 62, 64, 65, 67]), vec![64, 65, 62, 67, 60]);
        assert_eq!(order_of(SHAPE_UPDOWN, &[60, 64]), vec![60, 64]);
    }

    #[test]
    fn sixteenths_at_120_land_every_6000_frames() {
        let mut s = Seq::default();
        let p = params();
        s.key_down(60, 1.0, 0.0, &p);
        s.key_down(64, 1.0, 0.0, &p);
        let ons = run(&mut s, &p, 48_000 * 4);
        assert_eq!(ons.len(), 32);
        for (i, &(f, note)) in ons.iter().enumerate() {
            assert_eq!(f, i * 6000, "step {i}");
            assert_eq!(note, [60, 64][i % 2]);
        }
    }

    #[test]
    fn octaves_repeats_gate_and_hold() {
        let mut s = Seq::default();
        let p = SeqParams { octaves: 2, repeats: 2, shift: 7, ..params() };
        s.key_down(60, 1.0, 0.0, &p);
        s.key_down(62, 1.0, 0.0, &p);
        let ons: Vec<u8> = run(&mut s, &p, 6000 * 8).into_iter().map(|x| x.1).collect();
        assert_eq!(ons, vec![60, 60, 62, 62, 67, 67, 69, 69]);
        // hold: letting go keeps it going; the next key replaces the chord
        let mut s = Seq::default();
        let p = SeqParams { hold: true, ..params() };
        s.key_down(60, 1.0, 0.0, &p);
        s.key_up(60, &p);
        assert_eq!(run(&mut s, &p, 6000 * 3).len(), 3);
        s.key_down(70, 1.0, s.beat, &p);
        let after: Vec<u8> = run(&mut s, &p, 6000 * 2).into_iter().map(|x| x.1).collect();
        assert!(after.iter().all(|&n| n == 70), "{after:?}");
    }

    #[test]
    fn clips_play_in_time_and_loop() {
        let mut s = Seq::default();
        let p = SeqParams { arp_on: false, clip_on: true, ..params() };
        s.set_clip(0, &[ClipNote { start: 0.0, len: 0.5, key: 60, velocity: 1.0, chance: 1.0, bend: 0.0 }, ClipNote { start: 1.5, len: 0.5, key: 67, velocity: 1.0, chance: 1.0, bend: 0.0 }], 2.0);
        let mut ev = Events::default();
        s.transport(true, &mut ev);
        let ons = run(&mut s, &p, 48_000 * 4); // 8 beats: four passes
        let want: Vec<(usize, u8)> = (0..4).flat_map(|k| [(k * 48_000, 60u8), (k * 48_000 + 36_000, 67u8)]).collect();
        assert_eq!(ons, want);
        // stopping ends everything the clip started
        let mut ev = Events::default();
        s.transport(false, &mut ev);
        assert_eq!(s.sounding(), 0);
    }

    #[test]
    fn a_strummed_chord_spreads_over_the_step() {
        let mut s = Seq::default();
        let p = SeqParams { shape: SHAPE_CHORD, ..params() };
        for st in 0..ARP_STEPS {
            s.patterns[0][L_STRUM * ARP_STEPS + st] = 0.5;
        }
        for n in [60, 64, 67] {
            s.key_down(n, 1.0, 0.0, &p);
        }
        // half of a 6,000-frame step, in three: 0, 1,000 and 2,000 frames in
        let ons = run(&mut s, &p, 12_000);
        assert_eq!(ons, vec![(0, 60), (1_000, 64), (2_000, 67), (6_000, 60), (7_000, 64), (8_000, 67)]);
    }

    #[test]
    fn swing_delays_the_off_beat_sixteenths() {
        let mut s = Seq::default();
        let p = SeqParams { arp_on: false, clip_on: true, swing: 1.0, ..params() };
        let notes: Vec<ClipNote> = (0..4).map(|k| ClipNote { start: k as f32 * 0.25, len: 0.1, key: 60 + k as u8, velocity: 1.0, chance: 1.0, bend: 0.0 }).collect();
        s.set_clip(0, &notes, 1.0);
        let mut ev = Events::default();
        s.transport(true, &mut ev);
        // a sixteenth is 6,000 frames at 120; full swing moves the second and fourth by 3,000
        assert_eq!(run(&mut s, &p, 24_000), vec![(0, 60), (9_000, 61), (12_000, 62), (21_000, 63)]);
        assert_eq!(swung(0.3, 0.0), 0.3);
        assert!((swung(0.125, 0.125) - 0.1875).abs() < 1e-12 && (swung(0.49, 0.125) - (0.375 + 0.24 * 0.5)).abs() < 1e-12);
    }

    #[test]
    fn switching_clips_waits_for_the_bar() {
        let mut s = Seq::default();
        let mut p = SeqParams { arp_on: false, clip_on: true, ..params() };
        s.set_clip(0, &[ClipNote { start: 0.0, len: 4.0, key: 60, velocity: 1.0, chance: 1.0, bend: 0.0 }], 4.0);
        s.set_clip(1, &[ClipNote { start: 0.0, len: 1.0, key: 72, velocity: 1.0, chance: 1.0, bend: 0.0 }], 4.0);
        let mut ev = Events::default();
        s.transport(true, &mut ev);
        run(&mut s, &p, 24_000); // one beat in
        p.clip_slot = 2;
        let ons = run(&mut s, &p, 96_000);
        // the next bar starts at beat 4 = frame 96,000 from the start, 72,000 after this point
        assert_eq!(ons.first(), Some(&(72_000, 72)));
    }
}
