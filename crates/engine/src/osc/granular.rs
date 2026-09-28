//! Granular: short windowed grains read from a recording, started at a
//! density (or at the note's frequency, with Pitch Sync), scattered around a
//! position that scans through the recording.
//!
//! Every voice's grains live in one engine-wide pool of MAX_GRAINS; when it's
//! full the oldest grain is stolen, so the cost has a ceiling however dense
//! the settings get. Windows come from tables made once.

use wt_dsp::math;
use wt_dsp::rng::Rng;

use super::sample::Recording;
use crate::filter::MAX_N;
use crate::spec::protocol::MAX_GRAINS;

pub const WIN_HANN: u8 = 0;
pub const WIN_TRIANGLE: u8 = 1;
pub const WIN_TUKEY: u8 = 2;
pub const WIN_RECT: u8 = 3;
pub const WIN_DECAY: u8 = 4;
pub const WIN_SWELL: u8 = 5;
pub const WINDOWS: usize = 6;

pub const DIR_FORWARD: u8 = 0;
pub const DIR_REVERSE: u8 = 1;
pub const DIR_RANDOM: u8 = 2;

const WIN_LEN: usize = 1024;

#[derive(Clone, Copy, Debug, Default)]
pub struct Grain {
    /// Who plays it (see `owner`); 0 is a free slot.
    pub owner: u16,
    pos: f64,
    step: f64,
    age: u32,
    len: u32,
    /// Samples of the current block to wait before it starts.
    delay: u32,
    gl: f32,
    gr: f32,
    window: u8,
    serial: u64,
}

/// The pool key for voice slot `v`, oscillator `o`.
pub fn owner(v: usize, o: usize) -> u16 {
    (v * 4 + o + 1) as u16
}

pub struct GrainPool {
    grains: Box<[Grain]>,
    windows: Box<[[f32; WIN_LEN + 1]]>,
    serial: u64,
    /// Grains cut short because the pool was full.
    pub stolen: u64,
    /// Grains started (a running count, for tests and stats).
    pub started: u64,
}

impl Default for GrainPool {
    fn default() -> Self {
        let mut windows = vec![[0.0f32; WIN_LEN + 1]; WINDOWS].into_boxed_slice();
        for (w, t) in windows.iter_mut().enumerate() {
            for (i, v) in t.iter_mut().enumerate() {
                *v = window(w as u8, i as f32 / WIN_LEN as f32);
            }
        }
        GrainPool { grains: vec![Grain::default(); MAX_GRAINS].into_boxed_slice(), windows, serial: 0, stolen: 0, started: 0 }
    }
}

/// A window's value at `t` in 0..1.
pub fn window(kind: u8, t: f32) -> f32 {
    use std::f32::consts::TAU;
    match kind {
        WIN_TRIANGLE => 1.0 - (2.0 * t - 1.0).abs(),
        WIN_TUKEY => {
            // flat in the middle half, cosine tapers at the ends
            let e = 0.25;
            if t < e {
                0.5 - 0.5 * (std::f32::consts::PI * t / e).cos()
            } else if t > 1.0 - e {
                0.5 - 0.5 * (std::f32::consts::PI * (1.0 - t) / e).cos()
            } else {
                1.0
            }
        }
        WIN_RECT => 1.0,
        WIN_DECAY => (1.0 - (-t * 200.0).exp()) * (-4.0 * t).exp(),
        WIN_SWELL => (1.0 - (-(1.0 - t) * 200.0).exp()) * (-4.0 * (1.0 - t)).exp(),
        _ => 0.5 - 0.5 * (TAU * t).cos(),
    }
}

impl GrainPool {
    pub fn active(&self) -> usize {
        self.grains.iter().filter(|g| g.owner != 0).count()
    }

    fn spawn(&mut self, g: Grain) {
        self.serial += 1;
        self.started += 1;
        let g = Grain { serial: self.serial, ..g };
        if let Some(slot) = self.grains.iter_mut().find(|s| s.owner == 0) {
            *slot = g;
            return;
        }
        // full: the oldest grain goes
        let old = self.grains.iter_mut().min_by_key(|s| s.serial).unwrap();
        *old = g;
        self.stolen += 1;
    }

    /// End every grain a voice's oscillator has going (the voice stopped).
    pub fn free(&mut self, owner: u16) {
        for g in self.grains.iter_mut().filter(|g| g.owner == owner) {
            g.owner = 0;
        }
    }

