//! birdsynth's noise library, generated on demand (seeded, so every build
//! makes the same samples). Order matches `noise.type` in params/noise.toml.
//! The coloured noises are built in the frequency domain, so they loop
//! seamlessly; the textures are built in time and crossfaded into a loop.

use std::f64::consts::TAU;

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;
use wt_dsp::rng::Rng;

/// Samples per noise, at 48 kHz: about 2.7 seconds.
pub const LEN: usize = 1 << 17;
pub const RATE: f32 = 48_000.0;
pub const NAMES: [&str; 12] = ["White", "Pink", "Brown", "Blue", "Violet", "Crackle", "Digital", "Hiss", "Wind", "Rain", "Metal", "Geiger"];

/// Build noise `index` into `out` (LEN samples), normalized to a 0.9 peak.
pub fn build(index: usize, out: &mut [f32]) {
    assert_eq!(out.len(), LEN);
    let mut rng = Rng::new(0x0015E + index as u32 * 104_729);
    match index {
        0 => spectral(out, &mut rng, |_| 1.0),
        1 => spectral(out, &mut rng, |f| 1.0 / f.max(20.0).sqrt()),
        2 => spectral(out, &mut rng, |f| 1.0 / f.max(20.0)),
        3 => spectral(out, &mut rng, |f| f.sqrt()),
        4 => spectral(out, &mut rng, |f| f),
        5 => crackle(out, &mut rng),
        6 => digital(out, &mut rng),
        7 => spectral(out, &mut rng, |f| {
            let x = (f / 4000.0).powi(2);
            x / (1.0 + x)
        }),
        8 => wind(out, &mut rng),
        9 => rain(out, &mut rng),
        10 => spectral(out, &mut rng, |f| {
            const MODES: [f64; 8] = [523.0, 1187.0, 1873.0, 2791.0, 3517.0, 4999.0, 6311.0, 8081.0];
            MODES.iter().enumerate().map(|(i, &m)| 1.0 / (1.0 + ((f - m) / (6.0 + i as f64 * 2.0)).powi(2)) / (1.0 + i as f64 * 0.3)).sum::<f64>() + 0.002
        }),
        _ => geiger(out, &mut rng),
    }
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak > 0.0 {
        for v in out.iter_mut() {
            *v *= 0.9 / peak;
        }
    }
}

/// Random-phase noise with a magnitude shape over frequency (Hz); periodic over LEN.
fn spectral(out: &mut [f32], rng: &mut Rng, mag: impl Fn(f64) -> f64) {
    let inv = RealFftPlanner::<f32>::new().plan_fft_inverse(LEN);
    let mut spec = inv.make_input_vec();
    let bin_hz = RATE as f64 / LEN as f64;
    for (k, c) in spec.iter_mut().enumerate().skip(1) {
        let f = k as f64 * bin_hz;
        if f > 20_000.0 {
            continue;
        }
        let a = mag(f);
        let ph = rng.next_f32() as f64 * TAU;
        *c = Complex::new((a * ph.cos()) as f32, (a * ph.sin()) as f32);
    }
    spec[0] = Complex::new(0.0, 0.0);
    let last = spec.len() - 1;
    spec[last].im = 0.0;
    inv.process(&mut spec, out).expect("inverse fft");
}

fn white(rng: &mut Rng) -> f32 {
    rng.next_f32() * 2.0 - 1.0
}

/// Fold `src` (out.len() + xfade samples) into a seamless loop: the extra
/// tail is crossfaded into the head, so the last sample flows into the first.
fn fold_loop(src: &[f32], xfade: usize, out: &mut [f32]) {
    let m = out.len();
    assert_eq!(src.len(), m + xfade);
    out.copy_from_slice(&src[..m]);
    for i in 0..xfade {
        let t = i as f32 / xfade as f32;
        out[i] = src[i] * t + src[m + i] * (1.0 - t);
    }
}

fn crackle(out: &mut [f32], rng: &mut Rng) {
    for v in out.iter_mut() {
        *v = white(rng) * 0.004;
    }
    let mut t = 0usize;
    while t < LEN {
        t += (rng.next_f32() * 3200.0) as usize + 60;
        if t >= LEN {
            break;
        }
        let a = (rng.next_f32().powi(3)) * if rng.next_f32() < 0.5 { 1.0 } else { -1.0 };
        let len = 2 + (rng.next_f32() * 30.0) as usize;
        for k in 0..len {
            if t + k < LEN {
                out[t + k] += a * (-(k as f32) / (len as f32 * 0.3)).exp() * if k % 2 == 0 { 1.0 } else { -0.6 };
            }
        }
    }
}

