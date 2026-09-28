//! A frame as harmonics: amplitude and phase for each of the 1024 harmonics
//! a 2048-sample cycle can hold (index 0 is the DC offset). A frame
//! `a·sin(2π·k·t + φ)` reads back as `mag[k] = a`, `phase[k] = φ`, so the
//! editor's bins mean what they show.

use std::f32::consts::FRAC_PI_2;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};
use wt_dsp::mip::FRAME_LEN;

/// Harmonics per frame (plus DC at index 0).
pub const HARMONICS: usize = FRAME_LEN / 2;

pub struct Spectra {
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    buf: Vec<f32>,
    spec: Vec<Complex<f32>>,
}

impl Default for Spectra {
    fn default() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(FRAME_LEN);
        let inv = planner.plan_fft_inverse(FRAME_LEN);
        Spectra { spec: fwd.make_output_vec(), fwd, inv, buf: vec![0.0; FRAME_LEN] }
    }
}

impl Spectra {
    /// `frame` (2048 samples) into `mag` and `phase` (HARMONICS + 1 each).
    pub fn analyze(&mut self, frame: &[f32], mag: &mut [f32], phase: &mut [f32]) {
        self.buf.copy_from_slice(frame);
        self.fwd.process(&mut self.buf, &mut self.spec).expect("forward fft");
        let n = FRAME_LEN as f32;
        mag[0] = self.spec[0].re / n;
        phase[0] = 0.0;
        for k in 1..=HARMONICS {
            let c = self.spec[k];
            // the Nyquist bin holds cos only and isn't doubled
            let scale = if k == HARMONICS { 1.0 / n } else { 2.0 / n };
            mag[k] = c.norm() * scale;
            phase[k] = if mag[k] > 1e-9 { wrap(c.im.atan2(c.re) + FRAC_PI_2) } else { 0.0 };
        }
    }

    /// `mag` and `phase` back into a 2048-sample frame.
    pub fn synthesize(&mut self, mag: &[f32], phase: &[f32], frame: &mut [f32]) {
        let n = FRAME_LEN as f32;
        self.spec[0] = Complex::new(mag[0] * n, 0.0);
        for k in 1..=HARMONICS {
            let scale = if k == HARMONICS { n } else { n / 2.0 };
            let a = phase[k] - FRAC_PI_2;
            self.spec[k] = Complex::new(a.cos(), a.sin()) * (mag[k] * scale);
        }
        self.spec[HARMONICS].im = 0.0;
        self.inv.process(&mut self.spec, frame).expect("inverse fft");
        for v in frame.iter_mut() {
            *v /= n;
        }
    }
}

/// An angle into (-π, π].
pub fn wrap(a: f32) -> f32 {
    use std::f32::consts::{PI, TAU};
    let mut x = (a + PI).rem_euclid(TAU) - PI;
    if x <= -PI {
        x += TAU;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    #[test]
    fn a_sine_reads_back_as_its_amplitude_and_phase() {
        let mut s = Spectra::default();
        let (k, a, ph) = (7usize, 0.6f32, 0.9f32);
        let frame: Vec<f32> = (0..FRAME_LEN).map(|i| a * (TAU * k as f32 * i as f32 / FRAME_LEN as f32 + ph).sin() + 0.1).collect();
        let mut mag = vec![0.0; HARMONICS + 1];
        let mut phase = vec![0.0; HARMONICS + 1];
        s.analyze(&frame, &mut mag, &mut phase);
        assert!((mag[k] - a).abs() < 1e-5 && (phase[k] - ph).abs() < 1e-4, "{} {}", mag[k], phase[k]);
        assert!((mag[0] - 0.1).abs() < 1e-6);
        assert!(mag.iter().enumerate().all(|(i, &m)| i == 0 || i == k || m < 1e-5));
        let mut back = vec![0.0; FRAME_LEN];
        s.synthesize(&mag, &phase, &mut back);
        assert!(frame.iter().zip(&back).all(|(a, b)| (a - b).abs() < 1e-5));
    }
}
