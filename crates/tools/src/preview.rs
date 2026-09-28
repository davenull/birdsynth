//! Preview one cycle of a frame as the oscillator plays it through its
//! warps: for the wavetable display, and as the reference the engine's
//! kernels are tested against. It runs the same chain from the same shared
//! code: phase warps, the read (from the mip-mapped block, with the read
//! warps), then the shape warps.

use wt_dsp::mip::{FRAME_LEN, FRAME_STRIDE, Pick};
use wt_dsp::warp::{self, Remap};

use crate::mips::MipBuilder;

/// Render `out.len()` points of one cycle of a mip-mapped frame block, read
/// with `pick`, through warp 1 then warp 2.
pub fn cycle_block(block: &[f32], pick: Pick, w1: (u8, f32), w2: (u8, f32), remap: &Remap, out: &mut [f32]) {
    assert_eq!(block.len(), FRAME_STRIDE);
    let read = warp::read_mode(w1.0, w1.1, w2.0, w2.1, pick);
    let n = out.len();
    for (i, v) in out.iter_mut().enumerate() {
        let p = i as f32 / n as f32;
        let (p1, g1) = warp::apply(w1.0, p, w1.1, remap);
        let (p2, g2) = warp::apply(w2.0, warp::wrap01(p1), w2.1, remap);
        let q = warp::wrap01(p2);
        let s = warp::read(block, pick, read, q, [w1.1, w2.1]);
        let y = s * (g1 * g2);
        *v = warp::shape(w2.0, warp::shape(w1.0, y, w1.1, remap), w2.1, remap);
    }
}

/// Preview a raw frame (FRAME_LEN samples) at full detail.
pub fn cycle(frame: &[f32], w1: (u8, f32), w2: (u8, f32), remap: &Remap, out: &mut [f32]) {
    assert_eq!(frame.len(), FRAME_LEN);
    let mut block = vec![0.0; FRAME_STRIDE];
    MipBuilder::default().frame(frame, &mut block);
    cycle_block(&block, Pick { lo: 0, hi: 0, w: 0.0 }, w1, w2, remap, out);
}

#[cfg(test)]
mod tests {
    use super::*;
    use wt_dsp::mip;
    use wt_engine::osc::kernel::{self, Kernel};
    use wt_engine::osc::unison::MAX_LANES;

    fn rich_frame() -> Vec<f32> {
        (0..FRAME_LEN)
            .map(|i| {
                let x = i as f32 / FRAME_LEN as f32;
                0.6 * (2.0 * x - 1.0) + 0.3 * (std::f32::consts::TAU * 3.0 * x).sin()
            })
            .collect()
    }

    #[test]
    fn unwarped_preview_is_the_frame() {
        let frame = rich_frame();
        let mut out = vec![0.0; 256];
        cycle(&frame, (warp::OFF, 0.0), (warp::OFF, 0.0), &Remap::default(), &mut out);
        for (i, v) in out.iter().enumerate() {
            assert!((v - frame[i * 8]).abs() < 2e-3, "{i}: {v} vs {}", frame[i * 8]);
        }
    }

