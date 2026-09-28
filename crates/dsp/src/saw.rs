//! The built-in band-limited sawtooth: the synth's init waveform, built
//! additively at every mip level. Tables loaded later are built by the tools
//! from their frames; this one exists so the engine makes sound on its own.

use crate::mip::{FRAME_LEN, FRAME_STRIDE, LEVELS, LEVEL_OFFSET, harmonics, level_len};

/// One frame block (see `mip`) holding a rising sawtooth: 0 at phase 0, up to
/// +1 at half a cycle, a drop to -1, and back up to 0. Every level shares one
/// scale, so the fundamental is equally loud whichever level plays.
pub fn saw_frame() -> Vec<f32> {
    additive_frame(|h| if h % 2 == 1 { 1.0 / h as f64 } else { -1.0 / h as f64 })
}

/// A band-limited triangle (0 at phase 0, peak at a quarter cycle), for the sub.
pub fn triangle_frame() -> Vec<f32> {
    additive_frame(|h| match h % 4 {
        1 => 1.0 / (h * h) as f64,
        3 => -1.0 / (h * h) as f64,
        _ => 0.0,
    })
}

/// Build a mip-mapped frame from sine partials `coef(h)` (h = 1..=1024).
pub fn additive_frame(coef: impl Fn(usize) -> f64) -> Vec<f32> {
    let mut out = vec![0.0f32; FRAME_STRIDE];
    let n = FRAME_LEN;
    // For each sample of the finest grid, sum harmonics with the Chebyshev
    // recurrence sin((h+1)x) = 2cos(x)sin(hx) - sin((h-1)x), and store the
    // partial sum into every level whose harmonic count has been reached.
    let c: Vec<f64> = (0..=harmonics(0)).map(|h| if h == 0 { 0.0 } else { coef(h) }).collect();
    for i in 0..n {
        let x = std::f64::consts::TAU * i as f64 / n as f64;
        let c2 = 2.0 * x.cos();
        let (mut s_prev, mut s) = (0.0f64, x.sin());
        let mut sum = 0.0f64;
        let mut level = LEVELS; // levels are reached from the top (1 harmonic) down
        for (h, &k) in c.iter().enumerate().skip(1) {
            sum += k * s;
            while level > 0 && harmonics(level - 1) == h {
                level -= 1;
                let len = level_len(level);
                let step = n / len;
                if i.is_multiple_of(step) {
                    out[LEVEL_OFFSET[level] + i / step] = sum as f32;
                }
            }
            let s_next = c2 * s - s_prev;
            s_prev = s;
            s = s_next;
        }
    }
    // One scale for all levels: level 0's peak (Gibbs overshoot included) maps to 1.
    let peak = out[..level_len(0)].iter().fold(0.0f32, |m, v| m.max(v.abs()));
    for v in out.iter_mut() {
        *v /= peak;
    }
    for level in 0..LEVELS {
        let o = LEVEL_OFFSET[level];
        out[o + level_len(level)] = out[o];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mip;

    #[test]
    fn levels_match_direct_sums() {
        let t = saw_frame();
        let lvl0 = &t[LEVEL_OFFSET[0]..LEVEL_OFFSET[0] + 2048];
        let peak_raw = (0..2048)
            .map(|i| direct(i as f64 / 2048.0, 1024).abs())
            .fold(0.0f64, f64::max);
        for level in [0usize, 3, 7, 10] {
            let len = level_len(level);
            for &i in &[0usize, 1, len / 7, len / 3, len / 2 - 1, len - 1] {
                let want = direct(i as f64 / len as f64, harmonics(level)) / peak_raw;
                let got = t[LEVEL_OFFSET[level] + i] as f64;
                assert!((got - want).abs() < 1e-5, "level {level} sample {i}: {got} vs {want}");
            }
            assert_eq!(t[LEVEL_OFFSET[level] + len], t[LEVEL_OFFSET[level]], "guard sample");
        }
        let peak = lvl0.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 1.0).abs() < 1e-6);
    }

    #[test]
    fn triangle_peaks_at_a_quarter_cycle() {
        let t = triangle_frame();
        assert!((t[512] - 1.0).abs() < 1e-3, "{}", t[512]);
        assert!(t[0].abs() < 1e-6);
        assert!((t[1536] + 1.0).abs() < 1e-3);
    }

    #[test]
    fn top_level_is_a_sine() {
        let t = saw_frame();
        // one harmonic, 2/π of the raw saw's peak before scaling
        let a = t[LEVEL_OFFSET[10] + 64]; // a quarter cycle of 256 samples
        let b = mip::read(&t, 10, 1 << 30); // phase 0.25
        assert!((a - b).abs() < 1e-6);
        assert!(a > 0.5 && a < 0.6, "{a}");
    }

    fn direct(p: f64, h: usize) -> f64 {
        (1..=h)
            .map(|k| {
                let sign = if k % 2 == 1 { 1.0 } else { -1.0 };
                sign * (std::f64::consts::TAU * k as f64 * p).sin() / k as f64
            })
            .sum()
    }
}
