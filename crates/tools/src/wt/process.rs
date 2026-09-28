//! The editor's Process menu and its spectral operations. Each works on the
//! frames it's given (the selection, or the whole table) in place.

use wt_dsp::mip::FRAME_LEN;
use wt_dsp::rng::Rng;

use super::spectrum::{HARMONICS, Spectra};

/// One operation and its settings, as the tools ABI passes them (op, args).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Process {
    /// Each frame's peak to 1.
    NormalizeEach,
    /// One gain for all frames: the loudest peak to 1.
    NormalizeAll,
    RemoveDc,
    /// Play each frame backwards.
    FlipH,
    /// Turn each frame upside down.
    FlipV,
    /// Fade the level in (0 → 1) or out across the frames.
    FadeIn,
    FadeOut,
    /// Low-pass (or high-pass) on the harmonics: cutoff harmonic, order (2 = 12 dB/oct).
    LowPass { cutoff: f32, order: f32 },
    HighPass { cutoff: f32, order: f32 },
    /// Hold every `factor`-th sample: a lower sample rate inside the cycle.
    Downsample { factor: f32 },
    RemoveFundamental,
    /// Smooth the harmonic amplitudes over `width` harmonics (phases stay).
    BlurSpectrum { width: f32 },
    /// Zero every harmonic above `from` (clear highs) or below `to` (clear lows).
    ClearAbove { from: f32 },
    ClearBelow { to: f32 },
    RandomizePhases { seed: u32 },
    /// Every harmonic an octave up (the wave twice per cycle) or down (keeping the even ones).
    OctaveUp,
    OctaveDown,
    OddOnly,
    EvenOnly,
}

impl Process {
    /// From the ABI: `op` and up to two float arguments.
    pub fn from_abi(op: u32, a: f32, b: f32) -> Option<Process> {
        Some(match op {
            0 => Process::NormalizeEach,
            1 => Process::NormalizeAll,
            2 => Process::RemoveDc,
            3 => Process::FlipH,
            4 => Process::FlipV,
            5 => Process::FadeIn,
            6 => Process::FadeOut,
            7 => Process::LowPass { cutoff: a, order: b },
            8 => Process::HighPass { cutoff: a, order: b },
            9 => Process::Downsample { factor: a },
            10 => Process::RemoveFundamental,
            11 => Process::BlurSpectrum { width: a },
            12 => Process::ClearAbove { from: a },
            13 => Process::ClearBelow { to: a },
            14 => Process::RandomizePhases { seed: a as u32 },
            15 => Process::OctaveUp,
            16 => Process::OctaveDown,
            17 => Process::OddOnly,
            18 => Process::EvenOnly,
            _ => return None,
        })
    }
}

fn peak(f: &[f32]) -> f32 {
    f.iter().fold(0.0f32, |m, &v| m.max(v.abs()))
}

/// Apply `op` to `frames` (count × 2048).
pub fn apply(op: Process, frames: &mut [f32]) {
    let count = frames.len() / FRAME_LEN;
    let frame = |i: usize| i * FRAME_LEN..(i + 1) * FRAME_LEN;
    match op {
        Process::NormalizeEach => {
            for i in 0..count {
                let f = &mut frames[frame(i)];
                let p = peak(f);
                if p > 1e-9 {
                    f.iter_mut().for_each(|v| *v /= p);
                }
            }
        }
        Process::NormalizeAll => {
            let p = peak(frames);
            if p > 1e-9 {
                frames.iter_mut().for_each(|v| *v /= p);
            }
        }
        Process::RemoveDc => {
            for i in 0..count {
                let f = &mut frames[frame(i)];
                let m = f.iter().map(|&v| v as f64).sum::<f64>() / FRAME_LEN as f64;
                f.iter_mut().for_each(|v| *v -= m as f32);
            }
        }
        Process::FlipH => {
            // time reversal about sample 0, so a saw stays a saw, just falling
            for i in 0..count {
                let f = &mut frames[frame(i)];
                f[1..].reverse();
            }
        }
        Process::FlipV => frames.iter_mut().for_each(|v| *v = -*v),
        Process::FadeIn | Process::FadeOut => {
            for i in 0..count {
                let t = if count > 1 { i as f32 / (count - 1) as f32 } else { 1.0 };
                let g = if op == Process::FadeIn { t } else { 1.0 - t };
                frames[frame(i)].iter_mut().for_each(|v| *v *= g);
            }
        }
        Process::Downsample { factor } => {
            let step = factor.max(1.0);
            for i in 0..count {
                let f = &mut frames[frame(i)];
                let mut hold = f[0];
                let mut next = 0.0f32;
                for (n, v) in f.iter_mut().enumerate() {
                    if n as f32 >= next {
                        hold = *v;
                        next += step;
                    }
                    *v = hold;
                }
            }
        }
        _ => spectral(op, frames),
    }
}

