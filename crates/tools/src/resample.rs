//! Resample a single cycle of any length to the 2048-sample frame size,
//! band-limited through the FFT (exact for periodic material).

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;
use wt_dsp::mip::FRAME_LEN;

/// Resample one cycle (`src`, any length ≥ 2) to FRAME_LEN samples.
pub fn cycle_to_frame(src: &[f32], dst: &mut [f32]) {
    assert!(src.len() >= 2);
    assert_eq!(dst.len(), FRAME_LEN);
    if src.len() == FRAME_LEN {
        dst.copy_from_slice(src);
        return;
    }
    let n = src.len();
    let mut planner = RealFftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(FRAME_LEN);
    let mut input = src.to_vec();
    let mut spec = fwd.make_output_vec();
    fwd.process(&mut input, &mut spec).expect("forward fft");
    let mut out_spec = vec![Complex::new(0.0f32, 0.0); FRAME_LEN / 2 + 1];
    // keep every partial both sizes can hold; a partial exactly at the source's
    // Nyquist is split between ±, so it counts half
    let keep = (n / 2).min(FRAME_LEN / 2);
    let scale = 1.0 / n as f32;
    for k in 0..=keep {
        let mut c = spec[k] * scale;
        if n.is_multiple_of(2) && k == n / 2 && k < FRAME_LEN / 2 {
            c *= 0.5;
        }
        out_spec[k] = c;
    }
    out_spec[0].im = 0.0;
    out_spec[FRAME_LEN / 2].im = 0.0;
    inv.process(&mut out_spec, dst).expect("inverse fft");
}

/// Split `samples` into consecutive cycles of `frame_size` and resample each to FRAME_LEN.
/// Returns the frames (count × FRAME_LEN), at most `max_frames`.
pub fn split_frames(samples: &[f32], frame_size: usize, max_frames: usize) -> Vec<f32> {
    let count = (samples.len() / frame_size).clamp(1, max_frames);
    let mut out = vec![0.0f32; count * FRAME_LEN];
    for i in 0..count {
        let a = i * frame_size;
        let b = (a + frame_size).min(samples.len());
        let mut cyc = samples[a..b].to_vec();
        cyc.resize(frame_size, 0.0);
        cycle_to_frame(&cyc, &mut out[i * FRAME_LEN..(i + 1) * FRAME_LEN]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_a_sine_keeps_its_shape() {
        for &n in &[600usize, 256, 4096, 2047] {
            let src: Vec<f32> = (0..n).map(|i| (std::f32::consts::TAU * 3.0 * i as f32 / n as f32).sin()).collect();
            let mut dst = vec![0.0; FRAME_LEN];
            cycle_to_frame(&src, &mut dst);
            for (i, &v) in dst.iter().enumerate().step_by(5) {
                let want = (std::f32::consts::TAU * 3.0 * i as f32 / FRAME_LEN as f32).sin();
                assert!((v - want).abs() < 1e-4, "n {n} i {i}: {v} vs {want}");
            }
        }
    }

    #[test]
    fn splits_into_frames() {
        let samples: Vec<f32> = (0..2048 * 3).map(|i| ((i % 2048) as f32 / 2048.0) - 0.5).collect();
        let f = split_frames(&samples, 2048, 256);
        assert_eq!(f.len(), 3 * FRAME_LEN);
        assert_eq!(&f[..FRAME_LEN], &samples[..FRAME_LEN]);
    }
}
