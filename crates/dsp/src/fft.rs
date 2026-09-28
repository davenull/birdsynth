//! A small real FFT for power-of-two sizes, for the engine's convolver
//! (the tools use `realfft`, which is larger). It computes the standard DFT,
//! X[k] = Σ x[n]·e^(-2πikn/N) for k = 0..=N/2, and an unnormalized inverse,
//! so spectra made by either library work with the other.
//!
//! Method: pack the N reals as N/2 complex values, run an iterative radix-2
//! complex FFT, then split the result into the real signal's spectrum.
//! Tables are made once; `forward` and `inverse` never allocate.

use std::f64::consts::PI;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Complex {
    pub re: f32,
    pub im: f32,
}

impl Complex {
    #[inline(always)]
    pub fn new(re: f32, im: f32) -> Complex {
        Complex { re, im }
    }

    #[inline(always)]
    pub fn times(self, o: Complex) -> Complex {
        Complex { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
    }

    #[inline(always)]
    pub fn mac(&mut self, a: Complex, b: Complex) {
        self.re += a.re * b.re - a.im * b.im;
        self.im += a.re * b.im + a.im * b.re;
    }
}

pub struct RealFft {
    n: usize,
    /// Twiddles for the N/2 complex FFT: e^(-2πik/(N/2)), k < N/4.
    tw: Box<[Complex]>,
    /// Twiddles for the real split: e^(-2πik/N), k ≤ N/4.
    split: Box<[Complex]>,
    rev: Box<[u32]>,
    work: Box<[Complex]>,
}

impl RealFft {
    pub fn new(n: usize) -> RealFft {
        assert!(n.is_power_of_two() && n >= 4, "RealFft needs a power of two ≥ 4");
        let h = n / 2;
        let tw = (0..h / 2).map(|k| {
            let a = -2.0 * PI * k as f64 / h as f64;
            Complex::new(a.cos() as f32, a.sin() as f32)
        });
        let split = (0..=n / 4).map(|k| {
            let a = -2.0 * PI * k as f64 / n as f64;
            Complex::new(a.cos() as f32, a.sin() as f32)
        });
        let bits = h.trailing_zeros();
        let rev = (0..h as u32).map(|i| if bits == 0 { 0 } else { i.reverse_bits() >> (32 - bits) });
        RealFft { n, tw: tw.collect(), split: split.collect(), rev: rev.collect(), work: vec![Complex::default(); h].into_boxed_slice() }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    pub fn is_empty(&self) -> bool {
        self.n == 0
    }

    /// In-place iterative radix-2 FFT of `work` (inverse uses conjugated twiddles).
    fn complex(&mut self, inverse: bool) {
        let h = self.n / 2;
        let w = &mut self.work;
        for i in 0..h {
            let j = self.rev[i] as usize;
            if j > i {
                w.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= h {
            let half = len / 2;
            let stride = h / len;
            for start in (0..h).step_by(len) {
                for k in 0..half {
                    let t = self.tw[k * stride];
                    let t = if inverse { Complex::new(t.re, -t.im) } else { t };
                    let a = w[start + k];
                    let b = w[start + k + half].times(t);
                    w[start + k] = Complex::new(a.re + b.re, a.im + b.im);
                    w[start + k + half] = Complex::new(a.re - b.re, a.im - b.im);
                }
            }
            len *= 2;
        }
    }

    /// `input` (N reals) to `out` (N/2 + 1 bins).
    pub fn forward(&mut self, input: &[f32], out: &mut [Complex]) {
        let (n, h) = (self.n, self.n / 2);
        assert!(input.len() == n && out.len() == h + 1);
        for i in 0..h {
            self.work[i] = Complex::new(input[2 * i], input[2 * i + 1]);
        }
        self.complex(false);
        // split: X[k] = (Z[k] + conj(Z[h-k]))/2 - i·W^k·(Z[k] - conj(Z[h-k]))/2
        let z0 = self.work[0];
        out[0] = Complex::new(z0.re + z0.im, 0.0);
        out[h] = Complex::new(z0.re - z0.im, 0.0);
        for k in 1..h {
            let a = self.work[k];
            let b = self.work[h - k];
            let (er, ei) = (0.5 * (a.re + b.re), 0.5 * (a.im - b.im));
            let (or, oi) = (0.5 * (a.im + b.im), -0.5 * (a.re - b.re));
            let t = self.twiddle(k);
            out[k] = Complex::new(er + (or * t.re - oi * t.im), ei + (or * t.im + oi * t.re));
        }
    }

    /// `input` (N/2 + 1 bins) to `out` (N reals), unnormalized (a round trip scales by N).
    pub fn inverse(&mut self, input: &[Complex], out: &mut [f32]) {
        let (n, h) = (self.n, self.n / 2);
        assert!(input.len() == h + 1 && out.len() == n);
        for k in 0..h {
            let a = input[k];
            let b = Complex::new(input[h - k].re, -input[h - k].im);
            let e = Complex::new(a.re + b.re, a.im + b.im);
            let d = Complex::new(a.re - b.re, a.im - b.im);
            let t = self.twiddle(k);
            let tc = Complex::new(t.re, -t.im); // conj(W^k)
            let o = d.times(tc);
            // Z[k] = E + i·O
            self.work[k] = Complex::new(e.re - o.im, e.im + o.re);
        }
        self.complex(true);
        for i in 0..h {
            out[2 * i] = self.work[i].re;
            out[2 * i + 1] = self.work[i].im;
        }
    }

    /// W^k = e^(-2πik/N) for 0 ≤ k < N/2, from the quarter table.
    #[inline(always)]
    fn twiddle(&self, k: usize) -> Complex {
        let q = self.n / 4;
        if k <= q {
            self.split[k]
        } else {
            // e^(-2πik/N) = -conj(e^(-2πi(N/2-k)/N))
            let t = self.split[self.n / 2 - k];
            Complex::new(-t.re, t.im)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dft(x: &[f32]) -> Vec<(f64, f64)> {
        let n = x.len();
        (0..=n / 2)
            .map(|k| {
                let (mut re, mut im) = (0.0, 0.0);
                for (i, &v) in x.iter().enumerate() {
                    let a = -2.0 * PI * (k * i) as f64 / n as f64;
                    re += v as f64 * a.cos();
                    im += v as f64 * a.sin();
                }
                (re, im)
            })
            .collect()
    }

    #[test]
    fn forward_matches_the_dft_and_inverse_undoes_it() {
        let mut rng = crate::rng::Rng::new(4);
        for n in [4usize, 8, 64, 256, 2048] {
            let x: Vec<f32> = (0..n).map(|_| rng.next_f32() * 2.0 - 1.0).collect();
            let mut f = RealFft::new(n);
            let mut spec = vec![Complex::default(); n / 2 + 1];
            f.forward(&x, &mut spec);
            let want = dft(&x);
            let err = spec.iter().zip(&want).map(|(g, w)| (g.re as f64 - w.0).abs().max((g.im as f64 - w.1).abs())).fold(0.0, f64::max);
            assert!(err < 1e-3 * (n as f64).sqrt(), "n {n}: forward error {err}");
            let mut back = vec![0.0f32; n];
            f.inverse(&spec, &mut back);
            let err = x.iter().zip(&back).map(|(a, b)| (*a as f64 - *b as f64 / n as f64).abs()).fold(0.0, f64::max);
            assert!(err < 1e-5, "n {n}: round trip error {err}");
        }
    }
}
