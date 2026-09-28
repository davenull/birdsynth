//! Halfband decimators for the oversampled voice path.
//!
//! One stage halves the sample rate with a linear-phase FIR (a Kaiser-windowed
//! sinc at a quarter of the input rate). Every other tap of a halfband is zero,
//! so each output costs about half the taps. Aliases only come from content
//! above the output's Nyquist; the transition band is placed so anything that
//! folds lands at 19 kHz or higher at 48 kHz.

/// Taps per stage (odd, so the filter has a centre tap).
pub const TAPS: usize = 47;
/// Group delay of one stage, in input samples.
pub const DELAY: usize = (TAPS - 1) / 2;

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let half = x * 0.5;
    for k in 1..50 {
        term *= (half / k as f64) * (half / k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

/// The halfband's coefficients (unity DC gain).
pub fn design() -> [f32; TAPS] {
    let beta = 7.5f64;
    let mut h = [0.0f64; TAPS];
    let c = DELAY as f64;
    for (i, v) in h.iter_mut().enumerate() {
        let n = i as f64 - c;
        let sinc = if n == 0.0 { 0.5 } else { (std::f64::consts::PI * n * 0.5).sin() / (std::f64::consts::PI * n) };
        let r = n / c;
        let w = bessel_i0(beta * (1.0 - r * r).max(0.0).sqrt()) / bessel_i0(beta);
        *v = sinc * w;
    }
    let sum: f64 = h.iter().sum();
    let mut out = [0.0f32; TAPS];
    for i in 0..TAPS {
        out[i] = (h[i] / sum) as f32;
    }
    out
}

/// One 2:1 stage for one channel.
#[derive(Clone, Copy, Debug)]
pub struct Halfband {
    hist: [f32; TAPS * 2],
    pos: usize,
}

impl Default for Halfband {
    fn default() -> Self {
        Halfband { hist: [0.0; TAPS * 2], pos: 0 }
    }
}

impl Halfband {
    /// Decimate `input` (2·out.len() samples) into `out`.
    pub fn process(&mut self, h: &[f32; TAPS], input: &[f32], out: &mut [f32]) {
        debug_assert_eq!(input.len(), out.len() * 2);
        for (m, y) in out.iter_mut().enumerate() {
            for k in 0..2 {
                let x = input[2 * m + k];
                // history is kept twice over so the window never wraps
                self.hist[self.pos] = x;
                self.hist[self.pos + TAPS] = x;
                self.pos = if self.pos + 1 == TAPS { 0 } else { self.pos + 1 };
            }
            // newest sample sits just before pos
            let base = self.pos; // hist[base..base+TAPS] is oldest..newest
            let w = &self.hist[base..base + TAPS];
            let mut acc = h[DELAY] * w[DELAY];
            let mut i = (DELAY + 1) % 2; // the non-zero taps sit at odd distances from the centre
            while i < TAPS {
                if i != DELAY {
                    acc += h[i] * w[i];
                }
                i += 2;
            }
            *y = acc;
        }
    }

    pub fn reset(&mut self) {
        *self = Halfband::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gain_at(freq: f64, sr_in: f64) -> f64 {
        // steady sine through one stage; measure the output amplitude
        let h = design();
        let mut hb = Halfband::default();
        let n = 1 << 14;
        let input: Vec<f32> = (0..2 * n).map(|i| (std::f64::consts::TAU * freq * i as f64 / sr_in).sin() as f32).collect();
        let mut out = vec![0.0f32; n];
        hb.process(&h, &input, &mut out);
        out[n / 2..].iter().fold(0.0f32, |m, v| m.max(v.abs())) as f64
    }

    #[test]
    fn passes_the_audio_band_and_stops_what_would_alias() {
        let sr_in = 96_000.0;
        // passband: flat to 18 kHz
        for f in [100.0, 1000.0, 10_000.0, 18_000.0] {
            let g = gain_at(f, sr_in);
            assert!((g - 1.0).abs() < 0.01, "{f} Hz: {g}");
        }
        // anything that would fold below 19 kHz is at least 70 dB down
        for f in [29_000.0, 35_000.0, 45_000.0] {
            let g = gain_at(f, sr_in);
            assert!(20.0 * g.log10() < -70.0, "{f} Hz: {:.1} dB", 20.0 * g.log10());
        }
    }

    #[test]
    fn taps_are_a_halfband() {
        let h = design();
        let dc: f32 = h.iter().sum();
        assert!((dc - 1.0).abs() < 1e-6);
        for (i, &v) in h.iter().enumerate() {
            if i != DELAY && (i as isize - DELAY as isize) % 2 == 0 {
                assert!(v.abs() < 1e-7, "tap {i} should be zero: {v}");
            }
        }
    }
}
