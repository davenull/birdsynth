//! Frequency shifter: a Hilbert pair makes the signal's analytic form, which
//! is multiplied by a complex oscillator at the shift frequency. Every
//! partial moves by the same number of Hz (up for a positive shift).

use wt_dsp::hilbert::Hilbert;
use wt_dsp::math::flush;

use super::Ctx;
use super::util::{Block, N};
use crate::spec::params as p;

pub struct Bode {
    hilbert: [Hilbert; 2],
    phase: [f64; 2],
    fb: [f32; 2],
}

impl Default for Bode {
    fn default() -> Self {
        Bode::new()
    }
}

impl Bode {
    pub fn new() -> Bode {
        Bode { hilbert: [Hilbert::default(); 2], phase: [0.0; 2], fb: [0.0; 2] }
    }

    pub fn reset(&mut self) {
        *self = Bode::new();
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let shift = cx.p(p::FX_BODE_SHIFT[i]) as f64;
        let fb = cx.p(p::FX_BODE_FEEDBACK[i]);
        let spread = cx.p(p::FX_BODE_SPREAD[i]) as f64;
        let mix = cx.p(p::FX_BODE_MIX[i]);
        let inc = [shift / cx.sr as f64, shift * (1.0 - 2.0 * spread) / cx.sr as f64];
        for ch in 0..2 {
            for n in 0..N {
                let x = buf[ch][n];
                let (re, im) = self.hilbert[ch].tick(x + fb * self.fb[ch]);
                let ph = self.phase[ch] * std::f64::consts::TAU;
                // this sign keeps the upper sideband: partials move up for a positive shift
                let y = (re as f64 * ph.cos() + im as f64 * ph.sin()) as f32;
                self.phase[ch] = (self.phase[ch] + inc[ch]).rem_euclid(1.0);
                self.fb[ch] = y;
                buf[ch][n] = x + (y - x) * mix;
            }
            self.hilbert[ch].flush();
            self.fb[ch] = flush(self.fb[ch]);
        }
    }
}
