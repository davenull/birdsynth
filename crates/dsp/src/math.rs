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

/// sin(2πp) for a phase in turns, from an odd polynomial on a quarter wave
/// (error under 2e-6). Cheaper than `f32::sin` and the same everywhere.
#[inline]
pub fn sin_turns(p: f32) -> f32 {
    let q = p - p.floor(); // 0..1
    // fold into -1/4..1/4 turn and track the sign
    let (x, sign) = if q < 0.25 {
        (q, 1.0)
    } else if q < 0.75 {
        (0.5 - q, 1.0)
    } else {
        (q - 1.0, 1.0)
    };
    let t = x * std::f32::consts::TAU;
    let t2 = t * t;
    sign * t * (1.0 + t2 * (-1.0 / 6.0 + t2 * (1.0 / 120.0 + t2 * (-1.0 / 5040.0 + t2 * (1.0 / 362_880.0)))))
}

/// Semitones to a frequency ratio.
#[inline]
pub fn semis_to_ratio(semis: f32) -> f32 {
    (semis * (1.0 / 12.0)).exp2()
}

/// A cheap tanh (rational approximation, exact at 0 and saturating at ±1),
/// good to about 2% and monotonic, for saturation stages.
#[inline]
pub fn fast_tanh(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
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
    fn sine_is_accurate() {
        for i in 0..=1000 {
            let p = i as f32 / 1000.0 * 3.0 - 1.0;
            let exact = (std::f32::consts::TAU * p).sin();
            assert!((sin_turns(p) - exact).abs() < 2e-5, "{p}: {} vs {exact}", sin_turns(p));
        }
    }

    #[test]
    fn balance_law() {
        assert_eq!(balance(0.0), (1.0, 1.0));
        assert_eq!(balance(1.0), (0.0, 1.0));
        assert_eq!(balance(-1.0), (1.0, 0.0));
    }
}
