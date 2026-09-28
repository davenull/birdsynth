//! Spectral: a recording rebuilt from its spectrum (a phase vocoder), so
//! its speed and its pitch move independently.
//!
//! The tools analyse the recording once: Hann-windowed frames of SIZE
//! samples every HOP, centred on multiples of HOP, starting PRE frames
//! before time 0, each stored as magnitudes, phases and a transient flag.
//! A voice builds one output frame every HOP samples: it reads the analysis
//! where its (scanning) position is, works out each partial's true frequency
//! from the phase change between neighbouring frames, moves it to the
//! pitch it should have (and through the warp), advances its phase, then
//! inverse-transforms, windows and overlap-adds. At a transient the phases
//! are reset to the analysis ones, which keeps attacks sharp.
//!
//! Zero latency: when a note starts, the voice builds the frames that
//! overlap its first hop straight away (the ones centred before time 0 use
//! the PRE frames), so the first output sample is already right.

use std::f32::consts::{PI, TAU};

use wt_dsp::fft::{Complex, RealFft};

use crate::filter::MAX_N;
use crate::spec::protocol::{SPEC_FILTER_POINTS, SPEC_HOP, SPEC_PRE, SPEC_SIZE};
use crate::tables::AssetBuf;

pub const N: usize = SPEC_SIZE;
pub const HOP: usize = SPEC_HOP;
pub const BINS: usize = N / 2 + 1;
/// Floats per analysis frame: magnitudes, phases, the transient flag.
pub const FRAME_FLOATS: usize = 2 * BINS + 1;

pub const W_OFF: u8 = 0;
pub const W_SMEAR: u8 = 1;
pub const W_SPREAD: u8 = 2;
pub const W_DETUNE_UP: u8 = 3;
pub const W_DETUNE_DOWN: u8 = 4;
pub const W_BEND_UP: u8 = 5;
pub const W_BEND_DOWN: u8 = 6;
pub const W_HARMONICS: u8 = 7;
pub const W_SUBHARMONICS: u8 = 8;
pub const W_COMB: u8 = 9;
pub const W_PITCH: u8 = 10;
pub const W_BLEND: u8 = 11;

pub struct Analysis {
    buf: AssetBuf,
    pub frames: usize,
    pub rate: f32,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BadAnalysis;

impl Analysis {
    pub fn new(buf: AssetBuf, frames: usize, rate: f32) -> Result<Analysis, BadAnalysis> {
        if frames < SPEC_PRE + 2 || rate.is_nan() || rate <= 0.0 || buf.bytes() < frames * FRAME_FLOATS * 4 {
            return Err(BadAnalysis);
        }
        Ok(Analysis { buf, frames, rate })
    }

    fn frame(&self, m: usize) -> &[f32] {
        &self.buf.as_f32()[m * FRAME_FLOATS..(m + 1) * FRAME_FLOATS]
    }

    /// Length of the recording in samples (frames after the PRE ones, one hop each).
    pub fn duration(&self) -> f64 {
        ((self.frames - SPEC_PRE) * HOP) as f64
    }
}

/// Resolved settings for one sub-block.
#[derive(Clone, Copy, Debug)]
pub struct SpecSettings {
    /// Where notes start, 0..1 of the recording.
    pub position: f32,
    /// Speed through the recording (1: its own), independent of pitch.
    pub scan: f32,
    /// Frequency ratio of the output to the recording (the note against the root, and the osc's tuning).
    pub ratio: f32,
    pub low: f32,
    pub high: f32,
    pub warp: u8,
    pub amount: f32,
    pub looped: bool,
    /// The note's frequency (for the Harmonics warp).
    pub hz: f32,
    pub filter: bool,
}

/// One voice's oscillator state.
pub struct SpectralVoice {
    acc: [f32; N],
    phase: [f32; BINS],
    smear: [f32; BINS],
    /// Source position (samples) of the next frame's centre.
    t: f64,
    /// Samples of the current hop already played.
    out_pos: usize,
    started: bool,
    /// Take the analysis phases for the next frame (the first one, and after a transient).
    reset: bool,
    last_transient: usize,
    pub done: bool,
}

impl Default for SpectralVoice {
    fn default() -> Self {
        SpectralVoice { acc: [0.0; N], phase: [0.0; BINS], smear: [0.0; BINS], t: 0.0, out_pos: 0, started: false, reset: false, last_transient: usize::MAX, done: false }
    }
}

impl SpectralVoice {
    pub fn start(&mut self) {
        self.started = false;
        self.done = false;
    }

