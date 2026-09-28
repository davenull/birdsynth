//! Factory impulse responses for the convolver, all generated here (no
//! recordings): rooms and halls from shaped noise with early reflections, a
//! plate, a dispersive spring, a speaker cabinet and a telephone from
//! resonances and band limits, and a few special spaces. Every response is
//! normalized to unit energy, so switching between them keeps the level.

use std::f32::consts::TAU;

use wt_dsp::rng::Rng;

/// In the order of `fx.convolve.*.ir` (params/fx/convolve.toml); "User File" follows.
pub const NAMES: [&str; 12] = ["Small Room", "Studio", "Hall", "Cathedral", "Plate", "Spring", "Cabinet", "Telephone", "Reverse", "Air", "Metal Box", "Tunnel"];

/// Length in seconds.
const SECONDS: [f32; 12] = [0.45, 0.8, 2.6, 3.9, 2.2, 2.6, 0.04, 0.03, 1.8, 0.45, 1.4, 3.2];

pub fn taps(index: usize, sr: f32) -> usize {
    (SECONDS[index.min(NAMES.len() - 1)] * sr) as usize
}

/// A decaying noise tail: RT60 `rt`, the brightness falling from `bright`
/// Hz toward `dark` Hz over the decay, after a `gap` in seconds.
#[allow(clippy::too_many_arguments)]
fn tail(out: &mut [f32], sr: f32, rng: &mut Rng, rt: f32, bright: f32, dark: f32, gap: f32, gain: f32) {
    let mut lp = 0.0f32;
    let n0 = (gap * sr) as usize;
    for (i, v) in out.iter_mut().enumerate().skip(n0) {
        let t = (i - n0) as f32 / sr;
        let env = 10f32.powf(-3.0 * t / rt);
        let fc = dark + (bright - dark) * (-t / (0.35 * rt)).exp();
        let a = 1.0 - (-TAU * fc / sr).exp();
        lp += ((rng.next_f32() * 2.0 - 1.0) - lp) * a;
        // a short fade-in keeps the start of the tail from clicking
        let rise = (t / 0.004).min(1.0);
        *v += gain * env * lp * rise;
    }
}

/// Early reflections: `count` taps spread over `span` seconds.
fn early(out: &mut [f32], sr: f32, rng: &mut Rng, count: usize, span: f32, gain: f32) {
    for k in 0..count {
        let t = span * (k as f32 + rng.next_f32()) / count as f32;
        let i = (t * sr) as usize;
        if i < out.len() {
            let sign = if rng.next_f32() < 0.5 { -1.0 } else { 1.0 };
            out[i] += sign * gain * (1.0 - 0.7 * k as f32 / count as f32);
        }
    }
}

/// A damped sine at `f` Hz with decay time `decay` (to -60 dB).
fn mode(out: &mut [f32], sr: f32, f: f32, decay: f32, gain: f32, phase: f32) {
    for (i, v) in out.iter_mut().enumerate() {
        let t = i as f32 / sr;
        *v += gain * 10f32.powf(-3.0 * t / decay) * (TAU * f * t + phase).sin();
    }
}

fn normalize(l: &mut [f32], r: &mut [f32]) {
    let e: f64 = l.iter().chain(r.iter()).map(|&v| (v as f64) * (v as f64)).sum();
    if e > 0.0 {
        let g = (2.0 / e).sqrt() as f32;
        l.iter_mut().chain(r.iter_mut()).for_each(|v| *v *= g);
    }
}