    /// Add an owner's grains into `out` over `start..len`.
    fn render(&mut self, owner: u16, rec: &Recording, gain: f32, start: usize, len: usize, out: &mut [[f32; MAX_N]; 2]) {
        let windows = &self.windows;
        for g in self.grains.iter_mut().filter(|g| g.owner == owner) {
            let from = start + g.delay as usize;
            g.delay = 0;
            if from >= len {
                g.delay = (from - len) as u32;
                continue;
            }
            let (lvl, scale) = rec.level_for(g.step);
            let win = &windows[g.window as usize];
            let wstep = WIN_LEN as f32 / g.len as f32;
            for i in from..len {
                if g.age >= g.len {
                    g.owner = 0;
                    break;
                }
                let wpos = g.age as f32 * wstep;
                let wi = wpos as usize;
                let w = win[wi] + (win[wi + 1] - win[wi]) * (wpos - wi as f32);
                let (a, b) = rec.read(lvl, scale, g.pos);
                out[0][i] += a * w * g.gl * gain;
                out[1][i] += b * w * g.gr * gain;
                g.pos += g.step;
                g.age += 1;
            }
            if g.age >= g.len {
                g.owner = 0;
            }
        }
    }
}

/// Resolved settings for one sub-block.
#[derive(Clone, Copy, Debug)]
pub struct GrainSettings {
    pub density: f32,
    pub length_ms: f32,
    pub position: f32,
    pub scan: f32,
    pub spray: f32,
    pub pitch_rand: f32,
    pub pan: f32,
    pub window: u8,
    pub direction: u8,
    pub sync: bool,
    pub timbre: f32,
    pub looped: bool,
    /// Source frames per output sample for the note (pitch, rate and sample rates).
    pub step: f64,
    /// The note's frequency (Pitch Sync starts a grain every cycle).
    pub hz: f32,
    /// Where grains may come from, in frames (the Sample start and end).
    pub region: (f64, f64),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct GranularVoice {
    /// Samples until the next grain.
    next: f64,
    /// The scanning position, in frames.
    pub pos: f64,
    rng: u32,
    started: bool,
}

impl GranularVoice {
    pub fn start(&mut self, seed: u32) {
        *self = GranularVoice { rng: seed | 1, ..GranularVoice::default() };
    }

