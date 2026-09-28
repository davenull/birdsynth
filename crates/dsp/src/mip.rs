//! Mip-mapped wavetable layout.
//!
//! Every 2048-sample frame is stored at 11 band-limited levels. Level L keeps
//! the first `1024 >> L` harmonics in `clamp(8·H, 256, 2048)` samples, so from
//! level 2 up each level is at least 8× oversampled for linear interpolation.
//! Each level is followed by one guard sample (a copy of its sample 0), so an
//! interpolated read never has to wrap. One frame is `FRAME_STRIDE` floats:
//! 9,216 samples plus 11 guards.

/// Samples per single-cycle frame (the Serum format).
pub const FRAME_LEN: usize = 2048;
/// Band-limited levels per frame: 1024, 512, ... 1 harmonics.
pub const LEVELS: usize = 11;

/// Harmonics kept at a level.
pub const fn harmonics(level: usize) -> usize {
    1024 >> level
}

/// Stored samples at a level (excluding the guard sample).
pub const fn level_len(level: usize) -> usize {
    let n = 8 * harmonics(level);
    if n < 256 {
        256
    } else if n > FRAME_LEN {
        FRAME_LEN
    } else {
        n
    }
}

/// log2 of `level_len`.
pub const LEVEL_BITS: [u32; LEVELS] = {
    let mut out = [0; LEVELS];
    let mut l = 0;
    while l < LEVELS {
        out[l] = level_len(l).trailing_zeros();
        l += 1;
    }
    out
};

/// Where each level starts inside a frame's block.
pub const LEVEL_OFFSET: [usize; LEVELS] = {
    let mut out = [0; LEVELS];
    let mut acc = 0;
    let mut l = 0;
    while l < LEVELS {
        out[l] = acc;
        acc += level_len(l) + 1;
        l += 1;
    }
    out
};

/// Floats per frame, all levels and guards included.
pub const FRAME_STRIDE: usize = LEVEL_OFFSET[LEVELS - 1] + level_len(LEVELS - 1) + 1;

/// The highest partial frequency allowed through. Anything between Nyquist
/// and this folds back to 18 kHz or above, where it's inaudible, so the
/// tables stay brighter than a strict Nyquist cut would allow.
#[inline]
pub fn top_hz(sr: f32) -> f32 {
    (sr * 0.5).max(sr - 18_000.0)
}

/// Which levels to read: sample `lo`, crossfaded toward `hi` by `w`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pick {
    pub lo: usize,
    pub hi: usize,
    pub w: f32,
}

/// Choose levels for a peak phase increment, in cycles per sample.
///
/// This uses the brightest level whose top harmonic stays under `top_hz`.
/// In the top quarter octave below each switch point it crossfades into the
/// next, duller level, so timbre never steps as the pitch rises. Both levels
/// in a crossfade are legal, so the fade adds no aliasing.
pub fn pick(cycles: f32, sr: f32) -> Pick {
    let f0 = cycles * sr;
    if f0.is_nan() || f0 <= 0.0 {
        return Pick { lo: 0, hi: 0, w: 0.0 };
    }
    let budget = top_hz(sr) / f0; // harmonics that fit
    let l = (1024.0 / budget).log2(); // continuous level
    let k = l.ceil().max(0.0);
    if k >= (LEVELS - 1) as f32 {
        return Pick { lo: LEVELS - 1, hi: LEVELS - 1, w: 0.0 };
    }
    let d = k - l; // octaves below the point where level k stops being legal
    let lo = k as usize;
    if d < 0.25 { Pick { lo, hi: lo + 1, w: 1.0 - d * 4.0 } } else { Pick { lo, hi: lo, w: 0.0 } }
}

/// Linear-interpolated read of one level. `frame` is one frame's block.
#[inline(always)]
pub fn read(frame: &[f32], level: usize, phase: u32) -> f32 {
    let bits = LEVEL_BITS[level];
    let idx = (phase >> (32 - bits)) as usize;
    let frac = ((phase << bits) >> 8) as f32 * (1.0 / 16_777_216.0);
    let o = LEVEL_OFFSET[level] + idx;
    let a = frame[o];
    let b = frame[o + 1];
    a + (b - a) * frac
}

/// Read with the crossfade a `Pick` asks for.
#[inline(always)]
pub fn read_pick(frame: &[f32], pick: Pick, phase: u32) -> f32 {
    let a = read(frame, pick.lo, phase);
    if pick.w > 0.0 { a + (read(frame, pick.hi, phase) - a) * pick.w } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout() {
        let lens: Vec<usize> = (0..LEVELS).map(level_len).collect();
        assert_eq!(lens, [2048, 2048, 2048, 1024, 512, 256, 256, 256, 256, 256, 256]);
        assert_eq!(lens.iter().sum::<usize>(), 9216);
        assert_eq!(FRAME_STRIDE, 9216 + LEVELS);
        assert_eq!(LEVEL_BITS[0], 11);
        assert_eq!(LEVEL_BITS[5], 8);
    }

    // The top harmonic of every level used must stay below top_hz.
    #[test]
    fn picks_are_legal() {
        for &sr in &[44_100.0f32, 48_000.0, 96_000.0] {
            let mut hz = 8.0f32;
            while hz < sr * 0.5 {
                let p = pick(hz / sr, sr);
                for l in [p.lo, p.hi] {
                    if l < LEVELS - 1 {
                        let top = harmonics(l) as f32 * hz;
                        assert!(top <= top_hz(sr) * 1.0001, "sr {sr} hz {hz} level {l} reaches {top}");
                    }
                }
                hz *= 1.01;
            }
        }
    }

    // The effective level (lo + w·(hi-lo)) must rise smoothly with pitch.
    #[test]
    fn picks_are_continuous() {
        let sr = 48_000.0f32;
        let mut prev: Option<f32> = None;
        let mut hz = 10.0f32;
        while hz < 20_000.0 {
            let p = pick(hz / sr, sr);
            let eff = p.lo as f32 + p.w * (p.hi as f32 - p.lo as f32);
            if let Some(q) = prev {
                assert!(eff >= q - 1e-6, "level went down at {hz} Hz");
                assert!(eff - q < 0.05, "level jumped {q} -> {eff} at {hz} Hz");
            }
            prev = Some(eff);
            hz *= 1.001;
        }
    }
}
