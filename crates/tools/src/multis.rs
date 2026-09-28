//! birdsynth's factory multisamples, played by physical models (ours): a
//! plucked string, a marimba bar and a tine piano. Each has a zone per
//! octave from C1 to C7 and two velocity layers, packed the way the engine
//! reads multisamples (see crates/engine/src/osc/multi.rs).

use std::f64::consts::TAU;

use wt_dsp::rng::Rng;

pub const ZONE_FLOATS: usize = 16;
pub const NAMES: &[&str] = &["Plucked String", "Marimba", "Tine Piano"];
const ROOTS: [u8; 7] = [24, 36, 48, 60, 72, 84, 96];
const SECS: f64 = 2.0;

fn hz(note: f64) -> f64 {
    440.0 * 2f64.powf((note - 69.0) / 12.0)
}

/// A plucked string: Karplus-Strong with a pick-position comb and a
/// fractional-delay allpass for exact tuning.
fn pluck(f0: f64, sr: f64, hard: bool, rng: &mut Rng, out: &mut [f32]) {
    let l = sr / f0;
    // loop delay: n samples, 0.5 from the averaging filter, the rest from the allpass
    let n = ((l - 0.5 - 0.1).floor() as usize).max(2);
    let frac = l - 0.5 - n as f64;
    let c = (1.0 - frac) / (1.0 + frac);
    let t60 = (5.0 * (110.0 / f0).powf(0.5)).clamp(0.8, 8.0);
    let g = 10f64.powf(-3.0 / (f0 * t60));
    let bright = if hard { 0.95 } else { 0.45 };
    // excitation: noise, low-passed for a soft pick, notched at the pick position
    let mut exc = vec![0.0f64; n];
    let mut lp = 0.0;
    for e in exc.iter_mut() {
        let w = rng.next_f32() as f64 * 2.0 - 1.0;
        lp += (w - lp) * bright;
        *e = lp;
    }
    let p = ((0.13 * n as f64) as usize).max(1);
    let exc: Vec<f64> = (0..n).map(|i| exc[i] - if i >= p { exc[i - p] } else { 0.0 }).collect();
    let mut line = exc;
    let mut prev = 0.0f64;
    let (mut ap_x, mut ap_y) = (0.0f64, 0.0f64);
    let mut idx = 0;
    for o in out.iter_mut() {
        let y = line[idx];
        let avg = 0.5 * (y + prev) * g;
        prev = y;
        // first-order allpass for the fractional part
        let a = c * avg + ap_x - c * ap_y;
        ap_x = avg;
        ap_y = a;
        line[idx] = a;
        idx = (idx + 1) % n;
        *o = y as f32;
    }
}

/// A marimba bar: its first modes (tuned about 1 : 4 : 10), each ringing
/// down at its own rate, struck by a soft or a hard mallet.
fn marimba(f0: f64, sr: f64, hard: bool, rng: &mut Rng, out: &mut [f32]) {
    let modes = [(1.0, 1.0, 1.6), (3.99, if hard { 0.5 } else { 0.18 }, 0.45), (10.65, if hard { 0.25 } else { 0.05 }, 0.16), (18.4, if hard { 0.1 } else { 0.01 }, 0.07)];
    let scale = (220.0 / f0).powf(0.3);
    let mut click = 0.0f64;
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f64 / sr;
        let mut v = 0.0;
        for &(r, a, t60) in &modes {
            let f = f0 * r;
            if f < sr * 0.45 {
                v += a * (TAU * f * t).sin() * (-6.9 * t / (t60 * scale)).exp();
            }
        }
        // the mallet's contact: a millisecond or two of filtered noise
        if t < 0.003 {
            click += ((rng.next_f32() as f64 * 2.0 - 1.0) - click) * if hard { 0.6 } else { 0.2 };
            v += click * (1.0 - t / 0.003) * if hard { 0.3 } else { 0.1 };
        }
        *o = (v * (1.0 - (-t / 0.0004).exp())) as f32;
    }
}

