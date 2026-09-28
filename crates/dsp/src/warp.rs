//! Warps: what an oscillator does to its wavetable as it plays it.
//!
//! Mode numbers match the `warp1_mode`/`warp2_mode` option lists in
//! params/osc.toml. There are four families, applied in this order:
//! - **phase** warps remap the phase (0..1) before the table read (sync,
//!   bend, PWM, asym, flip, mirror, remap 1-3, quantize), in slot order;
//! - **cross-mod** warps (FM, PD, AM, RM) are handled by the oscillators;
//! - **read** warps change how the table is read (lowpass, highpass,
//!   even/odd); the first one switched on wins;
//! - **shape** warps bend the output (the distortions and remap 4), in
//!   slot order.
//!
//! The simple phase formulas are written the same way in the SIMD kernel,
//! operation for operation; the rest the kernel evaluates lane by lane with
//! these very functions. Either way the two agree bit for bit.

use crate::math::{fast_tanh, sin_turns};
use crate::mip::{self, Pick};

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
pub const FLIP: u8 = 10;
pub const MIRROR: u8 = 11;
pub const REMAP_1: u8 = 12;
pub const REMAP_2: u8 = 13;
pub const REMAP_3: u8 = 14;
pub const REMAP_4: u8 = 15;
pub const QUANTIZE: u8 = 16;
/// 17..=45 are the cross-modulation modes (see `osc::xmod` in the engine).
pub const SOFT_CLIP: u8 = 46;
pub const HARD_CLIP: u8 = 47;
pub const SOFT_SAT: u8 = 48;
pub const TAPE_SAT: u8 = 49;
pub const TUBE: u8 = 50;
pub const DIODE_1: u8 = 51;
pub const DIODE_2: u8 = 52;
pub const LIN_FOLD: u8 = 53;
pub const SIN_FOLD: u8 = 54;
pub const SINE_SHAPER: u8 = 55;
pub const ASYM_DIST: u8 = 56;
pub const STOMP: u8 = 57;
pub const ZERO_SQUARE: u8 = 58;
pub const LOWPASS: u8 = 59;
pub const HIGHPASS: u8 = 60;
pub const EVEN_ODD: u8 = 61;

/// Highest warp mode this build implements.
pub const LAST_IMPLEMENTED: u8 = EVEN_ODD;

/// A phase warp (runs before the table read).
#[inline]
pub fn is_phase(mode: u8) -> bool {
    (SYNC..=QUANTIZE).contains(&mode) && mode != REMAP_4
}

/// A shape warp (bends the output after the read).
#[inline]
pub fn is_shape(mode: u8) -> bool {
    (SOFT_CLIP..=ZERO_SQUARE).contains(&mode) || mode == REMAP_4
}

/// A read warp (changes which harmonics the read returns).
#[inline]
pub fn is_read(mode: u8) -> bool {
    (LOWPASS..=EVEN_ODD).contains(&mode)
}

/// True if a mode does anything the oscillator itself handles (so the fast
/// un-warped kernel can't be used). Cross-mod modes are false here.
#[inline]
pub fn active(mode: u8, amount: f32) -> bool {
    amount > 0.0 && (is_phase(mode) || is_shape(mode) || is_read(mode))
}

/// Does this warp add harmonics that oversampling helps with? (Cross-mod is
/// the oscillators' call.)
#[inline]
pub fn aliases(mode: u8) -> bool {
    matches!(mode, SYNC | WINDOW_SYNC | FLIP | QUANTIZE) || (SOFT_CLIP..=ZERO_SQUARE).contains(&mode)
}

// ------------------------------------------------------------ remap curve

/// Points in a remap lookup table (the curve drawn for Remap 1-4).
pub const REMAP_POINTS: usize = 257;

/// An oscillator's drawn remap curve, as a lookup table over 0..1 with
/// values 0..1. As a phase map it sends x to `eval(x)`; Remap 4 uses it as a
/// waveshaper on -1..1 instead.
#[derive(Clone, Copy, Debug)]
pub struct Remap {
    lut: [f32; REMAP_POINTS],
    /// Steepest slope of the curve (for mip selection).
    pub slope: f32,
}

