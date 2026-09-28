//! Small numeric helpers.

/// Decibels to linear gain. -inf dB gives exactly 0.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    if db == f32::NEG_INFINITY { 0.0 } else { 10f32.powf(db * 0.05) }
}

/// A (fractional) MIDI note number to Hz, with A4 = note 69 = 440 Hz.
#[inline]
pub fn note_to_hz(note: f64) -> f64 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// Flush values too small to hear to zero, so feedback state never goes subnormal.
/// Wasm has no flush-to-zero mode, and subnormals are slow on x86.
#[inline]
pub fn flush(x: f32) -> f32 {
    if x.abs() < 1e-20 { 0.0 } else { x }
}

/// Pan gains with a balance law: centre is unity on both sides, and a side
/// only gets quieter as the pan moves away from it.
#[inline]
pub fn balance(pan: f32) -> (f32, f32) {
    let p = pan.clamp(-1.0, 1.0);
    (1.0 - p.max(0.0), 1.0 + p.min(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_and_notes() {
        assert_eq!(db_to_gain(f32::NEG_INFINITY), 0.0);
        assert!((db_to_gain(-6.0) - 0.501_187).abs() < 1e-5);
        assert!((note_to_hz(69.0) - 440.0).abs() < 1e-12);
        assert!((note_to_hz(81.0) - 880.0).abs() < 1e-9);
    }

    #[test]
    fn balance_law() {
        assert_eq!(balance(0.0), (1.0, 1.0));
        assert_eq!(balance(1.0), (0.0, 1.0));
        assert_eq!(balance(-1.0), (1.0, 0.0));
    }
}
