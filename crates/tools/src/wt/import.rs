//! Making a wavetable from a recording:
//!
//! - **Pitch**: the fundamental of a steady recording (YIN, refined over
//!   many periods, so a held note reads within a fraction of a cent).
//! - **Constant**: consecutive cycles of one length (the pitch's period, or
//!   one typed in), each resampled to a 2048-sample frame.
//! - **Dynamic**: cycles that follow the pitch as it moves, optionally
//!   snapped to zero crossings so each frame starts cleanly.
//! - **FFT**: consecutive blocks of 256 to 2048 samples, each block's
//!   spectrum becoming one frame's harmonics.
//!
//! When there are more cycles than frames allowed, evenly spaced ones are kept.

use std::f64::consts::PI;

use realfft::RealFftPlanner;
use wt_dsp::mip::FRAME_LEN;

use crate::resample::cycle_to_frame;

use super::spectrum::{HARMONICS, Spectra};

const LOWEST_HZ: f64 = 30.0;
const HIGHEST_HZ: f64 = 4000.0;

/// YIN's cumulative-mean-normalised difference over `x[start..]`, lags 0..=max_lag.
fn cmndf(x: &[f32], start: usize, window: usize, max_lag: usize) -> Vec<f64> {
    let mut d = vec![0.0f64; max_lag + 1];
    for (tau, dv) in d.iter_mut().enumerate().skip(1) {
        let mut s = 0.0f64;
        for j in start..start + window {
            let e = (x[j] - x[j + tau]) as f64;
            s += e * e;
        }
        *dv = s;
    }
    let mut run = 0.0;
    let mut out = vec![1.0f64; max_lag + 1];
    for tau in 1..=max_lag {
        run += d[tau];
        out[tau] = if run > 0.0 { d[tau] * tau as f64 / run } else { 1.0 };
    }
    out
}

/// Parabolic interpolation of a minimum at `i` of `f` (fractional offset added to i).
fn parabola(f: impl Fn(usize) -> f64, i: usize) -> f64 {
    let (a, b, c) = (f(i - 1), f(i), f(i + 1));
    let den = a - 2.0 * b + c;
    if den.abs() < 1e-18 { i as f64 } else { i as f64 + 0.5 * (a - c) / den }
}

/// The period in samples of the part of `x` starting at `start` (a YIN estimate), or None if unpitched.
fn yin_period(x: &[f32], start: usize, window: usize, sr: f64) -> Option<f64> {
    let min_lag = ((sr / HIGHEST_HZ).floor() as usize).max(2);
    let max_lag = ((sr / LOWEST_HZ).ceil() as usize).min(x.len().saturating_sub(start + window + 1));
    if max_lag <= min_lag + 2 {
        return None;
    }
    let d = cmndf(x, start, window, max_lag);
    let mut best = None;
    let mut tau = min_lag;
    while tau < max_lag {
        if d[tau] < 0.12 {
            while tau + 1 < max_lag && d[tau + 1] < d[tau] {
                tau += 1;
            }
            best = Some(tau);
            break;
        }
        tau += 1;
    }
    let tau = match best {
        Some(t) => t,
        None => {
            let (t, v) = (min_lag..max_lag).map(|t| (t, d[t])).fold((0, f64::MAX), |a, b| if b.1 < a.1 { b } else { a });
            if v > 0.4 {
                return None;
            }
            t
        }
    };
    Some(parabola(|i| d[i], tau))
}

/// Sum of squared differences between `x` and itself `lag` samples later.
fn diff(x: &[f32], lag: usize) -> f64 {
    let n = x.len() - lag;
    let mut s = 0.0f64;
    for j in 0..n {
        let e = (x[j] - x[j + lag]) as f64;
        s += e * e;
    }
    s / n as f64
}