impl Default for Remap {
    fn default() -> Self {
        let mut lut = [0.0; REMAP_POINTS];
        for (i, v) in lut.iter_mut().enumerate() {
            *v = i as f32 / (REMAP_POINTS - 1) as f32;
        }
        Remap { lut, slope: 1.0 }
    }
}

impl Remap {
    /// Build from table values (clamped to 0..1). Shorter input keeps the identity tail.
    pub fn from_values(v: &[f32]) -> Remap {
        let mut r = Remap::default();
        for (d, s) in r.lut.iter_mut().zip(v) {
            *d = if s.is_nan() { 0.0 } else { s.clamp(0.0, 1.0) };
        }
        let n = (REMAP_POINTS - 1) as f32;
        r.slope = r.lut.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs() * n)).max(1.0);
        r
    }

    #[inline]
    pub fn eval(&self, x: f32) -> f32 {
        let f = x.clamp(0.0, 1.0) * (REMAP_POINTS - 1) as f32;
        let i = (f as usize).min(REMAP_POINTS - 2);
        let t = f - i as f32;
        let a = self.lut[i];
        a + (self.lut[i + 1] - a) * t
    }
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
pub fn stretch(mode: u8, a: f32, remap: &Remap) -> f32 {
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
        MIRROR => (1.0 + a).max((1.0 - 3.0 * a).abs()),
        REMAP_1 | REMAP_2 | REMAP_3 => 1.0 + a * (remap.slope - 1.0).max(0.0),
        _ => 1.0,
    }
}

/// Quantize: steps per cycle for an amount (256 near zero, 2 at full).
#[inline]
pub fn quant_steps(a: f32) -> f32 {
    let b = 1.0 - a;
    (2.0 + b * b * 254.0).round()
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

/// Phase warp a phase in [0, 1). Returns the new phase and an amplitude
/// factor (Window Sync fades, Flip inverts). Other families pass through.
#[inline]
pub fn apply(mode: u8, p: f32, a: f32, remap: &Remap) -> (f32, f32) {
    if !active(mode, a) || !is_phase(mode) {
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
        // invert the waveform from 1 - amount to the end of the cycle
        FLIP => (p, if p >= 1.0 - a { -1.0 } else { 1.0 }),
        // play the cycle forward, then backward
        MIRROR => {
            let m = if p < 0.5 { 2.0 * p } else { 2.0 - 2.0 * p };
            (p + a * (m - p), 1.0)
        }
        REMAP_1 => (p + a * (remap.eval(p) - p), 1.0),
        // the curve over the first half, mirrored over the second: symmetric
        REMAP_2 => {
            let m = if p < 0.5 { 0.5 * remap.eval(2.0 * p) } else { 1.0 - 0.5 * remap.eval(2.0 - 2.0 * p) };
            (p + a * (m - p), 1.0)
        }
        // the curve applied to each half of the cycle
        REMAP_3 => {
            let h = if p < 0.5 { 0.0 } else { 0.5 };
            let m = h + 0.5 * remap.eval(2.0 * (p - h));
            (p + a * (m - p), 1.0)
        }
        QUANTIZE => {
            let n = quant_steps(a);
            ((p * n).floor() / n, 1.0)
        }
        _ => (p, 1.0),
    }
}

// ------------------------------------------------------------ shape warps

/// Drive for the distortion warps: up to 24 dB.
#[inline]
pub fn drive(a: f32) -> f32 {
    1.0 + a * 15.0
}

/// One distortion curve at unit scale (no drive, no normalization).
#[inline]
fn curve(mode: u8, x: f32) -> f32 {
    match mode {
        SOFT_CLIP => fast_tanh(x),
        HARD_CLIP => x.clamp(-1.0, 1.0),
        SOFT_SAT => x / (1.0 + x.abs()),
        TAPE_SAT => x / (1.0 + x * x).sqrt(),
        TUBE => fast_tanh(x + 0.3) - fast_tanh(0.3),
        DIODE_1 => {
            if x >= 0.0 {
                fast_tanh(x)
            } else {
                0.25 * fast_tanh(x)
            }
        }
        DIODE_2 => {
            if x >= 0.0 {
                fast_tanh(x)
            } else {
                let t = fast_tanh(-x);
                -(t * t)
            }
        }
        LIN_FOLD => {
            let t = (x - 1.0) * 0.25;
            4.0 * (t - t.floor() - 0.5).abs() - 1.0
        }
        SIN_FOLD => sin_turns(x * 0.25),
        ASYM_DIST => {
            if x >= 0.0 {
                fast_tanh(1.5 * x)
            } else {
                fast_tanh(0.5 * x)
            }
        }
        STOMP => x.clamp(-0.6, 0.9) / 0.9,
        _ => x,
    }
}

/// Shape warp an output sample. The first quarter of the amount fades the
/// effect in, so switching one on never jumps.
#[inline]
pub fn shape(mode: u8, y: f32, a: f32, remap: &Remap) -> f32 {
    if !active(mode, a) || !is_shape(mode) {
        return y;
    }
    let mix = (a * 4.0).min(1.0);
    let wet = match mode {
        REMAP_4 => 2.0 * remap.eval(0.5 * (y + 1.0)) - 1.0,
        ZERO_SQUARE => {
            let m = y.abs().min(1.0);
            let e = 1.0 + 7.0 * a;
            y.signum() * (1.0 - (1.0 - m).powf(e))
        }
        SINE_SHAPER => sin_turns(y * 0.25 * (1.0 + 7.0 * a)),
        LIN_FOLD | SIN_FOLD => curve(mode, y * drive(a)),
        _ => {
            // clippers and saturators: drive in, then bring full scale (either polarity) back to 1
            let g = drive(a);
            let peak = curve(mode, g).abs().max(curve(mode, -g).abs()).max(0.1);
            curve(mode, y * g) / peak
        }
    };
    y + (wet - y) * mix
}

// ------------------------------------------------------------- read warps

/// How the table is read this block.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Read {
    Plain,
    /// Read a duller mip level: a lowpass stepping smoothly through the octaves.
    Low(Pick),
    /// The full read minus a duller one, faded in by `mix`.
    High(Pick, f32),
    /// Odd and even harmonics weighted separately (from the amount of warp slot `slot`).
    EvenOdd(u8),
}