    /// Where it is in the recording, 0..1 (for the display).
    pub fn position(&self, an: &Analysis) -> f32 {
        (self.t / an.duration()).clamp(0.0, 1.0) as f32
    }
}

/// Buffers every voice shares (one voice renders at a time).
pub struct SpecScratch {
    fft: RealFft,
    spec: Vec<Complex>,
    frame: Vec<f32>,
    win: Vec<f32>,
    mag: Vec<f32>,
    best: Vec<f32>,
    adv: Vec<f32>,
    srck: Vec<u32>,
}

impl Default for SpecScratch {
    fn default() -> Self {
        SpecScratch {
            fft: RealFft::new(N),
            spec: vec![Complex::default(); BINS],
            frame: vec![0.0; N],
            win: (0..N).map(|i| 0.5 - 0.5 * (TAU * i as f32 / N as f32).cos()).collect(),
            mag: vec![0.0; BINS],
            best: vec![0.0; BINS],
            adv: vec![0.0; BINS],
            srck: vec![0; BINS],
        }
    }
}

/// The drawn filter as a gain per bin, from SPEC_FILTER_POINTS gains over log frequency (20 Hz–20 kHz).
pub fn filter_table(points: &[f32], sr: f32, out: &mut [f32; BINS]) {
    let n = points.len().clamp(1, SPEC_FILTER_POINTS);
    for (k, o) in out.iter_mut().enumerate() {
        let f = (k as f32 * sr / N as f32).max(20.0);
        let x = ((f / 20.0).log2() / 1000f32.log2()).clamp(0.0, 1.0) * (n - 1) as f32;
        let i = (x as usize).min(n - 1);
        let j = (i + 1).min(n - 1);
        *o = (points[i] + (points[j] - points[i]) * (x - i as f32)).clamp(0.0, 1.0);
    }
}

#[inline]
fn wrap(a: f32) -> f32 {
    a - TAU * ((a + PI) / TAU).floor()
}

impl SpectralVoice {
    /// Render `start..len` into `out` (both channels, mono content).
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, sc: &mut SpecScratch, an: &Analysis, filter: &[f32; BINS], s: &SpecSettings, sr: f32, start: usize, len: usize, out: &mut [[f32; MAX_N]; 2]) {
        let rho = s.scan as f64 * an.rate as f64 / sr as f64; // source samples per output sample
        if !self.started {
            self.started = true;
            self.acc = [0.0; N];
            self.smear = [0.0; BINS];
            self.out_pos = 0;
            self.last_transient = usize::MAX;
            // the frame centred before the note, so its first hop is complete
            let t0 = s.position.clamp(0.0, 1.0) as f64 * an.duration();
            self.t = t0 + (N as f64 / 2.0 - 3.0 * HOP as f64) * rho;
            // frames centred at -512, 0, 512 and 1024 all reach into the first hop:
            // add each where it starts, moving on a hop between them. They take the
            // analysis phases, which (unwarped, at speed 1) rebuilds the start exactly.
            for f in 0..4 {
                self.reset = true;
                self.shift_and_add(sc, an, filter, s, sr, rho, f > 0);
            }
        }
        for i in start..len {
            if self.out_pos == HOP {
                self.shift_and_add(sc, an, filter, s, sr, rho, true);
            }
            let v = self.acc[self.out_pos];
            out[0][i] = v;
            out[1][i] = v;
            self.out_pos += 1;
        }
    }

    /// The analysis frames around the current position: (m0, m1, fraction), or the loop wrapped.
    fn at(&self, an: &Analysis, s: &SpecSettings) -> (usize, usize, f32) {
        let dur = an.duration();
        let mut t = self.t;
        // the loop wraps once playback runs off either end; just before the
        // start is the pre-roll (the PRE frames), not the end of the loop
        let before = -((SPEC_PRE * HOP) as f64);
        if s.looped && dur > 0.0 && (t >= dur || t < before) {
            t = t.rem_euclid(dur);
        }
        let m = (t / HOP as f64 + SPEC_PRE as f64).max(0.0);
        let last = an.frames - 1;
        let m0 = (m.floor() as usize).min(last);
        let m1 = (m0 + 1).min(last);
        (m0, m1, (m - m.floor()) as f32)
    }

