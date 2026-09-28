//! The convolver's impulse-response layout, shared by the tools (which
//! prepare responses) and the engine (which plays them); see
//! crates/engine/src/fx/convolve.rs for how they're used.
//!
//! Per channel (L then R), in f32s:
//!   head[HEAD] · H1[P1][BINS1 complex] · H2[P2][BINS2 complex] · FDL2[P2][BINS2 complex]
//! Spectra carry the inverse FFT's 1/size, and FDL2 is the engine's level-2
//! input history (zeros when prepared).

use crate::fft::{Complex, RealFft};

pub const HEAD: usize = 64;
pub const B1: usize = 64;
pub const P1: usize = 31;
pub const B2: usize = 1024;
/// First tap handled by level 2.
pub const L2_START: usize = B1 * (P1 + 1);
pub const BINS1: usize = B1 + 1;
pub const BINS2: usize = B2 + 1;
/// Longest response, in taps (about 4 s at 48 kHz).
pub const MAX_TAPS: usize = L2_START + 186 * B2;

/// Level-2 partitions for a response length.
pub fn partitions2(taps: usize) -> usize {
    taps.saturating_sub(L2_START).div_ceil(B2)
}

/// f32s per channel.
pub fn channel_len(taps: usize) -> usize {
    HEAD + P1 * BINS1 * 2 + 2 * partitions2(taps) * BINS2 * 2
}

/// Prepare a response (each channel's taps; the shorter is zero-padded) into `out`
/// (2 × channel_len floats).
pub fn prepare(ir: [&[f32]; 2], out: &mut [f32]) {
    let taps = ir[0].len().max(ir[1].len()).min(MAX_TAPS);
    let cl = channel_len(taps);
    assert_eq!(out.len(), 2 * cl);
    let p2 = partitions2(taps);
    let mut f1 = RealFft::new(2 * B1);
    let mut f2 = RealFft::new(2 * B2);
    let mut buf1 = vec![0.0f32; 2 * B1];
    let mut spec1 = vec![Complex::default(); BINS1];
    let mut buf2 = vec![0.0f32; 2 * B2];
    let mut spec2 = vec![Complex::default(); BINS2];
    for c in 0..2 {
        let h = ir[c];
        let tap = |t: usize| if t < taps { h.get(t).copied().unwrap_or(0.0) } else { 0.0 };
        let o = &mut out[c * cl..(c + 1) * cl];
        o.fill(0.0);
        for t in 0..HEAD {
            o[t] = tap(t);
        }
        for p in 0..P1 {
            buf1.fill(0.0);
            for t in 0..B1 {
                buf1[t] = tap(B1 * (p + 1) + t) / (2 * B1) as f32;
            }
            f1.forward(&buf1, &mut spec1);
            for (b, v) in spec1.iter().enumerate() {
                o[HEAD + (p * BINS1 + b) * 2] = v.re;
                o[HEAD + (p * BINS1 + b) * 2 + 1] = v.im;
            }
        }
        let h2 = HEAD + P1 * BINS1 * 2;
        for q in 0..p2 {
            buf2.fill(0.0);
            for t in 0..B2 {
                buf2[t] = tap(L2_START + B2 * q + t) / (2 * B2) as f32;
            }
            f2.forward(&buf2, &mut spec2);
            for (b, v) in spec2.iter().enumerate() {
                o[h2 + (q * BINS2 + b) * 2] = v.re;
                o[h2 + (q * BINS2 + b) * 2 + 1] = v.im;
            }
        }
    }
}
