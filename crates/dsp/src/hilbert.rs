//! A Hilbert transformer from two chains of second-order allpasses (after
//! Olli Niemitalo's design): the two outputs stay 90° apart from about
//! 20 Hz to 20 kHz, which is what a frequency shifter needs.

const A: [f32; 4] = [0.692_387_8, 0.936_065_43, 0.988_229_5, 0.998_748_85];
const B: [f32; 4] = [0.402_192_12, 0.856_171_1, 0.972_290_95, 0.995_288_5];

#[derive(Clone, Copy, Debug, Default)]
struct Section {
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Section {
    #[inline(always)]
    fn tick(&mut self, x: f32, a2: f32) -> f32 {
        let y = a2 * (x + self.y2) - self.x2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Hilbert {
    a: [Section; 4],
    b: [Section; 4],
    delay: f32,
}

impl Hilbert {
    /// One sample in; the in-phase and quadrature outputs.
    #[inline]
    pub fn tick(&mut self, x: f32) -> (f32, f32) {
        let mut i = x;
        for (s, a) in self.a.iter_mut().zip(A) {
            i = s.tick(i, a * a);
        }
        let mut q = x;
        for (s, b) in self.b.iter_mut().zip(B) {
            q = s.tick(q, b * b);
        }
        // the first chain runs one sample behind
        let out = self.delay;
        self.delay = i;
        (out, q)
    }

    pub fn flush(&mut self) {
        for s in self.a.iter_mut().chain(self.b.iter_mut()) {
            s.x1 = crate::math::flush(s.x1);
            s.x2 = crate::math::flush(s.x2);
            s.y1 = crate::math::flush(s.y1);
            s.y2 = crate::math::flush(s.y2);
        }
        self.delay = crate::math::flush(self.delay);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outputs_are_in_quadrature_across_the_band() {
        let sr = 48_000.0f64;
        for f in [50.0f64, 200.0, 1000.0, 5000.0, 15000.0] {
            let mut h = Hilbert::default();
            let n = 1 << 15;
            let (mut ii, mut qq, mut iq) = (0.0f64, 0.0f64, 0.0f64);
            for k in 0..n {
                let x = (std::f64::consts::TAU * f * k as f64 / sr).sin() as f32;
                let (i, q) = h.tick(x);
                if k > n / 2 {
                    ii += (i * i) as f64;
                    qq += (q * q) as f64;
                    iq += (i * q) as f64;
                }
            }
            // equal level and zero correlation: 90° apart
            let level = 10.0 * (ii / qq).log10();
            let corr = iq / (ii * qq).sqrt();
            assert!(level.abs() < 0.1, "{f} Hz: levels differ by {level:.2} dB");
            assert!(corr.abs() < 0.03, "{f} Hz: correlation {corr:.3}");
        }
    }
}
