//! Phase warps: remap an oscillator's phase (0..1) before the table read.
//!
//! Mode numbers match the `warp1_mode`/`warp2_mode` option lists in
//! params/osc.toml. P1 implements the phase-reshaping modes; the rest pass the
//! phase through until their phase of the roadmap.
//!
//! Every formula here is written the same way in the SIMD kernel
//! (`osc::simd`), operation for operation, so the two agree bit for bit.

pub const OFF: u8 = 0;
pub const SYNC: u8 = 1;
pub const WINDOW_SYNC: u8 = 2;
pub const BEND_PLUS: u8 = 3;
pub const BEND_MINUS: u8 = 4;
pub const BEND_BOTH: u8 = 5;
pub const PWM: u8 = 6;
pub const ASYM_PLUS: u8 = 7;
pub const ASYM_MINUS: u8 = 8;
pub const ASYM_BOTH: u8 = 9;

/// Highest warp mode this build implements.
pub const LAST_IMPLEMENTED: u8 = ASYM_BOTH;

/// True if a mode changes the phase (so the fast un-warped kernel can't be used).
#[inline]
pub fn active(mode: u8, amount: f32) -> bool {
    mode != OFF && mode <= LAST_IMPLEMENTED && amount > 0.0
}

/// Sync ratio for an amount: 1 to 16 times the base frequency.
#[inline]
pub fn sync_ratio(a: f32) -> f32 {
    1.0 + a * 15.0
}

/// Bend strength: the steepest slope the bend reaches is 1 + k.
#[inline]
pub fn bend_k(a: f32) -> f32 {
    a * 8.0
}

/// PWM: the fraction of the cycle that still holds the waveform.
#[inline]
pub fn pwm_width(a: f32) -> f32 {
    1.0 - a * 0.98
}

/// Asym split point: where the first half of the waveform ends.
#[inline]
pub fn asym_split(mode: u8, a: f32) -> f32 {
    if mode == ASYM_MINUS { 0.5 + 0.49 * a } else { 0.5 - 0.49 * a }
}

/// How much faster than the base rate the warped phase can move. The
/// oscillator picks its mip level from base speed × stretch, which keeps
/// bends and squeezes band-limited without oversampling.
pub fn stretch(mode: u8, a: f32) -> f32 {
    if !active(mode, a) {
        return 1.0;
    }
    match mode {
        SYNC | WINDOW_SYNC => sync_ratio(a),
        BEND_PLUS | BEND_MINUS | BEND_BOTH => 1.0 + bend_k(a),
        PWM => 1.0 / pwm_width(a),
        ASYM_PLUS | ASYM_MINUS | ASYM_BOTH => {
            let s = asym_split(mode, a);
            (0.5 / s).max(0.5 / (1.0 - s))
        }
        _ => 1.0,
    }
}

/// Bend+ : p(1+k) / (1+kp). The start of the cycle is compressed (slope 1+k
/// at 0), the end relaxed, so the waveform's features lean earlier.
#[inline]
fn bend_plus(p: f32, k: f32) -> f32 {
    p * (1.0 + k) / (1.0 + k * p)
}

/// Bend- : p / (1 + k(1-p)). The mirror image: features lean later.
#[inline]
fn bend_minus(p: f32, k: f32) -> f32 {
    p / (1.0 + k * (1.0 - p))
}

/// The first half of the waveform plays in [0, s), the second in [s, 1).
#[inline]
fn asym(p: f32, s: f32) -> f32 {
    if p < s { 0.5 * p / s } else { 0.5 + 0.5 * (p - s) / (1.0 - s) }
}