/// Mip position a lowpass warp keeps: 10 octaves of harmonics removed at full.
#[inline]
pub fn low_level(a: f32) -> f32 {
    10.0 * a
}

/// The level the highpass subtracts: the fundamental only near zero, the
/// lowest 512 harmonics at full.
#[inline]
pub fn high_level(a: f32) -> f32 {
    10.0 - 9.0 * a
}

/// Even/odd weights: full at 0, odd only at 0.5, even only at 1.
#[inline]
pub fn even_odd(a: f32) -> (f32, f32) {
    ((2.0 - 2.0 * a).min(1.0), (1.0 - 2.0 * a).abs())
}

/// The read mode for a block: the first read warp switched on, by slot.
/// `pick` is the band-limiting pick; `a1`/`a2` the slots' largest amounts.
pub fn read_mode(w1: u8, a1: f32, w2: u8, a2: f32, pick: Pick) -> Read {
    for (slot, (mode, a)) in [(w1, a1), (w2, a2)].into_iter().enumerate() {
        if !active(mode, a) || !is_read(mode) {
            continue;
        }
        return match mode {
            LOWPASS => Read::Low(mip::duller(pick, mip::pick_level(low_level(a)))),
            HIGHPASS => Read::High(mip::duller(pick, mip::pick_level(high_level(a))), (a * 4.0).min(1.0)),
            _ => Read::EvenOdd(slot as u8),
        };
    }
    Read::Plain
}

