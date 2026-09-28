//! The sub oscillator: one band-limited shape, no unison. Saw, square and
//! pulse come from the built-in saw table (a square is a saw minus the same
//! saw half a cycle later), the triangle from its own table, and the sine
//! and rounded rectangle are computed directly.

use wt_dsp::math::{fast_tanh, sin_turns};
use wt_dsp::{mip, phase};

use crate::filter::MAX_N;

pub const SINE: u8 = 0;
pub const ROUND_RECT: u8 = 1;
pub const TRIANGLE: u8 = 2;
pub const SAW: u8 = 3;
pub const SQUARE: u8 = 4;
pub const PULSE: u8 = 5;

#[derive(Clone, Copy, Debug, Default)]
pub struct SubOsc {
    phase: u32,
    inc_prev: f32,
    primed: bool,
}

impl SubOsc {
    pub fn start(&mut self, phase: u32) {
        *self = SubOsc { phase, ..SubOsc::default() };
    }

    /// Render samples `start..len` of a block at sample rate `sr` into `out`.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, shape: u8, hz: f32, sr: f32, saw: &[f32], tri: &[f32], start: usize, len: usize, out: &mut [f32; MAX_N]) {
        let inc1 = (hz / sr).min(0.4999);
        let inc0 = if self.primed { self.inc_prev } else { inc1 };
        self.primed = true;
        self.inc_prev = inc1;
        let pick = mip::pick(inc0.max(inc1), sr);
        let mut ph = self.phase;
        let step = if inc0 > 0.0 && inc0 != inc1 { (inc1 / inc0).powf(1.0 / len as f32) } else { 1.0 };
        let mut inc_f = inc0;
        for _ in 0..start {
            inc_f *= step;
        }
        const HALF: u32 = 1 << 31;
        const QUARTER: u32 = 1 << 30;
        for v in out.iter_mut().take(len).skip(start) {
            let p = phase::cycles(ph);
            *v = match shape {
                SINE => sin_turns(p),
                ROUND_RECT => fast_tanh(3.0 * sin_turns(p)),
                TRIANGLE => mip::read_pick(tri, pick, ph),
                SAW => mip::read_pick(saw, pick, ph),
                SQUARE => 0.5 * (mip::read_pick(saw, pick, ph) - mip::read_pick(saw, pick, ph.wrapping_add(HALF))),
                _ => 0.5 * (mip::read_pick(saw, pick, ph) - mip::read_pick(saw, pick, ph.wrapping_add(QUARTER))),
            };
            ph = ph.wrapping_add((inc_f * 4_294_967_296.0) as i32 as u32);
            inc_f *= step;
        }
        self.phase = ph;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wt_dsp::saw;

    #[test]
    fn shapes_are_bounded_and_periodic() {
        let (sw, tr) = (saw::saw_frame(), saw::triangle_frame());
        for shape in 0..=PULSE {
            let mut o = SubOsc::default();
            let mut peak = 0.0f32;
            let mut out = [0.0f32; MAX_N];
            for _ in 0..3000 {
                o.render(shape, 110.0, 48_000.0, &sw, &tr, 0, 16, &mut out);
                for v in &out[..16] {
                    assert!(v.is_finite());
                    peak = peak.max(v.abs());
                }
            }
            assert!(peak > 0.4 && peak < 1.3, "shape {shape} peak {peak}");
        }
    }
}