/// Operations on the harmonics: analyse each frame, change, resynthesize.
fn spectral(op: Process, frames: &mut [f32]) {
    let count = frames.len() / FRAME_LEN;
    let mut s = Spectra::default();
    let mut mag = vec![0.0f32; HARMONICS + 1];
    let mut ph = vec![0.0f32; HARMONICS + 1];
    let mut tmp = vec![0.0f32; HARMONICS + 1];
    let mut rng = if let Process::RandomizePhases { seed } = op { Rng::new(seed) } else { Rng::new(1) };
    for i in 0..count {
        let f = &mut frames[i * FRAME_LEN..(i + 1) * FRAME_LEN];
        s.analyze(f, &mut mag, &mut ph);
        match op {
            Process::LowPass { cutoff, order } | Process::HighPass { cutoff, order } => {
                let kc = cutoff.max(0.5);
                let o = order.clamp(0.5, 8.0);
                for k in 1..=HARMONICS {
                    let r = (k as f32 / kc).powf(2.0 * o);
                    mag[k] *= if matches!(op, Process::LowPass { .. }) { 1.0 / (1.0 + r).sqrt() } else { (r / (1.0 + r)).sqrt() };
                }
                if matches!(op, Process::HighPass { .. }) {
                    mag[0] = 0.0;
                }
            }
            Process::RemoveFundamental => mag[1] = 0.0,
            Process::BlurSpectrum { width } => {
                let w = width.max(0.0);
                if w > 0.0 {
                    let r = (3.0 * w).ceil() as isize;
                    for k in 1..=HARMONICS {
                        let (mut sum, mut norm) = (0.0f32, 0.0f32);
                        for d in -r..=r {
                            let j = k as isize + d;
                            if j < 1 || j > HARMONICS as isize {
                                continue;
                            }
                            let g = (-(d * d) as f32 / (2.0 * w * w)).exp();
                            sum += mag[j as usize] * g;
                            norm += g;
                        }
                        tmp[k] = sum / norm;
                    }
                    mag[1..].copy_from_slice(&tmp[1..]);
                }
            }
            Process::ClearAbove { from } => {
                for k in (from.max(1.0).floor() as usize + 1)..=HARMONICS {
                    mag[k] = 0.0;
                }
            }
            Process::ClearBelow { to } => {
                for k in 1..(to.max(1.0).ceil() as usize).min(HARMONICS + 1) {
                    mag[k] = 0.0;
                }
                mag[0] = 0.0;
            }
            Process::RandomizePhases { .. } => {
                // not the Nyquist bin: it has no phase to turn (it's cos only)
                for p in ph[1..HARMONICS].iter_mut() {
                    *p = (rng.next_f32() * 2.0 - 1.0) * std::f32::consts::PI;
                }
            }
            Process::OctaveUp => {
                tmp.fill(0.0);
                let mut tp = vec![0.0f32; HARMONICS + 1];
                for k in 1..=HARMONICS / 2 {
                    tmp[2 * k] = mag[k];
                    tp[2 * k] = ph[k];
                }
                mag[1..].copy_from_slice(&tmp[1..]);
                ph[1..].copy_from_slice(&tp[1..]);
            }
            Process::OctaveDown => {
                for k in 1..=HARMONICS {
                    let src = 2 * k;
                    if src <= HARMONICS {
                        mag[k] = mag[src];
                        ph[k] = ph[src];
                    } else {
                        mag[k] = 0.0;
                    }
                }
            }
            Process::OddOnly | Process::EvenOnly => {
                let keep_odd = op == Process::OddOnly;
                for k in 1..=HARMONICS {
                    if (k % 2 == 1) != keep_odd {
                        mag[k] = 0.0;
                    }
                }
            }
            _ => {}
        }
        s.synthesize(&mag, &ph, f);
    }
}

/// A PWM series from one frame: `count` frames of `f(t) - f(t + w)`, the
/// width sweeping from half a cycle down to one step, DC removed and
/// normalised together.
pub fn create_pwm(src: &[f32], count: usize) -> Vec<f32> {
    let count = count.max(1);
    let mut out = vec![0.0f32; count * FRAME_LEN];
    for i in 0..count {
        let w = 0.5 * (1.0 - i as f32 / count as f32);
        let shift = (w * FRAME_LEN as f32).round() as usize;
        let f = &mut out[i * FRAME_LEN..(i + 1) * FRAME_LEN];
        for n in 0..FRAME_LEN {
            f[n] = src[n] - src[(n + shift) % FRAME_LEN];
        }
    }
    apply(Process::RemoveDc, &mut out);
    apply(Process::NormalizeAll, &mut out);
    out
}