/// Warp a phase in [0, 1). Returns the new phase in [0, 1) and an
/// amplitude factor (only Window Sync uses it).
#[inline]
pub fn apply(mode: u8, p: f32, a: f32) -> (f32, f32) {
    if !active(mode, a) {
        return (p, 1.0);
    }
    match mode {
        SYNC => {
            let x = p * sync_ratio(a);
            (x - x.floor(), 1.0)
        }
        WINDOW_SYNC => {
            let x = p * sync_ratio(a);
            // fade the waveform in and out over the base cycle so the reset is smooth
            let s = (a * 8.0).min(1.0);
            let w = sin_sq_pi(p);
            (x - x.floor(), 1.0 - s + s * w)
        }
        BEND_PLUS => (bend_plus(p, bend_k(a)), 1.0),
        BEND_MINUS => (bend_minus(p, bend_k(a)), 1.0),
        BEND_BOTH => {
            let k = bend_k(a);
            let q = if p < 0.5 { 0.5 * bend_minus(2.0 * p, k) } else { 1.0 - 0.5 * bend_minus(2.0 - 2.0 * p, k) };
            (q, 1.0)
        }
        PWM => {
            let w = pwm_width(a);
            (if p < w { p / w } else { 0.0 }, 1.0)
        }
        ASYM_PLUS | ASYM_MINUS => (asym(p, asym_split(mode, a)), 1.0),
        ASYM_BOTH => {
            let s = asym_split(ASYM_PLUS, a);
            let q = if p < 0.5 { 0.5 * asym(2.0 * p, s) } else { 0.5 + 0.5 * asym(2.0 * p - 1.0, 1.0 - s) };
            (q, 1.0)
        }
        _ => (p, 1.0),
    }
}

/// sin²(πp) for p in [0, 1), from a parabola pair: accurate to 0.1%, cheap,
/// and the same polynomial the SIMD kernel evaluates.
#[inline]
pub fn sin_sq_pi(p: f32) -> f32 {
    // sin(πp) ≈ 4p(1-p) refined by Bhaskara's correction
    let x = 4.0 * p * (1.0 - p);
    let s = x * (0.775 + 0.225 * x);
    s * s
}

/// Clamp a warped phase into [0, 1) so table indexing never overruns.
#[inline]
pub fn wrap01(p: f32) -> f32 {
    let q = p - p.floor();
    if q >= 1.0 { 0.0 } else { q }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_and_zero_amount_are_identity() {
        for mode in 0..=LAST_IMPLEMENTED {
            for &p in &[0.0f32, 0.1, 0.5, 0.9, 0.999] {
                assert_eq!(apply(mode, p, 0.0), (p, 1.0), "mode {mode}");
            }
        }
    }

    #[test]
    fn phases_stay_in_range_and_are_monotonic_where_expected() {
        for mode in 1..=LAST_IMPLEMENTED {
            for &a in &[0.2f32, 0.6, 1.0] {
                let mut prev = -1.0f32;
                for i in 0..4096 {
                    let p = i as f32 / 4096.0;
                    let (q, g) = apply(mode, p, a);
                    assert!((0.0..1.0).contains(&q), "mode {mode} a {a} p {p} -> {q}");
                    assert!((0.0..=1.0001).contains(&g));
                    if matches!(mode, BEND_PLUS | BEND_MINUS | BEND_BOTH | ASYM_PLUS | ASYM_MINUS | ASYM_BOTH) {
                        assert!(q >= prev, "mode {mode} not monotonic at {p}");
                    }
                    prev = q;
                }
            }
        }
    }

    #[test]
    fn stretch_bounds_the_warped_slope() {
        for mode in 1..=LAST_IMPLEMENTED {
            for &a in &[0.3f32, 1.0] {
                let s = stretch(mode, a);
                let n = 1 << 16;
                let mut worst = 0.0f32;
                for i in 0..n - 1 {
                    let p0 = i as f32 / n as f32;
                    let p1 = (i + 1) as f32 / n as f32;
                    let (q0, _) = apply(mode, p0, a);
                    let (q1, _) = apply(mode, p1, a);
                    let d = q1 - q0;
                    if d > 0.0 {
                        worst = worst.max(d / (p1 - p0));
                    }
                }
                assert!(worst <= s * 1.01, "mode {mode} a {a}: slope {worst} > stretch {s}");
            }
        }
    }

    #[test]
    fn sync_doubles_at_ratio_two() {
        let a = 1.0 / 15.0; // ratio 2
        let (q, _) = apply(SYNC, 0.75, a);
        assert!((q - 0.5).abs() < 1e-6);
    }

    #[test]
    fn window_is_accurate() {
        for i in 0..=100 {
            let p = i as f32 / 101.0;
            let exact = (std::f32::consts::PI * p).sin().powi(2);
            assert!((sin_sq_pi(p) - exact).abs() < 2e-3, "{p}");
        }
    }
}