/// Build response `index` at sample rate `sr` into `l` and `r` (`taps(index, sr)` each).
pub fn build(index: usize, sr: f32, l: &mut [f32], r: &mut [f32]) {
    l.fill(0.0);
    r.fill(0.0);
    let index = index.min(NAMES.len() - 1);
    for (ch, out) in [&mut *l, &mut *r].into_iter().enumerate() {
        let mut rng = Rng::new(0x1234_5000 + index as u32 * 16 + ch as u32);
        match index {
            // rooms and halls: early reflections, then a darkening tail
            0 => {
                early(out, sr, &mut rng, 10, 0.03, 0.5);
                tail(out, sr, &mut rng, 0.4, 9000.0, 3000.0, 0.008, 0.25);
            }
            1 => {
                early(out, sr, &mut rng, 12, 0.045, 0.45);
                tail(out, sr, &mut rng, 0.7, 12000.0, 4000.0, 0.012, 0.22);
            }
            2 => {
                early(out, sr, &mut rng, 14, 0.08, 0.35);
                tail(out, sr, &mut rng, 2.3, 9000.0, 1800.0, 0.025, 0.15);
            }
            3 => {
                early(out, sr, &mut rng, 16, 0.12, 0.3);
                tail(out, sr, &mut rng, 3.5, 7000.0, 1200.0, 0.04, 0.13);
            }
            // plate: no early reflections, bright and dense from the start
            4 => tail(out, sr, &mut rng, 2.0, 16000.0, 6000.0, 0.001, 0.2),
            // spring: dispersive chirps repeating at the spring's round trip
            5 => {
                let trip = 0.034 + 0.003 * ch as f32;
                let mut amp = 0.6;
                let mut t0 = 0.004;
                while t0 < SECONDS[5] {
                    let i0 = (t0 * sr) as usize;
                    let len = (0.012 * sr) as usize;
                    let mut ph = 0.0f32;
                    for j in 0..len {
                        let u = j as f32 / len as f32;
                        let f = 4500.0 * (1.0 - u) + 180.0 * u; // falling chirp
                        ph += f / sr;
                        if i0 + j < out.len() {
                            out[i0 + j] += amp * (1.0 - u) * (TAU * ph).sin();
                        }
                    }
                    t0 += trip;
                    amp *= 0.72;
                }
                tail(out, sr, &mut rng, 2.2, 5000.0, 1500.0, 0.004, 0.03);
            }
            // speaker cabinet: a few body resonances and a high cut
            6 => {
                for (f, d, g) in [(110.0f32, 0.03f32, 0.8f32), (190.0, 0.02, 0.6), (650.0, 0.012, 0.5), (1700.0, 0.008, 0.45), (3300.0, 0.005, 0.3)] {
                    mode(out, sr, f * (1.0 + 0.01 * ch as f32), d, g, rng.next_f32() * TAU);
                }
            }
            // telephone: a band-limited click (300 Hz to 3.4 kHz)
            7 => {
                let len = out.len();
                let c = len / 4;
                for (i, v) in out.iter_mut().enumerate() {
                    let n = i as f32 - c as f32;
                    let sinc = |f: f32| if n == 0.0 { 2.0 * f / sr } else { (TAU * f * n / sr).sin() / (std::f32::consts::PI * n) };
                    let w = 0.5 - 0.5 * (TAU * i as f32 / len as f32).cos();
                    *v = (sinc(3400.0) - sinc(300.0)) * w;
                }
            }
            // reverse: a hall played backwards, swelling into the next note
            8 => {
                tail(out, sr, &mut rng, 1.8, 9000.0, 2500.0, 0.0, 0.2);
                out.reverse();
            }
            // air: a short, very bright wash
            9 => tail(out, sr, &mut rng, 0.35, 18000.0, 12000.0, 0.002, 0.3),
            // metal box: inharmonic ringing modes
            10 => {
                for (k, f) in [312.0f32, 487.0, 733.0, 1049.0, 1401.0, 1873.0, 2511.0, 3307.0].iter().enumerate() {
                    mode(out, sr, f * (1.0 + 0.004 * ch as f32), 1.3 - 0.1 * k as f32, 0.3, rng.next_f32() * TAU);
                }
                tail(out, sr, &mut rng, 0.8, 8000.0, 3000.0, 0.001, 0.05);
            }
            // tunnel: flutter echoes down a long tube
            _ => {
                let period = 0.058 + 0.004 * ch as f32;
                let mut amp = 0.5;
                let mut t0 = 0.01;
                while t0 < SECONDS[11] {
                    let i = (t0 * sr) as usize;
                    for j in 0..6 {
                        if i + j < out.len() {
                            out[i + j] += amp * (1.0 - j as f32 / 6.0);
                        }
                    }
                    t0 += period;
                    amp *= 0.83;
                }
                tail(out, sr, &mut rng, 3.0, 5000.0, 1500.0, 0.01, 0.05);
            }
        }
    }
    normalize(l, r);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_response_is_finite_and_normalized() {
        for i in 0..NAMES.len() {
            let n = taps(i, 48_000.0);
            let (mut l, mut r) = (vec![0.0; n], vec![0.0; n]);
            build(i, 48_000.0, &mut l, &mut r);
            let e: f64 = l.iter().chain(r.iter()).map(|&v| (v as f64).powi(2)).sum();
            assert!(l.iter().chain(r.iter()).all(|v| v.is_finite()), "{} has non-finite taps", NAMES[i]);
            assert!((e - 2.0).abs() < 1e-3, "{} energy {e}", NAMES[i]);
            assert!(n <= wt_dsp::conv::MAX_TAPS, "{} is longer than the convolver allows", NAMES[i]);
        }
    }
}