/// Frame order by brightness (spectral centroid), darkest first.
pub fn brightness_order(frames: &[f32]) -> Vec<usize> {
    let count = frames.len() / FRAME_LEN;
    let mut s = Spectra::default();
    let mut mag = vec![0.0f32; HARMONICS + 1];
    let mut ph = vec![0.0f32; HARMONICS + 1];
    let mut c: Vec<(f32, usize)> = (0..count)
        .map(|i| {
            s.analyze(&frames[i * FRAME_LEN..(i + 1) * FRAME_LEN], &mut mag, &mut ph);
            let (mut num, mut den) = (0.0f64, 0.0f64);
            for (k, &m) in mag.iter().enumerate().skip(1) {
                let e = (m as f64) * (m as f64);
                num += k as f64 * e;
                den += e;
            }
            ((if den > 0.0 { num / den } else { 0.0 }) as f32, i)
        })
        .collect();
    c.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    c.into_iter().map(|(_, i)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn saw() -> Vec<f32> {
        (0..FRAME_LEN).map(|i| 2.0 * i as f32 / FRAME_LEN as f32 - 1.0).collect()
    }
    fn harmonics(f: &[f32]) -> Vec<f32> {
        let mut s = Spectra::default();
        let mut m = vec![0.0; HARMONICS + 1];
        let mut p = vec![0.0; HARMONICS + 1];
        s.analyze(f, &mut m, &mut p);
        m
    }

    #[test]
    fn normalize_dc_flips_and_fades() {
        let mut t: Vec<f32> = saw().iter().chain(saw().iter()).map(|v| v * 0.5 + 0.25).collect();
        t[FRAME_LEN..].iter_mut().for_each(|v| *v *= 0.5);
        apply(Process::RemoveDc, &mut t);
        assert!(t[..FRAME_LEN].iter().sum::<f32>().abs() < 1e-2);
        let mut e = t.clone();
        apply(Process::NormalizeEach, &mut e);
        assert!((peak(&e[..FRAME_LEN]) - 1.0).abs() < 1e-6 && (peak(&e[FRAME_LEN..]) - 1.0).abs() < 1e-6);
        let mut a = t.clone();
        apply(Process::NormalizeAll, &mut a);
        assert!((peak(&a[..FRAME_LEN]) - 1.0).abs() < 1e-6 && (peak(&a[FRAME_LEN..]) - 0.5).abs() < 1e-3);
        let mut v = saw();
        apply(Process::FlipV, &mut v);
        assert_eq!(v[10], -saw()[10]);
        let mut h = saw();
        apply(Process::FlipH, &mut h);
        assert_eq!(h[1], saw()[FRAME_LEN - 1]);
        let mut f = [saw(), saw(), saw()].concat();
        apply(Process::FadeIn, &mut f);
        assert_eq!(peak(&f[..FRAME_LEN]), 0.0);
        assert!((peak(&f[FRAME_LEN..2 * FRAME_LEN]) - 0.5).abs() < 1e-3);
    }

    #[test]
    fn spectral_operations_move_the_harmonics() {
        let s = saw();
        let base = harmonics(&s);
        let mut odd = s.clone();
        apply(Process::OddOnly, &mut odd);
        let m = harmonics(&odd);
        assert!(m[2] < 1e-5 && (m[3] - base[3]).abs() < 1e-4);
        let mut up = s.clone();
        apply(Process::OctaveUp, &mut up);
        let m = harmonics(&up);
        assert!(m[1] < 1e-5 && (m[2] - base[1]).abs() < 1e-4 && (m[4] - base[2]).abs() < 1e-4);
        let mut down = up.clone();
        apply(Process::OctaveDown, &mut down);
        let m = harmonics(&down);
        assert!((m[1] - base[1]).abs() < 1e-4 && (m[3] - base[3]).abs() < 1e-4);
        let mut lp = s.clone();
        apply(Process::LowPass { cutoff: 10.0, order: 2.0 }, &mut lp);
        let m = harmonics(&lp);
        assert!((m[10] / base[10] - 0.5f32.sqrt()).abs() < 1e-3, "-3 dB at the cutoff");
        assert!((m[20] / base[20] - 1.0 / 17f32.sqrt()).abs() < 1e-3, "12 dB/oct");
        let mut clear = s.clone();
        apply(Process::ClearAbove { from: 5.0 }, &mut clear);
        let m = harmonics(&clear);
        assert!(m[6] < 1e-5 && (m[5] - base[5]).abs() < 1e-4);
        let mut nf = s.clone();
        apply(Process::RemoveFundamental, &mut nf);
        assert!(harmonics(&nf)[1] < 1e-5);
        let mut rp = s.clone();
        apply(Process::RandomizePhases { seed: 3 }, &mut rp);
        let m = harmonics(&rp);
        assert!(m.iter().zip(&base).skip(1).all(|(a, b)| (a - b).abs() < 1e-4), "magnitudes stay");
        assert!(rp.iter().zip(&s).any(|(a, b)| (a - b).abs() > 0.1), "the shape changes");
    }

    #[test]
    fn pwm_from_a_saw_makes_pulses() {
        let t = create_pwm(&saw(), 8);
        assert_eq!(t.len(), 8 * FRAME_LEN);
        // a saw minus itself shifted half a cycle is a square: two levels
        let f0 = &t[..FRAME_LEN];
        let hi = f0.iter().filter(|&&v| v > 0.5).count();
        let lo = f0.iter().filter(|&&v| v < -0.5).count();
        assert!(hi > 900 && lo > 900, "{hi} {lo}");
        assert!((peak(&t) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn sorts_by_brightness() {
        let sine: Vec<f32> = (0..FRAME_LEN).map(|i| (TAU * i as f32 / FRAME_LEN as f32).sin()).collect();
        let t = [saw(), sine.clone(), saw()].concat();
        assert_eq!(brightness_order(&t), vec![1, 0, 2]);
    }
}