/// A tine piano: a ringing tine and its bell-like overtone, through a
/// pickup that adds even harmonics the harder it's driven.
fn tine(f0: f64, sr: f64, hard: bool, _rng: &mut Rng, out: &mut [f32]) {
    let drive = if hard { 0.9 } else { 0.35 };
    let decay = (3.0 * (220.0 / f0).powf(0.4)).clamp(0.6, 6.0);
    for (i, o) in out.iter_mut().enumerate() {
        let t = i as f64 / sr;
        let mut x = (TAU * f0 * t).sin() * (-t / decay).exp();
        if f0 * 7.03 < sr * 0.45 {
            x += 0.35 * drive * (TAU * f0 * 7.03 * t).sin() * (-t / 0.06).exp();
        }
        let y = x + drive * 0.5 * x * x;
        *o = (y * (1.0 - (-t / 0.0008).exp())) as f32;
    }
}

/// Floats the packed instrument takes at `sr`.
pub fn floats(sr: f32) -> usize {
    let n = (SECS * sr as f64) as usize;
    ROOTS.len() * 2 * (ZONE_FLOATS + n)
}

/// Pack instrument `i` into `out`; returns the zone count.
pub fn build(i: usize, sr: f32, out: &mut [f32]) -> usize {
    let n = (SECS * sr as f64) as usize;
    let zones = ROOTS.len() * 2;
    let head = zones * ZONE_FLOATS;
    let mut rng = Rng::new(1234 + i as u32);
    let model: fn(f64, f64, bool, &mut Rng, &mut [f32]) = match i {
        0 => pluck,
        1 => marimba,
        _ => tine,
    };
    for (r, &root) in ROOTS.iter().enumerate() {
        let lo = if r == 0 { 0 } else { root - 6 };
        let hi = if r + 1 == ROOTS.len() { 127 } else { root + 5 };
        // hard first, so both layers share its level: soft stays softer
        let mut peak = 0.0f32;
        for (layer, hard) in [(1usize, true), (0, false)] {
            let z = r * 2 + layer;
            let a = head + z * n;
            let audio = &mut out[a..a + n];
            model(hz(root as f64), sr as f64, hard, &mut rng, audio);
            if hard {
                peak = audio.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
            }
            let g = if hard { 0.9 } else { 0.9 * 0.55 };
            let p = if hard { peak } else { audio.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9) };
            audio.iter_mut().for_each(|v| *v *= g / p);
            let (vlo, vhi) = if hard { (80.0, 127.0) } else { (0.0, 79.0) };
            let h = &mut out[z * ZONE_FLOATS..(z + 1) * ZONE_FLOATS];
            h.copy_from_slice(&[(z * n) as f32, n as f32, 1.0, sr, root as f32, lo as f32, hi as f32, vlo, vhi, 0.0, 0.0, n as f32, 1.0, 0.0, 1.0, 0.0]);
        }
    }
    zones
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruments_are_in_tune_and_levelled() {
        let sr = 48_000.0f32;
        let n = (SECS * sr as f64) as usize;
        let mut out = vec![0.0; floats(sr)];
        for i in 0..NAMES.len() {
            let zones = build(i, sr, &mut out);
            assert_eq!(zones, 14);
            let head = zones * ZONE_FLOATS;
            for z in 0..zones {
                let h = &out[z * ZONE_FLOATS..(z + 1) * ZONE_FLOATS];
                let audio = &out[head + h[0] as usize..head + h[0] as usize + n];
                assert!(audio.iter().all(|v| v.is_finite() && v.abs() <= 0.91), "{} zone {z}", NAMES[i]);
                // the string and the bar are pitched: check the middle of the range
                let root = h[4] as f64;
                if i < 2 && (36.0..=84.0).contains(&root) {
                    let f = crate::wt::import::detect_pitch(&audio[2400..40_000], sr).expect("pitched") as f64;
                    let cents = 1200.0 * (f / hz(root)).log2();
                    assert!(cents.abs() < 5.0, "{} root {root}: {cents:+.2} cents", NAMES[i]);
                }
            }
            // key ranges cover the keyboard without gaps
            let mut covered = [false; 128];
            for z in (0..zones).step_by(2) {
                let h = &out[z * ZONE_FLOATS..];
                for k in h[5] as usize..=h[6] as usize {
                    covered[k] = true;
                }
            }
            assert!(covered.iter().all(|&c| c));
        }
    }
}