/// The fundamental of a steady recording, in Hz.
pub fn detect_pitch(x: &[f32], sr: f32) -> Option<f32> {
    let sr = sr as f64;
    let max_lag = (sr / LOWEST_HZ).ceil() as usize;
    if x.len() < 2 * max_lag + 64 {
        return None;
    }
    let window = (x.len() - max_lag - 2).min(8192);
    let start = (x.len() - window - max_lag - 1) / 2;
    let t0 = yin_period(x, start, window, sr)?;
    // Refine over many periods: the lag of k periods, found to a fraction of
    // a sample and divided by k, is k times more precise than one period.
    // Doubling k each time keeps every step's search within a sample or two
    // of the minimum it's after.
    let mut t = t0;
    let mut k = 1.0f64;
    while 2.0 * k * t < x.len() as f64 / 2.0 {
        k *= 2.0;
        let mut at = (k * t).round() as usize;
        for _ in 0..(t as usize / 2).max(4) {
            let (a, b, c) = (diff(x, at - 1), diff(x, at), diff(x, at + 1));
            if a < b && a <= c {
                at -= 1;
            } else if c < b {
                at += 1;
            } else {
                break;
            }
        }
        t = parabola(|i| diff(x, i), at) / k;
    }
    Some((sr / t) as f32)
}

/// Read `x` at a fractional position through a Blackman-windowed sinc (x is band-limited).
fn read(x: &[f32], pos: f64) -> f32 {
    const HALF: isize = 16;
    let i = pos.floor() as isize;
    let f = pos - i as f64;
    let mut s = 0.0f64;
    for k in -HALF + 1..=HALF {
        let j = i + k;
        if j < 0 || j >= x.len() as isize {
            continue;
        }
        let t = k as f64 - f;
        let sinc = if t.abs() < 1e-9 { 1.0 } else { (PI * t).sin() / (PI * t) };
        let u = (t / HALF as f64 + 1.0) / 2.0; // 0..1 across the window
        let w = 0.42 - 0.5 * (2.0 * PI * u).cos() + 0.08 * (4.0 * PI * u).cos();
        s += x[j as usize] as f64 * sinc * w;
    }
    s as f32
}

/// One cycle of `len` samples from `start`, as a 2048-sample frame.
fn cycle(x: &[f32], start: f64, len: f64, dst: &mut [f32]) {
    if len >= FRAME_LEN as f64 {
        // longer than a frame: take whole samples and band-limit through the FFT
        let a = start.round() as usize;
        let n = (len.round() as usize).max(2);
        let mut c: Vec<f32> = x[a.min(x.len())..(a + n).min(x.len())].to_vec();
        c.resize(n, 0.0);
        cycle_to_frame(&c, dst);
    } else {
        for (j, d) in dst.iter_mut().enumerate() {
            *d = read(x, start + j as f64 * len / FRAME_LEN as f64);
        }
    }
}

/// Keep at most `max` of `n` items, evenly spaced (first and last included).
fn spread(n: usize, max: usize) -> Vec<usize> {
    if n <= max {
        return (0..n).collect();
    }
    (0..max).map(|i| ((i as f64 * (n - 1) as f64) / (max - 1).max(1) as f64).round() as usize).collect()
}

/// Consecutive cycles of `period` samples (fractional is fine).
pub fn constant(x: &[f32], period: f64, max_frames: usize) -> Vec<f32> {
    let period = period.max(2.0);
    // (a hair of slack, so a recording of exactly n cycles counts n)
    let n = ((x.len() as f64 / period + 1e-3).floor() as usize).max(1);
    let keep = spread(n, max_frames.max(1));
    let mut out = vec![0.0f32; keep.len() * FRAME_LEN];
    for (f, &c) in keep.iter().enumerate() {
        cycle(x, c as f64 * period, period.min(x.len() as f64), &mut out[f * FRAME_LEN..(f + 1) * FRAME_LEN]);
    }
    out
}

/// The rising zero crossing nearest `pos` within `reach` samples, fractional.
fn zero_near(x: &[f32], pos: f64, reach: f64) -> Option<f64> {
    let a = (pos - reach).max(1.0) as usize;
    let b = ((pos + reach) as usize).min(x.len() - 1);
    let mut best: Option<f64> = None;
    for i in a..=b {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            let z = (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64;
            if best.is_none_or(|b| (z - pos).abs() < (b - pos).abs()) {
                best = Some(z);
            }
        }
    }
    best
}