/// One read of a frame block at phase `q` in a read mode. `amt` holds the
/// lane's two warp amounts (for Even/Odd).
#[inline]
pub fn read(block: &[f32], pick: Pick, mode: Read, q: f32, amt: [f32; 2]) -> f32 {
    match mode {
        Read::Plain => mip::read_pick_f(block, pick, q),
        Read::Low(p) => mip::read_pick_f(block, p, q),
        Read::High(p, mix) => {
            let full = mip::read_pick_f(block, pick, q);
            full - mix * mip::read_pick_f(block, p, q)
        }
        Read::EvenOdd(slot) => {
            let f = mip::read_pick_f(block, pick, q);
            let g = mip::read_pick_f(block, pick, wrap01(q + 0.5));
            let (wo, we) = even_odd(amt[slot as usize]);
            wo * (0.5 * (f - g)) + we * (0.5 * (f + g))
        }
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

    const ID: Remap = Remap { lut: [0.0; REMAP_POINTS], slope: 1.0 };

    fn identity() -> Remap {
        Remap::default()
    }

    #[test]
    fn off_and_zero_amount_are_identity() {
        let r = identity();
        let _ = ID;
        for mode in 0..=LAST_IMPLEMENTED {
            for &p in &[0.0f32, 0.1, 0.5, 0.9, 0.999] {
                assert_eq!(apply(mode, p, 0.0, &r), (p, 1.0), "mode {mode}");
                assert_eq!(shape(mode, p - 0.5, 0.0, &r), p - 0.5, "mode {mode}");
            }
        }
    }

    #[test]
    fn families_do_not_overlap() {
        for mode in 0..=LAST_IMPLEMENTED {
            let n = [is_phase(mode), is_shape(mode), is_read(mode)].iter().filter(|&&b| b).count();
            let xmod = (17..=45).contains(&mode);
            assert_eq!(n + xmod as usize, (mode != OFF) as usize, "mode {mode}");
        }
    }

    #[test]
    fn identity_remaps_leave_the_waveform_alone() {
        let r = identity();
        for mode in [REMAP_1, REMAP_2, REMAP_3] {
            for i in 0..100 {
                let p = i as f32 / 100.0;
                assert!((apply(mode, p, 1.0, &r).0 - p).abs() < 1e-6, "mode {mode} p {p}");
            }
        }
        for i in 0..100 {
            let y = i as f32 / 50.0 - 1.0;
            assert!((shape(REMAP_4, y, 1.0, &r) - y).abs() < 1e-6);
        }
    }

    #[test]
    fn shapes_stay_bounded_and_keep_silence() {
        let r = identity();
        for mode in SOFT_CLIP..=ZERO_SQUARE {
            for &a in &[0.1f32, 0.5, 1.0] {
                assert!(shape(mode, 0.0, a, &r).abs() < 1e-6, "mode {mode} turns silence into DC");
                for i in 0..=200 {
                    let y = i as f32 / 100.0 - 1.0;
                    let v = shape(mode, y, a, &r);
                    assert!(v.is_finite() && v.abs() <= 1.0001, "mode {mode} a {a} y {y} -> {v}");
                }
            }
        }
    }

    #[test]
    fn quantize_makes_steps() {
        let r = identity();
        let a = 1.0; // 2 steps
        assert_eq!(apply(QUANTIZE, 0.3, a, &r).0, 0.0);
        assert_eq!(apply(QUANTIZE, 0.7, a, &r).0, 0.5);
        assert_eq!(quant_steps(0.0001), 256.0);
    }

    #[test]
    fn even_odd_weights() {
        assert_eq!(even_odd(0.0), (1.0, 1.0));
        assert_eq!(even_odd(0.5), (1.0, 0.0));
        assert_eq!(even_odd(1.0), (0.0, 1.0));
    }

    #[test]
    fn phases_stay_in_range_and_are_monotonic_where_expected() {
        let r = identity();
        for mode in (1..=LAST_IMPLEMENTED).filter(|&m| is_phase(m)) {
            for &a in &[0.2f32, 0.6, 1.0] {
                let mut prev = -1.0f32;
                for i in 0..4096 {
                    let p = i as f32 / 4096.0;
                    let (q, g) = apply(mode, p, a, &r);
                    assert!((0.0..=1.0).contains(&q), "mode {mode} a {a} p {p} -> {q}");
                    assert!((-1.0..=1.0001).contains(&g));
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
        let r = identity();
        // quantize moves in steps: its slope between steps is zero, so skip it
        for mode in (1..=LAST_IMPLEMENTED).filter(|&m| is_phase(m) && m != QUANTIZE) {
            for &a in &[0.3f32, 1.0] {
                let s = stretch(mode, a, &r);
                let n = 1 << 16;
                let mut worst = 0.0f32;
                for i in 0..n - 1 {
                    let p0 = i as f32 / n as f32;
                    let p1 = (i + 1) as f32 / n as f32;
                    let (q0, _) = apply(mode, p0, a, &r);
                    let (q1, _) = apply(mode, p1, a, &r);
                    let d = (q1 - q0).abs();
                    if d > 0.0 && d < 0.5 {
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
        let (q, _) = apply(SYNC, 0.75, a, &identity());
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