    /// Move the output on by a hop (unless it's the first frame) and add the next synthesis frame.
    #[allow(clippy::too_many_arguments)]
    fn shift_and_add(&mut self, sc: &mut SpecScratch, an: &Analysis, filter: &[f32; BINS], s: &SpecSettings, sr: f32, rho: f64, shift: bool) {
        if shift {
            self.acc.copy_within(HOP.., 0);
            self.acc[N - HOP..].fill(0.0);
            self.out_pos = 0;
        }
        let dur = an.duration();
        // once past the end (or before the start, going backwards) there's nothing new to add
        let past_end = !s.looped && (self.t > dur || self.t < -(N as f64) / 2.0);
        if past_end {
            self.done = true;
        } else {
            self.synthesize(sc, an, filter, s, sr);
        }
        self.t += HOP as f64 * rho;
    }

    fn synthesize(&mut self, sc: &mut SpecScratch, an: &Analysis, filter: &[f32; BINS], s: &SpecSettings, sr: f32) {
        let (m0, m1, f) = self.at(an, s);
        let (a0, a1) = (an.frame(m0), an.frame(m1));
        let (mag0, ph0) = (&a0[..BINS], &a0[BINS..2 * BINS]);
        let (mag1, ph1) = (&a1[..BINS], &a1[BINS..2 * BINS]);
        // a transient between the frames: take the analysis phases, once
        let transient = (a0[2 * BINS] >= 0.5 && self.last_transient != m0) || (a1[2 * BINS] >= 0.5 && f > 0.5 && self.last_transient != m1);
        if transient {
            self.last_transient = if a1[2 * BINS] >= 0.5 && f > 0.5 { m1 } else { m0 };
        }
        let reset = transient || self.reset;
        self.reset = false;
        // the analysis phases nearest the position (for a reset)
        let ph_now = if f < 0.5 { ph0 } else { ph1 };
        let amt = s.amount.clamp(0.0, 1.0);
        let q = s.ratio * an.rate / sr * if s.warp == W_PITCH { (amt * 2.0).exp2() } else { 1.0 };
        let bin_hz = sr / N as f32;
        let (mag, best, adv, srck) = (&mut sc.mag, &mut sc.best, &mut sc.adv, &mut sc.srck);
        mag.fill(0.0);
        best.fill(0.0);
        let smear = s.warp == W_SMEAR;
        let keep = 1.0 - 0.97 * amt;
        let exp_adv = TAU * HOP as f32 / N as f32; // expected phase advance per hop, per bin index
        // the warps that move partials, as a map on (fractional) bin positions
        let map = |x: f32| -> f32 {
            match s.warp {
                W_SPREAD => x * (x.max(1.0) / 8.0).powf(amt * 0.3),
                W_DETUNE_UP => x + amt * 200.0 / bin_hz,
                W_DETUNE_DOWN => x - amt * 200.0 / bin_hz,
                W_BEND_UP => BINS as f32 * (x / BINS as f32).max(0.0).powf(1.0 - 0.5 * amt),
                W_BEND_DOWN => BINS as f32 * (x / BINS as f32).max(0.0).powf(1.0 + amt),
                _ => x,
            }
        };
        // put a partial's magnitude at bin `at`, sounding at `jf` (fractional bins)
        // the cuts at their ends (20 Hz, 20 kHz) cut nothing, not even DC or the top octave
        let (low, high) = (if s.low <= 20.0 { f32::NEG_INFINITY } else { s.low }, if s.high >= 20_000.0 { f32::INFINITY } else { s.high });
        let place = |at: f32, jf: f32, m: f32, k: usize, mag: &mut [f32], best: &mut [f32], adv: &mut [f32], srck: &mut [u32]| {
            let hz = jf * bin_hz;
            if hz < low || hz > high {
                return;
            }
            let j = at.round();
            if j < 0.0 || j > (BINS - 1) as f32 {
                return;
            }
            let j = j as usize;
            let mut g = if s.filter { filter[j] } else { 1.0 };
            match s.warp {
                W_HARMONICS => {
                    let c = 0.5 + 0.5 * (TAU * hz / s.hz.max(20.0)).cos();
                    g *= c.powf(1.0 + 20.0 * amt);
                }
                W_COMB => g *= 1.0 - amt * (0.5 + 0.5 * (TAU * hz / 150.0).cos()),
                _ => {}
            }
            let v = m * g;
            mag[j] += v;
            if v > best[j] {
                best[j] = v;
                // the phase moves on by this partial's frequency over one hop
                adv[j] = jf * exp_adv;
                srck[j] = k as u32;
            }
        };
        for k in 0..BINS {
            let mut m = mag0[k] + (mag1[k] - mag0[k]) * f;
            if smear {
                self.smear[k] += (m - self.smear[k]) * keep;
                m = self.smear[k];
            }
            if m < 1e-9 {
                continue;
            }
            // the partial's true frequency, in (fractional) bins, from how its phase moved
            let dev = wrap(ph1[k] - ph0[k] - k as f32 * exp_adv);
            let kf = k as f32 + dev / exp_adv;
            // the magnitude moves with the bin (keeping each partial's shape); the phase with the true frequency
            let (at, jf) = (map(k as f32 * q), map(kf * q));
            place(at, jf, m, k, mag, best, adv, srck);
            match s.warp {
                W_SUBHARMONICS if amt > 0.0 => place(at * 0.5, jf * 0.5, m * amt, k, mag, best, adv, srck),
                W_BLEND if amt > 0.0 => place(at * 2.0, jf * 2.0, m * amt, k, mag, best, adv, srck),
                _ => {}
            }
        }
        // advance the phases, build the spectrum
        let spec = &mut sc.spec;
        spec.fill(Complex::default());
        for j in 0..BINS {
            if mag[j] <= 0.0 {
                continue;
            }
            self.phase[j] = if reset { ph_now[srck[j] as usize] } else { wrap(self.phase[j] + adv[j]) };
            spec[j] = Complex::new(mag[j] * self.phase[j].cos(), mag[j] * self.phase[j].sin());
        }
        // DC and Nyquist are real
        spec[0].im = 0.0;
        spec[BINS - 1].im = 0.0;
        sc.fft.inverse(spec, &mut sc.frame);
        // Hann analysis × Hann synthesis overlapping four deep sum to 1.5
        let g = 1.0 / (1.5 * N as f32);
        for (i, a) in self.acc.iter_mut().enumerate() {
            *a += sc.frame[i] * sc.win[i] * g;
        }
    }
}

