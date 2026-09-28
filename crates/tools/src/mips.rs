//! Band-limit single-cycle frames into the engine's mip layout
//! (see `wt_dsp::mip`): one FFT per frame, then one inverse FFT per level
//! with the harmonics above that level's limit removed.

use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use wt_dsp::mip::{FRAME_LEN, FRAME_STRIDE, LEVELS, LEVEL_OFFSET, harmonics, level_len};

pub struct MipBuilder {
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Vec<Arc<dyn ComplexToReal<f32>>>,
    input: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    work: Vec<Complex<f32>>,
    out: Vec<f32>,
}

impl Default for MipBuilder {
    fn default() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(FRAME_LEN);
        let inv = (0..LEVELS).map(|l| planner.plan_fft_inverse(level_len(l))).collect();
        MipBuilder {
            input: vec![0.0; FRAME_LEN],
            spectrum: fwd.make_output_vec(),
            fwd,
            inv,
            work: vec![Complex::new(0.0, 0.0); FRAME_LEN / 2 + 1],
            out: vec![0.0; FRAME_LEN],
        }
    }
}

impl MipBuilder {
    /// Band-limit one 2048-sample frame into its FRAME_STRIDE block.
    pub fn frame(&mut self, src: &[f32], dst: &mut [f32]) {
        assert_eq!(src.len(), FRAME_LEN);
        assert_eq!(dst.len(), FRAME_STRIDE);
        self.input.copy_from_slice(src);
        self.fwd.process(&mut self.input, &mut self.spectrum).expect("forward fft");
        // An unnormalized inverse of size M, fed bins scaled by 1/2048, returns
        // each partial at its original amplitude whatever M is.
        let scale = 1.0 / FRAME_LEN as f32;
        for l in 0..LEVELS {
            let m = level_len(l);
            let h = harmonics(l);
            let bins = m / 2 + 1;
            let work = &mut self.work[..bins];
            for (k, w) in work.iter_mut().enumerate() {
                *w = if k <= h { self.spectrum[k] * scale } else { Complex::new(0.0, 0.0) };
            }
            work[0].im = 0.0;
            work[bins - 1].im = 0.0;
            let out = &mut self.out[..m];
            self.inv[l].process(work, out).expect("inverse fft");
            let o = LEVEL_OFFSET[l];
            dst[o..o + m].copy_from_slice(out);
            dst[o + m] = out[0];
        }
    }

    /// Band-limit `n` frames (n × 2048 samples) into n × FRAME_STRIDE floats.
    pub fn table(&mut self, frames: &[f32], out: &mut [f32]) {
        let n = frames.len() / FRAME_LEN;
        assert_eq!(out.len(), n * FRAME_STRIDE);
        for i in 0..n {
            self.frame(&frames[i * FRAME_LEN..(i + 1) * FRAME_LEN], &mut out[i * FRAME_STRIDE..(i + 1) * FRAME_STRIDE]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wt_dsp::{mip, saw};

    fn naive_saw() -> Vec<f32> {
        // rising 0 -> 1, drop to -1, rising to 0: the built-in saw's phase
        (0..FRAME_LEN).map(|i| {
            let p = i as f32 / FRAME_LEN as f32 + 0.5;
            2.0 * (p - p.floor()) - 1.0
        }).collect()
    }

    #[test]
    fn rebuilds_the_additive_saw_levels() {
        // Feed the additive saw's full-band level in; every band-limited level
        // must come out equal to the additive saw's own level.
        let add = saw::saw_frame();
        let mut b = MipBuilder::default();
        let mut out = vec![0.0; FRAME_STRIDE];
        b.frame(&add[..FRAME_LEN], &mut out);
        for l in 0..LEVELS {
            let o = LEVEL_OFFSET[l];
            for i in 0..=level_len(l) {
                assert!((out[o + i] - add[o + i]).abs() < 1e-4, "level {l} sample {i}: {} vs {}", out[o + i], add[o + i]);
            }
        }
    }

    #[test]
    fn levels_contain_nothing_above_their_limit() {
        let mut b = MipBuilder::default();
        let mut out = vec![0.0; FRAME_STRIDE];
        b.frame(&naive_saw(), &mut out);
        let mut planner = RealFftPlanner::<f32>::new();
        for l in 0..LEVELS {
            let m = level_len(l);
            let fft = planner.plan_fft_forward(m);
            let mut x = out[LEVEL_OFFSET[l]..LEVEL_OFFSET[l] + m].to_vec();
            let mut s = fft.make_output_vec();
            fft.process(&mut x, &mut s).unwrap();
            let fund = s[1].norm();
            for (k, c) in s.iter().enumerate().skip(harmonics(l) + 1) {
                assert!(c.norm() < fund * 1e-5, "level {l} bin {k}: {} vs fundamental {fund}", c.norm());
            }
            // and the fundamental keeps its amplitude at every level
            let ref_fund = {
                let fft0 = planner.plan_fft_forward(FRAME_LEN);
                let mut x0 = naive_saw();
                let mut s0 = fft0.make_output_vec();
                fft0.process(&mut x0, &mut s0).unwrap();
                s0[1].norm() / FRAME_LEN as f32
            };
            assert!((fund / m as f32 - ref_fund).abs() < 1e-4, "level {l}");
        }
        assert_eq!(out[mip::LEVEL_OFFSET[3] + level_len(3)], out[mip::LEVEL_OFFSET[3]]);
    }
}
