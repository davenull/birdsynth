//! The analysis the engine's Spectral type plays (layout and meaning in
//! crates/engine/src/osc/spectral.rs): Hann frames of SIZE every HOP,
//! centred on multiples of HOP from PRE frames before time 0, as magnitudes,
//! phases and a flag on the frame at each transient.

use realfft::RealFftPlanner;

use crate::onsets::onsets;

pub const SIZE: usize = 2048;
pub const HOP: usize = 512;
pub const PRE: usize = 2;
pub const BINS: usize = SIZE / 2 + 1;
pub const FRAME_FLOATS: usize = 2 * BINS + 1;
/// The longest recording analysed (longer ones are cut): about 40 s at 48 kHz.
pub const MAX_FRAMES: usize = 4000;

pub fn frames(len: usize) -> usize {
    (PRE + len.div_ceil(HOP) + 1).min(MAX_FRAMES)
}

/// Analyse mono `x` into `out` (frames(x.len()) × FRAME_FLOATS floats).
pub fn analyze(x: &[f32], rate: f32, out: &mut [f32]) {
    let n = frames(x.len());
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(SIZE);
    let win: Vec<f32> = (0..SIZE).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / SIZE as f32).cos()).collect();
    let mut buf = vec![0.0f32; SIZE];
    let mut spec = fft.make_output_vec();
    for m in 0..n {
        let c = (m as isize - PRE as isize) * HOP as isize;
        for (i, v) in buf.iter_mut().enumerate() {
            let j = c - (SIZE / 2) as isize + i as isize;
            *v = if j >= 0 && (j as usize) < x.len() { x[j as usize] * win[i] } else { 0.0 };
        }
        fft.process(&mut buf, &mut spec).expect("fft");
        let f = &mut out[m * FRAME_FLOATS..(m + 1) * FRAME_FLOATS];
        for k in 0..BINS {
            f[k] = spec[k].norm();
            f[BINS + k] = spec[k].im.atan2(spec[k].re);
        }
        f[2 * BINS] = 0.0;
    }
    // mark the first frame centred at or after each transient
    for t in onsets(x, rate) {
        let m = PRE + (t as usize).div_ceil(HOP);
        if m < n {
            out[m * FRAME_FLOATS + 2 * BINS] = 1.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wt_engine::spec::protocol as proto;

    #[test]
    fn matches_the_engine_layout() {
        assert_eq!((SIZE, HOP, PRE), (proto::SPEC_SIZE, proto::SPEC_HOP, proto::SPEC_PRE));
        let x: Vec<f32> = (0..10_000).map(|i| if i >= 5000 { (i as f32 * 0.3).sin() } else { 0.0 }).collect();
        let mut out = vec![0.0; frames(x.len()) * FRAME_FLOATS];
        analyze(&x, 48_000.0, &mut out);
        let flagged: Vec<usize> = (0..frames(x.len())).filter(|&m| out[m * FRAME_FLOATS + 2 * BINS] >= 0.5).collect();
        assert_eq!(flagged, vec![PRE + 10], "the frame centred at 5120, just after the tone starts");
    }
}