#[cfg(test)]
pub mod tests_support {
    use super::*;

    /// A stand-in for the tools' analysis (crates/tools/src/spectral.rs), for engine tests.
    pub fn analyse(x: &[f32], rate: f32) -> Analysis {
        let mut fft = RealFft::new(N);
        let win: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (TAU * i as f32 / N as f32).cos()).collect();
        let frames = SPEC_PRE + x.len().div_ceil(HOP) + 1;
        let mut buf = AssetBuf::alloc(frames * FRAME_FLOATS * 4).unwrap();
        let mut inp = vec![0.0f32; N];
        let mut out = vec![Complex::default(); BINS];
        for m in 0..frames {
            let c = (m as isize - SPEC_PRE as isize) * HOP as isize;
            for (i, v) in inp.iter_mut().enumerate() {
                let j = c - (N / 2) as isize + i as isize;
                *v = if j >= 0 && (j as usize) < x.len() { x[j as usize] * win[i] } else { 0.0 };
            }
            fft.forward(&inp, &mut out);
            let f = &mut buf.as_f32_mut()[m * FRAME_FLOATS..(m + 1) * FRAME_FLOATS];
            for k in 0..BINS {
                f[k] = (out[k].re * out[k].re + out[k].im * out[k].im).sqrt();
                f[BINS + k] = out[k].im.atan2(out[k].re);
            }
        }
        Analysis::new(buf, frames, rate).unwrap()
    }

}

#[cfg(test)]
mod tests {
    use super::tests_support::analyse;
    use super::*;

    fn settings() -> SpecSettings {
        SpecSettings { position: 0.0, scan: 1.0, ratio: 1.0, low: 20.0, high: 20_000.0, warp: W_OFF, amount: 0.0, looped: false, hz: 440.0, filter: false }
    }

    fn play(an: &Analysis, s: &SpecSettings, n: usize) -> Vec<f32> {
        let mut v = Box::<SpectralVoice>::default();
        let mut sc = SpecScratch::default();
        let flt = [1.0f32; BINS];
        let mut out = [[0.0f32; MAX_N]; 2];
        let mut y = Vec::with_capacity(n);
        while y.len() < n {
            v.render(&mut sc, an, &flt, s, 48_000.0, 0, 64, &mut out);
            y.extend_from_slice(&out[0][..64]);
        }
        y.truncate(n);
        y
    }