    /// Start this sub-block's grains and render the owner's grains into `out` (which it overwrites).
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, owner: u16, pool: &mut GrainPool, rec: &Recording, s: &GrainSettings, sr: f32, start: usize, len: usize, out: &mut [[f32; MAX_N]; 2]) {
        out[0][start..len].fill(0.0);
        out[1][start..len].fill(0.0);
        let (lo, hi) = s.region;
        let span = (hi - lo).max(2.0);
        if !self.started {
            self.started = true;
            self.pos = lo + s.position.clamp(0.0, 1.0) as f64 * span;
        }
        let mut rng = Rng::new(self.rng);
        let interval = if s.sync { sr as f64 / s.hz.max(1.0) as f64 } else { sr as f64 / s.density.max(0.01) as f64 };
        let glen = ((s.length_ms.max(1.0) * 0.001 * sr) as u32).max(8);
        // grains overlap about rate × length deep: keep the level steady as that changes
        let overlap = glen as f64 / interval;
        let gain = 1.0 / (overlap.max(1.0) as f32).sqrt();
        // with Scan the position moves at `scan` × the recording's own speed
        let pos_step = s.scan as f64 * rec.rate as f64 / sr as f64;
        let n = len - start;
        let timbre = math::semis_to_ratio(s.timbre) as f64;
        let base = if s.sync { rec.rate as f64 / sr as f64 } else { s.step };
        for i in 0..n {
            self.next -= 1.0;
            if self.next <= 0.0 {
                self.next += interval;
                let spray = s.spray as f64 * (rng.next_f32() as f64 * 2.0 - 1.0) * span;
                let mut p = self.pos + spray;
                p = if s.looped { lo + (p - lo).rem_euclid(span) } else { p.clamp(lo, hi - 1.0) };
                let detune = if s.pitch_rand > 0.0 { math::semis_to_ratio(s.pitch_rand * (rng.next_f32() * 2.0 - 1.0)) as f64 } else { 1.0 };
                let reverse = match s.direction {
                    DIR_REVERSE => true,
                    DIR_RANDOM => rng.next_f32() < 0.5,
                    _ => false,
                };
                let step = base * timbre * detune * if reverse { -1.0 } else { 1.0 };
                let pan = s.pan * (rng.next_f32() * 2.0 - 1.0);
                let (gl, gr) = math::balance((pan + 1.0) * 0.5);
                if s.looped || self.pos < hi {
                    pool.spawn(Grain { owner, pos: p, step, age: 0, len: glen, delay: i as u32, gl, gr, window: s.window, serial: 0 });
                }
            }
            self.pos += pos_step;
        }
        if s.looped {
            self.pos = lo + (self.pos - lo).rem_euclid(span);
        } else {
            self.pos = self.pos.clamp(lo, hi);
        }
        self.rng = rng.next_u32() | 1;
        pool.render(owner, rec, gain, start, len, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::osc::sample::tests_support::rec;

    fn settings() -> GrainSettings {
        GrainSettings {
            density: 50.0,
            length_ms: 80.0,
            position: 0.5,
            scan: 0.0,
            spray: 0.1,
            pitch_rand: 0.0,
            pan: 0.5,
            window: WIN_HANN,
            direction: DIR_FORWARD,
            sync: false,
            timbre: 0.0,
            looped: true,
            step: 1.0,
            hz: 220.0,
            region: (0.0, 48_000.0),
        }
    }

    #[test]
    fn grain_statistics_match_the_settings() {
        let x: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.05).sin()).collect();
        let r = rec(&x, 48_000.0);
        let mut pool = GrainPool::default();
        let mut v = GranularVoice::default();
        v.start(7);
        let s = settings();
        let mut out = [[0.0f32; MAX_N]; 2];
        let blocks = 48_000 * 10 / 64;
        let mut peak_active = 0;
        let mut starts: Vec<f64> = Vec::new();
        for _ in 0..blocks {
            let before = pool.started;
            v.render(owner(0, 0), &mut pool, &r, &s, 48_000.0, 0, 64, &mut out);
            // this block's new grains, back to where they started (they play at step 1)
            let newest = pool.serial;
            for g in pool.grains.iter().filter(|g| g.owner != 0 && g.serial > newest - (pool.started - before)) {
                starts.push(g.pos - g.age as f64);
            }
            peak_active = peak_active.max(pool.active());
        }
        // 10 s at 50 per second
        let n = pool.started as f64;
        assert!((n / 500.0 - 1.0).abs() < 0.05, "{n} grains");
        // each lasts 80 ms: 4 overlap at once on average
        assert!((3..=6).contains(&peak_active), "{peak_active} at once");
        // spray: starts spread uniformly ±10% of the recording around the middle
        let mean = starts.iter().sum::<f64>() / starts.len() as f64;
        let sd = (starts.iter().map(|p| (p - mean).powi(2)).sum::<f64>() / starts.len() as f64).sqrt();
        let want_sd = 0.1 * 48_000.0 / 3f64.sqrt();
        assert!((mean / 24_000.0 - 1.0).abs() < 0.05, "mean start {mean}");
        assert!((sd / want_sd - 1.0).abs() < 0.05, "spread {sd} vs {want_sd}");
    }

    #[test]
    fn the_pool_never_holds_more_than_its_budget() {
        let x = vec![0.5f32; 48_000];
        let r = rec(&x, 48_000.0);
        let mut pool = GrainPool::default();
        let mut voices = [GranularVoice::default(); 8];
        for (i, v) in voices.iter_mut().enumerate() {
            v.start(i as u32 + 1);
        }
        let s = GrainSettings { density: 200.0, length_ms: 1000.0, ..settings() };
        let mut out = [[0.0f32; MAX_N]; 2];
        for _ in 0..2000 {
            for (i, v) in voices.iter_mut().enumerate() {
                v.render(owner(i, 0), &mut pool, &r, &s, 48_000.0, 0, 64, &mut out);
            }
            assert!(pool.active() <= MAX_GRAINS);
        }
        assert_eq!(pool.active(), MAX_GRAINS);
        assert!(pool.stolen > 0);
        pool.free(owner(3, 0));
        assert!(pool.active() < MAX_GRAINS);
        // pitch-synchronous: one grain per cycle of the note
        let mut pool = GrainPool::default();
        let mut v = GranularVoice::default();
        v.start(1);
        let s = GrainSettings { sync: true, hz: 300.0, length_ms: 5.0, ..settings() };
        for _ in 0..750 {
            v.render(owner(0, 0), &mut pool, &r, &s, 48_000.0, 0, 64, &mut out);
        }
        assert!((pool.started as i64 - 300).abs() <= 1, "{}", pool.started);
    }
}