/// Cycles that follow the pitch: each cycle as long as the period where it starts.
pub fn dynamic(x: &[f32], sr: f32, snap: bool, max_frames: usize) -> Vec<f32> {
    let sr64 = sr as f64;
    let hop = 512usize;
    let window = 1024usize;
    let max_lag = (sr64 / LOWEST_HZ).ceil() as usize;
    // the period every hop (None where there's no pitch)
    let mut track: Vec<Option<f64>> = Vec::new();
    let mut s = 0;
    while s + window + max_lag + 2 < x.len() {
        track.push(yin_period(x, s, window, sr64));
        s += hop;
    }
    // fill gaps from the nearest pitched hop
    let known: Vec<(usize, f64)> = track.iter().enumerate().filter_map(|(i, p)| p.map(|p| (i, p))).collect();
    if known.is_empty() {
        return constant(x, FRAME_LEN as f64, max_frames);
    }
    let period_at = |pos: f64| -> f64 {
        let i = ((pos - window as f64 / 2.0) / hop as f64).max(0.0);
        let (i0, frac) = (i.floor() as usize, i - i.floor());
        let get = |i: usize| -> f64 {
            track.get(i).copied().flatten().unwrap_or_else(|| known.iter().min_by_key(|(j, _)| j.abs_diff(i)).unwrap().1)
        };
        get(i0) + (get(i0 + 1) - get(i0)) * frac
    };
    let mut cycles: Vec<(f64, f64)> = Vec::new();
    let mut p = if snap { zero_near(x, period_at(0.0), period_at(0.0)).unwrap_or(0.0) } else { 0.0 };
    loop {
        let t = period_at(p);
        let mut next = p + t;
        if snap && let Some(z) = zero_near(x, next, t / 4.0) {
            next = z;
        }
        if next + 1.0 >= x.len() as f64 || next <= p + 2.0 {
            break;
        }
        cycles.push((p, next - p));
        p = next;
    }
    if cycles.is_empty() {
        return constant(x, period_at(0.0), max_frames);
    }
    let keep = spread(cycles.len(), max_frames.max(1));
    let mut out = vec![0.0f32; keep.len() * FRAME_LEN];
    for (f, &c) in keep.iter().enumerate() {
        let (start, len) = cycles[c];
        cycle(x, start, len, &mut out[f * FRAME_LEN..(f + 1) * FRAME_LEN]);
    }
    out
}