fn digital(out: &mut [f32], rng: &mut Rng) {
    let mut i = 0;
    while i < LEN {
        let hold = 1 + (rng.next_f32().powi(2) * 48.0) as usize;
        let levels = [2.0f32, 4.0, 8.0, 16.0][(rng.next_u32() % 4) as usize];
        let v = (white(rng) * levels).round() / levels;
        for k in 0..hold {
            if i + k < LEN {
                out[i + k] = v;
            }
        }
        i += hold;
    }
}

fn wind(out: &mut [f32], rng: &mut Rng) {
    // white noise through a resonant band-pass whose centre wanders slowly
    const XF: usize = 8192;
    let mut raw = vec![0.0f32; LEN + XF];
    let (mut ic1, mut ic2) = (0.0f32, 0.0f32);
    for (i, v) in raw.iter_mut().enumerate() {
        let t = i as f32 / RATE;
        let centre = 500.0 + 400.0 * (0.21 * t * std::f32::consts::TAU).sin() + 250.0 * (0.53 * t * std::f32::consts::TAU + 1.3).sin();
        let g = (std::f32::consts::PI * centre / RATE).tan();
        let k = 0.25;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let x = white(rng);
        let v3 = x - ic2;
        let v1 = a1 * ic1 + g * a1 * v3;
        let v2 = ic2 + g * a1 * ic1 + g * g * a1 * v3;
        ic1 = 2.0 * v1 - ic1;
        ic2 = 2.0 * v2 - ic2;
        let gust = 0.6 + 0.4 * (0.13 * t * std::f32::consts::TAU).sin();
        *v = v1 * k * gust;
    }
    fold_loop(&raw, XF, out);
}

fn rain(out: &mut [f32], rng: &mut Rng) {
    let mut bg = vec![0.0f32; LEN];
    spectral(&mut bg, rng, |f| 1.0 / f.max(200.0).sqrt());
    let bg_peak = bg.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    for (o, b) in out.iter_mut().zip(bg) {
        *o = b / bg_peak * 0.08;
    }
    let drops = (LEN as f32 / RATE * 220.0) as usize;
    for _ in 0..drops {
        let t0 = (rng.next_f32() * LEN as f32) as usize;
        let hz = 1500.0 + rng.next_f32() * 3500.0;
        let decay = 0.002 + rng.next_f32() * 0.008;
        let amp = rng.next_f32().powi(2) * 0.5;
        let len = (decay * RATE * 6.0) as usize;
        for k in 0..len {
            let i = (t0 + k) % LEN; // wrap, so the loop stays seamless
            let t = k as f32 / RATE;
            out[i] += amp * (-t / decay).exp() * (std::f32::consts::TAU * hz * t).sin();
        }
    }
}

fn geiger(out: &mut [f32], rng: &mut Rng) {
    out.fill(0.0);
    let mut t = 0usize;
    while t < LEN {
        t += (-(1.0 - rng.next_f32()).ln() * RATE / 9.0) as usize + 20;
        if t + 8 >= LEN {
            break;
        }
        out[t] = 1.0;
        out[t + 1] = -0.7;
        out[t + 2] = 0.3;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_noise_builds_finite_normalized_audio() {
        for (i, name) in NAMES.iter().enumerate() {
            let mut out = vec![0.0f32; LEN];
            build(i, &mut out);
            let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(out.iter().all(|v| v.is_finite()), "{name}");
            assert!((peak - 0.9).abs() < 1e-4, "{name} peak {peak}");
        }
    }

    #[test]
    fn loops_are_seamless() {
        for i in 0..NAMES.len() {
            let mut x = vec![0.0f32; LEN];
            build(i, &mut x);
            // the jump from the last sample to the first is no bigger than typical steps
            let typical = x.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
            assert!((x[0] - x[LEN - 1]).abs() <= typical, "{}", NAMES[i]);
        }
    }

    #[test]
    fn coloured_noises_have_their_slopes() {
        // energy per octave band: white rises 3 dB/oct in band energy, pink stays flat, brown falls 3 dB/oct
        let band = |x: &[f32], lo: f64| {
            let fft = RealFftPlanner::<f32>::new().plan_fft_forward(LEN);
            let mut input = x.to_vec();
            let mut s = fft.make_output_vec();
            fft.process(&mut input, &mut s).unwrap();
            let bin = RATE as f64 / LEN as f64;
            s.iter().enumerate().filter(|(k, _)| (*k as f64 * bin) >= lo && (*k as f64 * bin) < lo * 2.0).map(|(_, c)| c.norm_sqr() as f64).sum::<f64>()
        };
        let slope = |i: usize| {
            let mut x = vec![0.0f32; LEN];
            build(i, &mut x);
            10.0 * (band(&x, 4000.0) / band(&x, 250.0)).log10() / 4.0 // dB per octave over 4 octaves
        };
        assert!((slope(0) - 3.0).abs() < 0.5, "white {}", slope(0));
        assert!(slope(1).abs() < 0.5, "pink {}", slope(1));
        assert!((slope(2) + 3.0).abs() < 0.5, "brown {}", slope(2));
    }
}
