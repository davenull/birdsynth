//! Building blocks shared by the effects: delay lines, modulation LFOs,
//! smoothing one-poles and a DC blocker.

use wt_dsp::math::flush;

use crate::lfo::SYNC_BEATS;
use crate::spec::protocol::SUB_BLOCK;

pub const N: usize = SUB_BLOCK;
/// One sub-block of stereo audio.
pub type Block = [[f32; N]; 2];

/// Note values of the delay's sync options, in beats (params/fx/delay.toml order).
pub const NOTE_BEATS: [f64; 16] = [0.0625, 0.125, 1.0 / 6.0, 0.25, 0.375, 1.0 / 3.0, 0.5, 0.75, 2.0 / 3.0, 1.0, 1.5, 4.0 / 3.0, 2.0, 3.0, 4.0, 8.0];

#[inline]
pub fn db(x: f32) -> f32 {
    10f32.powf(x * 0.05)
}

/// One-pole smoothing coefficient for a cutoff.
#[inline]
pub fn onepole_coef(fc: f32, sr: f32) -> f32 {
    1.0 - (-std::f32::consts::TAU * fc / sr).exp()
}

/// A circular delay line with fractional reads. `read(d)` returns the sample
/// written `d` samples before the newest one (so writing x then reading d
/// delays x by exactly d samples).
pub struct DelayLine {
    buf: Box<[f32]>,
    pos: usize,
}

impl DelayLine {
    pub fn new(len: usize) -> DelayLine {
        DelayLine { buf: vec![0.0; len.max(4)].into_boxed_slice(), pos: 0 }
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    #[inline(always)]
    pub fn write(&mut self, x: f32) {
        self.pos += 1;
        if self.pos == self.buf.len() {
            self.pos = 0;
        }
        self.buf[self.pos] = flush(x);
    }

    #[inline(always)]
    fn at(&self, back: usize) -> f32 {
        let n = self.buf.len();
        let i = if back <= self.pos { self.pos - back } else { self.pos + n - back };
        self.buf[i]
    }

    /// Linear interpolation, d in [0, len - 2].
    #[inline(always)]
    pub fn read(&self, d: f32) -> f32 {
        let d = d.clamp(0.0, (self.buf.len() - 2) as f32);
        let i = d as usize;
        let t = d - i as f32;
        let a = self.at(i);
        a + (self.at(i + 1) - a) * t
    }

    /// Cubic (Hermite) interpolation, d in [1, len - 3]: cleaner while the delay moves.
    #[inline(always)]
    pub fn read_cubic(&self, d: f32) -> f32 {
        let d = d.clamp(1.0, (self.buf.len() - 3) as f32);
        let i = d as usize;
        let t = d - i as f32;
        let (y0, y1, y2, y3) = (self.at(i - 1), self.at(i), self.at(i + 1), self.at(i + 2));
        let c1 = 0.5 * (y2 - y0);
        let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
        ((c3 * t + c2) * t + c1) * t + y1
    }

    pub fn clear(&mut self) {
        self.buf.fill(0.0);
        self.pos = 0;
    }

    pub fn has_subnormal(&self) -> bool {
        self.buf.iter().any(|&v| v != 0.0 && v.abs() < f32::MIN_POSITIVE)
    }
}

/// An effect's modulation LFO: free-running at a rate, or locked to the song
/// position when synced. Phases are in cycles.
#[derive(Clone, Copy, Debug, Default)]
pub struct ModLfo {
    phase: f64,
}

impl ModLfo {
    /// Move on by one sub-block. Returns the phase at the block's start and end.
    pub fn advance(&mut self, hz: f64, sync_beats: Option<f64>, beat: f64, dt: f64) -> (f64, f64) {
        let p0 = self.phase;
        match sync_beats {
            Some(b) => {
                let a = beat / b;
                let e = (beat + dt * hz * b) / b;
                self.phase = e;
                (a, e)
            }
            None => {
                self.phase = p0 + hz * dt;
                if self.phase > 1e6 {
                    self.phase -= self.phase.floor();
                }
                (p0, self.phase)
            }
        }
    }

    pub fn reset(&mut self) {
        self.phase = 0.0;
    }
}

/// Rate in Hz and, when synced, beats per cycle, for an effect's rate/bpm/sync parameters.
pub fn lfo_rate(rate: f32, bpm_on: bool, sync: f32, bpm: f32) -> (f64, Option<f64>) {
    if bpm_on {
        let beats = SYNC_BEATS[(sync as usize).min(SYNC_BEATS.len() - 1)];
        (bpm as f64 / 60.0 / beats, Some(beats))
    } else {
        (rate as f64, None)
    }
}

/// A one-pole low-pass.
#[derive(Clone, Copy, Debug, Default)]
pub struct Lp1 {
    pub z: f32,
}

impl Lp1 {
    #[inline(always)]
    pub fn tick(&mut self, x: f32, a: f32) -> f32 {
        self.z += (x - self.z) * a;
        self.z
    }

    pub fn flush(&mut self) {
        self.z = flush(self.z);
    }
}

/// Removes DC (a one-pole high-pass at a few Hz).
#[derive(Clone, Copy, Debug, Default)]
pub struct DcBlock {
    x1: f32,
    y1: f32,
}

impl DcBlock {
    #[inline(always)]
    pub fn tick(&mut self, x: f32, r: f32) -> f32 {
        let y = x - self.x1 + r * self.y1;
        self.x1 = x;
        self.y1 = y;
        y
    }

    pub fn flush(&mut self) {
        self.x1 = flush(self.x1);
        self.y1 = flush(self.y1);
    }
}

/// The DC blocker's pole for a cutoff.
#[inline]
pub fn dc_pole(fc: f32, sr: f32) -> f32 {
    1.0 - std::f32::consts::TAU * fc / sr
}

/// A light TPT state-variable filter (no delay memory, unlike the voice filter).
#[derive(Clone, Copy, Debug, Default)]
pub struct Svf2 {
    ic1: f32,
    ic2: f32,
}

impl Svf2 {
    /// One sample at prewarped gain `g` and damping `k`. Returns (low, band, high).
    #[inline(always)]
    pub fn tick(&mut self, x: f32, g: f32, k: f32) -> (f32, f32, f32) {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - k * v1 - v2)
    }

    pub fn flush(&mut self) {
        self.ic1 = flush(self.ic1);
        self.ic2 = flush(self.ic2);
    }
}

/// A Linkwitz-Riley 4th-order crossover for one channel: the low and high
/// outputs add back up to an allpass (flat magnitude).
#[derive(Clone, Copy, Debug, Default)]
pub struct Lr4 {
    lp: [Svf2; 2],
    hp: [Svf2; 2],
}

impl Lr4 {
    #[inline(always)]
    pub fn split(&mut self, x: f32, g: f32) -> (f32, f32) {
        let k = std::f32::consts::SQRT_2;
        let l1 = self.lp[0].tick(x, g, k).0;
        let l = self.lp[1].tick(l1, g, k).0;
        let h1 = self.hp[0].tick(x, g, k).2;
        let h = self.hp[1].tick(h1, g, k).2;
        (l, h)
    }

    pub fn flush(&mut self) {
        for s in self.lp.iter_mut().chain(self.hp.iter_mut()) {
            s.flush();
        }
    }
}