/// Consecutive blocks of `size` samples, each block's spectrum one frame's harmonics.
pub fn fft_split(x: &[f32], size: usize, max_frames: usize) -> Vec<f32> {
    let size = size.clamp(256, FRAME_LEN).next_power_of_two().min(FRAME_LEN);
    let n = (x.len() / size).max(1);
    let keep = spread(n, max_frames.max(1));
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(size);
    let mut buf = vec![0.0f32; size];
    let mut spec = fft.make_output_vec();
    let mut s = Spectra::default();
    let mut mag = vec![0.0f32; HARMONICS + 1];
    let mut ph = vec![0.0f32; HARMONICS + 1];
    let mut out = vec![0.0f32; keep.len() * FRAME_LEN];
    for (f, &b) in keep.iter().enumerate() {
        let a = b * size;
        buf.fill(0.0);
        let end = (a + size).min(x.len());
        buf[..end - a].copy_from_slice(&x[a..end]);
        fft.process(&mut buf, &mut spec).expect("forward fft");
        mag.fill(0.0);
        ph.fill(0.0);
        for k in 1..=size / 2 {
            let c = spec[k];
            mag[k] = c.norm() * 2.0 / size as f32;
            ph[k] = c.im.atan2(c.re) + std::f32::consts::FRAC_PI_2;
        }
        s.synthesize(&mag, &ph, &mut out[f * FRAME_LEN..(f + 1) * FRAME_LEN]);
    }
    super::process::apply(super::process::Process::NormalizeAll, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A band-limited saw at `hz` for `secs`.
    fn saw(hz: f64, sr: f64, secs: f64) -> Vec<f32> {
        let n = (sr * secs) as usize;
        let harmonics = ((sr / 2.0) / hz).floor() as usize;
        (0..n)
            .map(|i| {
                let t = i as f64 / sr;
                let mut v = 0.0;
                for h in 1..=harmonics.min(60) {
                    v += (2.0 * PI * hz * h as f64 * t).sin() / h as f64;
                }
                (v * 0.5) as f32
            })
            .collect()
    }

    #[test]
    fn pitch_detection_is_within_a_cent() {
        let mut worst = 0.0f64;
        for &sr in &[44_100.0, 48_000.0] {
            for &hz in &[55.0, 110.0, 220.5, 261.63, 441.0, 1000.0, 1760.0] {
                let x = saw(hz, sr, 1.0);
                let got = detect_pitch(&x, sr as f32).unwrap() as f64;
                let cents = 1200.0 * (got / hz).log2();
                assert!(cents.abs() < 1.0, "{hz} Hz at {sr}: read {got} ({cents:+.3} cents)");
                worst = worst.max(cents.abs());
            }
        }
        eprintln!("pitch detection: worst {worst:.4} cents");
        // noise has no pitch; a short clip can't be measured
        let mut r = wt_dsp::rng::Rng::new(3);
        let noise: Vec<f32> = (0..48_000).map(|_| r.next_f32() * 2.0 - 1.0).collect();
        assert!(detect_pitch(&noise, 48_000.0).is_none());
        assert!(detect_pitch(&saw(220.0, 48_000.0, 0.01), 48_000.0).is_none());
    }

    #[test]
    fn constant_import_makes_one_frame_per_cycle() {
        let sr = 48_000.0;
        let x = saw(200.0, sr, 0.5); // 100 cycles of 240 samples
        let hz = detect_pitch(&x, sr as f32).unwrap() as f64;
        let t = constant(&x, sr / hz, 256);
        assert_eq!(t.len() / FRAME_LEN, 100);
        // every frame is the same saw (same phase), up to the interpolation
        let a = &t[10 * FRAME_LEN..11 * FRAME_LEN];
        let b = &t[80 * FRAME_LEN..81 * FRAME_LEN];
        let err = a.iter().zip(b).fold(0.0f32, |m, (x, y)| m.max((x - y).abs()));
        assert!(err < 0.02, "{err}");
        // more cycles than frames: evenly spaced ones
        assert_eq!(constant(&saw(1000.0, sr, 1.0), 48.0, 256).len() / FRAME_LEN, 256);
        // a typed length longer than a frame
        assert_eq!(constant(&x, 3000.0, 256).len() / FRAME_LEN, 8);
    }

    #[test]
    fn dynamic_import_follows_a_glide() {
        // a sine gliding from 200 to 400 Hz over 0.5 s
        let sr = 48_000.0f64;
        let n = 24_000;
        let mut phase = 0.0f64;
        let x: Vec<f32> = (0..n)
            .map(|i| {
                let hz = 200.0 * 2f64.powf(i as f64 / n as f64);
                phase += hz / sr;
                (2.0 * PI * phase).sin() as f32
            })
            .collect();
        let cycles = phase.floor() as usize;
        for snap in [false, true] {
            let t = dynamic(&x, sr as f32, snap, 1024);
            let frames = t.len() / FRAME_LEN;
            assert!(frames.abs_diff(cycles) <= 3, "snap {snap}: {frames} frames for {cycles} cycles");
            // each frame is close to one cycle of a sine
            let f = &t[(frames / 2) * FRAME_LEN..(frames / 2 + 1) * FRAME_LEN];
            let mut s = Spectra::default();
            let mut m = vec![0.0; HARMONICS + 1];
            let mut p = vec![0.0; HARMONICS + 1];
            s.analyze(f, &mut m, &mut p);
            assert!(m[1] > 0.9 && m[2] < 0.05, "snap {snap}: fundamental {} second {}", m[1], m[2]);
        }
        assert_eq!(dynamic(&x, sr as f32, true, 64).len() / FRAME_LEN, 64);
    }

    #[test]
    fn fft_split_counts_blocks_and_keeps_harmonics() {
        // bins land on harmonics when the tone fits the block: 3 cycles per 1024 samples
        let x: Vec<f32> = (0..48_000).map(|i| (2.0 * PI * 3.0 * i as f64 / 1024.0).sin() as f32).collect();
        let t = fft_split(&x, 1024, 256);
        assert_eq!(t.len() / FRAME_LEN, 46);
        let mut s = Spectra::default();
        let mut m = vec![0.0; HARMONICS + 1];
        let mut p = vec![0.0; HARMONICS + 1];
        s.analyze(&t[..FRAME_LEN], &mut m, &mut p);
        assert!(m[3] > 0.99 && m.iter().enumerate().all(|(k, &v)| k == 3 || v < 1e-3));
        assert_eq!(fft_split(&x, 256, 100).len() / FRAME_LEN, 100);
    }
}