    /// The kernels (scalar and SIMD) against the preview, for every warp the
    /// kernel handles, in either slot: one lane at exactly 1024 samples per
    /// cycle, so every sample lands on a preview point. They must agree bit
    /// for bit.
    #[test]
    fn every_warp_matches_its_preview() {
        let mut block = vec![0.0; FRAME_STRIDE];
        MipBuilder::default().frame(&rich_frame(), &mut block);
        let curve: Vec<f32> = (0..warp::REMAP_POINTS)
            .map(|i| {
                let x = i as f32 / (warp::REMAP_POINTS - 1) as f32;
                x * x * (3.0 - 2.0 * x)
            })
            .collect();
        let remap = Remap::from_values(&curve);
        let n = 1024;
        let inc = 1.0 / n as f32;
        let sr = 48_000.0;
        let modes: Vec<u8> = (1..=warp::LAST_IMPLEMENTED).filter(|m| !(17..=45).contains(m)).collect();
        let mut checked = 0;
        for &mode in &modes {
            for &a in &[0.1f32, 0.35, 0.8, 1.0] {
                for slot in 0..2 {
                    let (w1, w2) = if slot == 0 { ((mode, a), (warp::OFF, 0.0)) } else { ((warp::BEND_PLUS, 0.2), (mode, a)) };
                    let stretch = warp::stretch(w1.0, w1.1, &remap) * warp::stretch(w2.0, w2.1, &remap);
                    let pick = mip::pick(inc * stretch, sr);
                    let mut want = vec![0.0f32; n];
                    cycle_block(&block, pick, w1, w2, &remap, &mut want);
                    for simd in [false, true] {
                        let k = Kernel {
                            len: 16,
                            table: &block,
                            frames: 1,
                            pick,
                            lanes: 1,
                            inc0: inc,
                            inc_step: 1.0,
                            ratio: [1.0; MAX_LANES],
                            gl: [1.0; MAX_LANES],
                            gr: [1.0; MAX_LANES],
                            fpos0: [0.0; MAX_LANES],
                            fpos_step: [0.0; MAX_LANES],
                            smooth: true,
                            w1_mode: w1.0,
                            w1_amt: [w1.1; MAX_LANES],
                            w2_mode: w2.0,
                            w2_amt: [w2.1; MAX_LANES],
                            warped: warp::active(w1.0, w1.1) || warp::active(w2.0, w2.1),
                            shaped: warp::is_shape(w1.0) || warp::is_shape(w2.0),
                            read: warp::read_mode(w1.0, w1.1, w2.0, w2.1, pick),
                            remap: &remap,
                        };
                        let mut phase = [0u32; MAX_LANES];
                        for b in 0..n / 16 {
                            let (mut l, mut r) = ([0.0; 64], [0.0; 64]);
                            if simd {
                                kernel::render_simd(&k, &mut phase, 0, &mut l, &mut r);
                            } else {
                                kernel::render_scalar(&k, &mut phase, 0, &mut l, &mut r);
                            }
                            for i in 0..16 {
                                let j = b * 16 + i;
                                // exact equality (the kernel adds into zeroed buffers, so -0 comes out as +0)
                                assert!(l[i] == want[j], "mode {mode} a {a} slot {slot} simd {simd}: sample {j} {} vs preview {}", l[i], want[j]);
                            }
                        }
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, modes.len() * 4 * 2 * 2);
    }

    #[test]
    fn read_warps_filter_the_harmonics() {
        // lowpass at full keeps only the fundamental; even/odd at 0.5 keeps odd harmonics
        let frame: Vec<f32> = (0..FRAME_LEN).map(|i| (2.0 * i as f32 / FRAME_LEN as f32) - 1.0).collect(); // saw
        let n = 2048;
        let mut lp = vec![0.0; n];
        cycle(&frame, (warp::LOWPASS, 1.0), (warp::OFF, 0.0), &Remap::default(), &mut lp);
        let harm = |x: &[f32], h: usize| {
            let (mut c, mut s) = (0.0f64, 0.0f64);
            for (i, v) in x.iter().enumerate() {
                let t = std::f64::consts::TAU * (h * i) as f64 / x.len() as f64;
                c += *v as f64 * t.cos();
                s += *v as f64 * t.sin();
            }
            (c * c + s * s).sqrt() * 2.0 / x.len() as f64
        };
        assert!(harm(&lp, 1) > 0.5 && harm(&lp, 2) < 1e-3 && harm(&lp, 5) < 1e-3, "lowpass at full: {} {} {}", harm(&lp, 1), harm(&lp, 2), harm(&lp, 5));
        let mut odd = vec![0.0; n];
        cycle(&frame, (warp::EVEN_ODD, 0.5), (warp::OFF, 0.0), &Remap::default(), &mut odd);
        assert!(harm(&odd, 1) > 0.5 && harm(&odd, 2) < 1e-4 && harm(&odd, 3) > 0.1, "odd only: {} {} {}", harm(&odd, 1), harm(&odd, 2), harm(&odd, 3));
        let mut hp = vec![0.0; n];
        cycle(&frame, (warp::HIGHPASS, 1.0), (warp::OFF, 0.0), &Remap::default(), &mut hp);
        assert!(harm(&hp, 1) < 1e-3 && harm(&hp, 600) > 1e-4, "highpass at full: {} {}", harm(&hp, 1), harm(&hp, 600));
    }
}
