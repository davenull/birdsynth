//! Preview one cycle of a frame as the oscillator would play it through its
//! warps, for the 2D wavetable display. It uses the engine's own warp math.

use wt_dsp::mip::FRAME_LEN;
use wt_dsp::warp;

/// Render `out.len()` points of one cycle of `frame` (FRAME_LEN samples,
/// linear interpolation) through warp 1 then warp 2.
pub fn cycle(frame: &[f32], w1: (u8, f32), w2: (u8, f32), out: &mut [f32]) {
    assert_eq!(frame.len(), FRAME_LEN);
    let n = out.len();
    for (i, v) in out.iter_mut().enumerate() {
        let p = i as f32 / n as f32;
        let (p1, g1) = warp::apply(w1.0, p, w1.1);
        let (p2, g2) = warp::apply(w2.0, warp::wrap01(p1), w2.1);
        let x = warp::wrap01(p2) * FRAME_LEN as f32;
        let i0 = (x as usize).min(FRAME_LEN - 1);
        let t = x - i0 as f32;
        let a = frame[i0];
        let b = frame[(i0 + 1) % FRAME_LEN];
        *v = (a + (b - a) * t) * g1 * g2;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwarped_preview_is_the_frame() {
        let frame: Vec<f32> = (0..FRAME_LEN).map(|i| (i as f32 / FRAME_LEN as f32) * 2.0 - 1.0).collect();
        let mut out = vec![0.0; 256];
        cycle(&frame, (warp::OFF, 0.0), (warp::OFF, 0.0), &mut out);
        for (i, v) in out.iter().enumerate() {
            assert!((v - frame[i * 8]).abs() < 1e-6);
        }
    }

    #[test]
    fn sync_preview_repeats_the_cycle() {
        let frame: Vec<f32> = (0..FRAME_LEN).map(|i| (std::f32::consts::TAU * i as f32 / FRAME_LEN as f32).sin()).collect();
        let mut out = vec![0.0; 512];
        cycle(&frame, (warp::SYNC, 1.0 / 15.0), (warp::OFF, 0.0), &mut out); // ratio 2
        assert!((out[64] - out[64 + 256]).abs() < 1e-4);
        assert!((out[64] - (std::f32::consts::TAU * 0.25).sin()).abs() < 1e-3);
    }
}