    /// Frequency by a least-squares fit through the rising zero crossings.
    fn pitch(x: &[f32], sr: f32) -> f64 {
        let z: Vec<f64> = (1..x.len()).filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0).map(|i| (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64).collect();
        let n = z.len() as f64;
        let mi = (n - 1.0) / 2.0;
        let mz = z.iter().sum::<f64>() / n;
        let (mut a, mut b) = (0.0, 0.0);
        for (i, &v) in z.iter().enumerate() {
            a += (i as f64 - mi) * (v - mz);
            b += (i as f64 - mi).powi(2);
        }
        sr as f64 / (a / b)
    }

    fn tone(hz: f32, secs: f32) -> Vec<f32> {
        (0..(48_000.0 * secs) as usize).map(|i| (TAU * hz * i as f32 / 48_000.0).cos() * 0.5).collect()
    }

    #[test]
    fn plays_back_the_recording_from_the_first_sample() {
        let x = tone(440.0, 1.0);
        let an = analyse(&x, 48_000.0);
        let y = play(&an, &settings(), 24_000);
        // zero latency: sound from the first sample, in step with the recording (no delay)
        assert!((y[0] - x[0]).abs() < 1e-3, "first sample {} vs {}", y[0], x[0]);
        let corr = |lag: isize| (0..512isize).map(|i| x[(i + 1024) as usize] * y[(i + 1024 + lag) as usize]).sum::<f32>();
        assert!((-64..=64).all(|l| corr(0) >= corr(l)), "best alignment is at lag 0");
        let err = x[..24_000].iter().zip(&y).fold(0.0f32, |m, (a, b)| m.max((a - b).abs()));
        assert!(err < 0.01, "resynthesis error {err}");
    }

    #[test]
    fn time_stretch_keeps_the_pitch() {
        let x = tone(440.0, 1.0);
        let an = analyse(&x, 48_000.0);
        let y = play(&an, &SpecSettings { scan: 0.5, ..settings() }, 110_000);
        let cents = 1200.0 * (pitch(&y[4096..68_000], 48_000.0) / 440.0).log2();
        assert!(cents.abs() < 1.0, "{cents} cents at half speed");
        // and it lasts twice as long
        let rms = |a: &[f32]| (a.iter().map(|v| v * v).sum::<f32>() / a.len() as f32).sqrt();
        let full = rms(&y[10_000..20_000]);
        assert!(rms(&y[86_000..90_000]) > 0.5 * full, "still playing at 1.8 s");
        assert!(rms(&y[102_000..106_000]) < 0.01 * full, "done by 2.15 s");
    }

    #[test]
    fn pitch_shift_keeps_the_duration() {
        let x = tone(440.0, 1.0);
        let an = analyse(&x, 48_000.0);
        let ratio = 2f32.powf(7.0 / 12.0);
        let y = play(&an, &SpecSettings { ratio, ..settings() }, 72_000);
        let cents = 1200.0 * (pitch(&y[4096..40_000], 48_000.0) / (440.0 * ratio as f64)).log2();
        assert!(cents.abs() < 1.0, "{cents} cents after a fifth up");
        // where the sound falls below -40 dB of its level
        let win = 480;
        let lvl = |i: usize| (y[i..i + win].iter().map(|v| v * v).sum::<f32>() / win as f32).sqrt();
        let full = lvl(10_000);
        let end = (0..(y.len() - win) / win).map(|b| b * win).rev().find(|&i| lvl(i) > full * 0.01).unwrap() + win;
        let secs = end as f64 / 48_000.0;
        assert!((secs - 1.0).abs() < 0.01 + 1024.0 / 48_000.0, "lasts {secs} s");
    }

    #[test]
    fn warps_and_cuts_stay_finite_and_do_something() {
        let x: Vec<f32> = (0..48_000).map(|i| ((i as f32 * 0.037).sin() + (i as f32 * 0.11).sin()) * 0.3).collect();
        let an = analyse(&x, 48_000.0);
        let base = play(&an, &settings(), 16_000);
        for w in 1..=11u8 {
            let y = play(&an, &SpecSettings { warp: w, amount: 0.7, looped: true, ..settings() }, 16_000);
            assert!(y.iter().all(|v| v.is_finite() && v.abs() < 4.0), "warp {w}");
            let diff = y.iter().zip(&base).skip(2048).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
            assert!(diff > 0.01, "warp {w} changed nothing");
        }
        let cut = play(&an, &SpecSettings { high: 100.0, ..settings() }, 16_000);
        assert!(cut.iter().skip(2048).all(|v| v.abs() < 1e-3), "everything is above 100 Hz here");
    }
}
