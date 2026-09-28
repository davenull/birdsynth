//! Oscillator phase as a u32 fraction of a cycle. It wraps for free and
//! never drifts: 2^32 steps per cycle gives better than 1e-5 Hz resolution
//! at audio sample rates.

/// One whole cycle, as a float.
pub const ONE: f64 = 4_294_967_296.0;

/// Phase increment per sample for `hz` at sample rate `sr`.
/// Saturates at one cycle per sample; negative frequencies give 0.
#[inline]
pub fn inc(hz: f64, sr: f64) -> u32 {
    (hz / sr * ONE) as u32
}

/// An increment as cycles per sample.
#[inline]
pub fn cycles(inc: u32) -> f32 {
    inc as f32 * (1.0 / 4_294_967_296.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn increments() {
        let i = inc(440.0, 48_000.0);
        let hz = i as f64 * 48_000.0 / ONE;
        assert!((hz - 440.0).abs() < 1e-5, "{hz}");
        assert_eq!(inc(96_000.0, 48_000.0), u32::MAX);
        assert_eq!(inc(-5.0, 48_000.0), 0);
    }
}
