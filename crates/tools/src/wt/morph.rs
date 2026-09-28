//! Morphs: fill a table between keyframes. The keys are spread evenly over
//! the new table and the frames between them interpolate:
//!
//! - Crossfade: sample by sample.
//! - Spectral: harmonic amplitudes linearly, phases along the shorter way
//!   round, so a sweep between two timbres keeps its level.
//! - Spectral, fundamental phase zeroed: each key is first shifted in time
//!   so its fundamental starts at phase 0, which lines up waves that differ
//!   only in where their cycle starts.
//! - Spectral, all phases zeroed: amplitudes only (every harmonic in sine
//!   phase), the smoothest morph, at the cost of each key's exact shape.

use wt_dsp::mip::FRAME_LEN;

use super::spectrum::{HARMONICS, Spectra, wrap};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Morph {
    Crossfade,
    Spectral,
    SpectralZeroFundamental,
    SpectralZeroPhases,
}

impl Morph {
    pub fn from_abi(m: u32) -> Option<Morph> {
        [Morph::Crossfade, Morph::Spectral, Morph::SpectralZeroFundamental, Morph::SpectralZeroPhases].get(m as usize).copied()
    }
}

struct Key {
    mag: Vec<f32>,
    ph: Vec<f32>,
}

/// `keys` (k × 2048) spread over `target` frames, the rest interpolated.
pub fn morph(keys: &[f32], target: usize, mode: Morph) -> Vec<f32> {
    let k = keys.len() / FRAME_LEN;
    let target = target.max(k).max(1);
    let mut out = vec![0.0f32; target * FRAME_LEN];
    if k == 0 {
        return out;
    }
    if k == 1 {
        for f in 0..target {
            out[f * FRAME_LEN..(f + 1) * FRAME_LEN].copy_from_slice(&keys[..FRAME_LEN]);
        }
        return out;
    }
    let mut s = Spectra::default();
    let spec: Vec<Key> = if mode == Morph::Crossfade {
        Vec::new()
    } else {
        (0..k)
            .map(|i| {
                let mut mag = vec![0.0; HARMONICS + 1];
                let mut ph = vec![0.0; HARMONICS + 1];
                s.analyze(&keys[i * FRAME_LEN..(i + 1) * FRAME_LEN], &mut mag, &mut ph);
                match mode {
                    Morph::SpectralZeroFundamental => {
                        // a time shift that puts the fundamental at phase 0 moves harmonic h by h·φ1
                        let p1 = ph[1];
                        for (h, p) in ph.iter_mut().enumerate().skip(1) {
                            *p = wrap(*p - h as f32 * p1);
                        }
                    }
                    Morph::SpectralZeroPhases => ph.fill(0.0),
                    _ => {}
                }
                Key { mag, ph }
            })
            .collect()
    };
    let pos = |i: usize| (i as f64 * (target - 1) as f64 / (k - 1) as f64).round() as usize;
    let mut mag = vec![0.0f32; HARMONICS + 1];
    let mut ph = vec![0.0f32; HARMONICS + 1];
    for seg in 0..k - 1 {
        let (a, b) = (pos(seg), pos(seg + 1));
        for f in a..=b {
            let t = if b > a { (f - a) as f32 / (b - a) as f32 } else { 0.0 };
            let dst = &mut out[f * FRAME_LEN..(f + 1) * FRAME_LEN];
            if mode == Morph::Crossfade {
                let (x, y) = (&keys[seg * FRAME_LEN..(seg + 1) * FRAME_LEN], &keys[(seg + 1) * FRAME_LEN..(seg + 2) * FRAME_LEN]);
                for n in 0..FRAME_LEN {
                    dst[n] = x[n] + (y[n] - x[n]) * t;
                }
                continue;
            }
            let (x, y) = (&spec[seg], &spec[seg + 1]);
            for h in 0..=HARMONICS {
                mag[h] = x.mag[h] + (y.mag[h] - x.mag[h]) * t;
                // interpolate the phase toward whichever key is louder at this harmonic
                // (a silent harmonic's phase means nothing)
                ph[h] = if x.mag[h] < 1e-7 {
                    y.ph[h]
                } else if y.mag[h] < 1e-7 {
                    x.ph[h]
                } else {
                    wrap(x.ph[h] + wrap(y.ph[h] - x.ph[h]) * t)
                };
            }
            s.synthesize(&mag, &ph, dst);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn frame(f: impl Fn(f32) -> f32) -> Vec<f32> {
        (0..FRAME_LEN).map(|i| f(i as f32 / FRAME_LEN as f32)).collect()
    }
    fn analyse(f: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let mut s = Spectra::default();
        let mut m = vec![0.0; HARMONICS + 1];
        let mut p = vec![0.0; HARMONICS + 1];
        s.analyze(f, &mut m, &mut p);
        (m, p)
    }

    #[test]
    fn spectral_morph_interpolates_the_amplitudes_to_a_tenth_of_a_db() {
        // a saw into a square-ish odd-harmonic wave, with different phases
        let a = frame(|t| 2.0 * t - 1.0);
        let b = frame(|t| (TAU * t).sin() + (TAU * 3.0 * t + 1.0).sin() / 3.0 + (TAU * 5.0 * t + 2.0).sin() / 5.0);
        let keys = [a.clone(), b.clone()].concat();
        let out = morph(&keys, 9, Morph::Spectral);
        assert_eq!(out.len(), 9 * FRAME_LEN);
        let (ma, _) = analyse(&a);
        let (mb, _) = analyse(&b);
        for f in 0..9 {
            let t = f as f32 / 8.0;
            let (m, _) = analyse(&out[f * FRAME_LEN..(f + 1) * FRAME_LEN]);
            for h in 1..=40 {
                let want = ma[h] + (mb[h] - ma[h]) * t;
                if want > 1e-4 {
                    let db = 20.0 * (m[h] / want).log10();
                    assert!(db.abs() < 0.1, "frame {f} harmonic {h}: {db} dB");
                }
            }
        }
        // the keys come through unchanged
        assert!(out[..FRAME_LEN].iter().zip(&a).all(|(x, y)| (x - y).abs() < 1e-4));
        assert!(out[8 * FRAME_LEN..].iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-4));
    }

    #[test]
    fn crossfade_is_sample_by_sample_and_zero_phase_modes_align() {
        let a = frame(|t| (TAU * t).sin());
        let b = frame(|t| (TAU * t + 2.0).sin() * 0.5);
        let keys = [a.clone(), b.clone()].concat();
        let x = morph(&keys, 3, Morph::Crossfade);
        assert!((x[FRAME_LEN + 100] - (a[100] + b[100]) / 2.0).abs() < 1e-6);
        // zeroing the fundamental's phase lines the two sines up: the middle frame is a 0.75 sine at phase 0
        let z = morph(&keys, 3, Morph::SpectralZeroFundamental);
        let (m, p) = analyse(&z[FRAME_LEN..2 * FRAME_LEN]);
        assert!((m[1] - 0.75).abs() < 1e-4 && p[1].abs() < 1e-3, "{} {}", m[1], p[1]);
        let zp = morph(&keys, 3, Morph::SpectralZeroPhases);
        let (m, p) = analyse(&zp[..FRAME_LEN]);
        assert!((m[1] - 1.0).abs() < 1e-4 && p[1].abs() < 1e-3);
    }

    #[test]
    fn keys_spread_over_the_table() {
        let keys: Vec<f32> = (0..3).flat_map(|i| vec![i as f32; FRAME_LEN]).collect();
        let out = morph(&keys, 5, Morph::Crossfade);
        let at = |f: usize| out[f * FRAME_LEN + 7];
        assert_eq!([at(0), at(1), at(2), at(3), at(4)], [0.0, 0.5, 1.0, 1.5, 2.0]);
    }
}
