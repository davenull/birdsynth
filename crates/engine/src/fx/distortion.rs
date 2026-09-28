//! Distortion: a waveshaper at 4× oversampling (two cascaded halfband
//! stages each way, 79 taps, about 90 dB of stopband), with an optional
//! filter before or after it, a DC bias for asymmetric distortion, and a DC
//! blocker. Dry and wet are mixed at the oversampled rate, so both take the
//! same path and stay aligned. Downsample mode is a sample-rate reducer and
//! runs at the base rate on purpose.

use wt_dsp::filter::{self as flt, FilterParams, FilterState, MAX_BLOCK};
use wt_dsp::math::{fast_tanh, sin_turns};
use wt_dsp::oversample::{Down2, Up2, design_taps};

use super::Ctx;
use super::util::{Block, DcBlock, N, db, dc_pole};
use crate::spec::params as p;

pub const TUBE: u8 = 0;
pub const SOFT_CLIP: u8 = 1;
pub const HARD_CLIP: u8 = 2;
pub const DIODE_1: u8 = 3;
pub const DIODE_2: u8 = 4;
pub const LIN_FOLD: u8 = 5;
pub const SIN_FOLD: u8 = 6;
pub const ZERO_SQUARE: u8 = 7;
pub const DOWNSAMPLE: u8 = 8;
pub const ASYM: u8 = 9;
pub const RECTIFY: u8 = 10;
pub const SINE_SHAPER: u8 = 11;
pub const STOMP: u8 = 12;
pub const TAPE: u8 = 13;
pub const OVERDRIVE: u8 = 14;

/// The distortion curves (the input already carries drive and bias).
#[inline(always)]
pub fn curve(mode: u8, x: f32) -> f32 {
    match mode {
        TUBE => fast_tanh(x + 0.25) - fast_tanh(0.25),
        SOFT_CLIP => fast_tanh(x),
        HARD_CLIP => x.clamp(-1.0, 1.0),
        DIODE_1 => {
            if x >= 0.0 {
                fast_tanh(x)
            } else {
                0.3 * fast_tanh(x)
            }
        }
        DIODE_2 => {
            if x >= 0.0 {
                fast_tanh(1.5 * x)
            } else {
                let t = fast_tanh(-x);
                -(t * t)
            }
        }
        LIN_FOLD => {
            let t = (x - 1.0) * 0.25;
            4.0 * (t - t.floor() - 0.5).abs() - 1.0
        }
        SIN_FOLD => sin_turns(0.25 * x),
        ZERO_SQUARE => x.signum() * (1.0 - (-4.0 * x.abs()).exp()),
        ASYM => {
            if x >= 0.0 {
                1.0 - (-x).exp()
            } else {
                -0.5 * (1.0 - (2.0 * x).exp())
            }
        }
        RECTIFY => fast_tanh(x.abs()),
        SINE_SHAPER => sin_turns(0.25 * (1.6 * fast_tanh(0.6 * x))),
        STOMP => fast_tanh(1.3 * x.clamp(-0.5, 1.0)),
        TAPE => x / (1.0 + x * x).sqrt(),
        OVERDRIVE => fast_tanh(1.5 * fast_tanh(x)) / fast_tanh(1.5),
        _ => x,
    }
}

const TAPS: usize = 79;
const BETA: f64 = 9.0;

pub struct Distortion {
    up: [[Up2; 2]; 2],
    down: [[Down2; 2]; 2],
    /// The odd sample kept for each decimator's next pair ([channel][stage]).
    prev: [[f32; 2]; 2],
    filt: Box<FilterState>,
    dc: [DcBlock; 2],
    hold: [f32; 2],
    hold_t: f32,
}

impl Default for Distortion {
    fn default() -> Self {
        Distortion::new()
    }
}

impl Distortion {
    pub fn new() -> Distortion {
        let h = design_taps(TAPS, BETA);
        Distortion {
            up: [[Up2::new(&h), Up2::new(&h)], [Up2::new(&h), Up2::new(&h)]],
            down: [[Down2::new(&h), Down2::new(&h)], [Down2::new(&h), Down2::new(&h)]],
            prev: [[0.0; 2]; 2],
            filt: Box::default(),
            dc: [DcBlock::default(); 2],
            hold: [0.0; 2],
            hold_t: 0.0,
        }
    }

    pub fn reset(&mut self) {
        for ch in 0..2 {
            for s in 0..2 {
                self.up[ch][s].reset();
                self.down[ch][s].reset();
            }
        }
        self.prev = [[0.0; 2]; 2];
        self.filt.reset();
        self.dc = [DcBlock::default(); 2];
        self.hold = [0.0; 2];
        self.hold_t = 0.0;
    }

    fn filter(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let kind = [flt::LOW12, flt::BAND12, flt::HIGH12][(cx.p(p::FX_DISTORTION_FILTER_TYPE[i]) as usize).min(2)];
        let key = if cx.p(p::FX_DISTORTION_KEYTRACK[i]) >= 0.5 { ((cx.note - 60.0) / 12.0).exp2() } else { 1.0 };
        let fp = FilterParams::new(kind, cx.p(p::FX_DISTORTION_FREQ[i]) * key, cx.p(p::FX_DISTORTION_Q[i]));
        let mut b = [[0.0f32; MAX_BLOCK]; 2];
        for ch in 0..2 {
            b[ch][..N].copy_from_slice(&buf[ch]);
        }
        self.filt.process(&fp, cx.sr, &mut b, 0, N);
        for ch in 0..2 {
            buf[ch].copy_from_slice(&b[ch][..N]);
        }
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let mode = cx.p(p::FX_DISTORTION_MODE[i]) as u8;
        let drive = cx.p(p::FX_DISTORTION_DRIVE[i]);
        let g = db(drive * 36.0);
        let bias = cx.p(p::FX_DISTORTION_BIAS[i]) * 0.5;
        let mix = cx.p(p::FX_DISTORTION_MIX[i]);
        let out = db(cx.p(p::FX_DISTORTION_OUTPUT[i]));
        let place = cx.p(p::FX_DISTORTION_FILTER[i]) as u8; // 0 off, 1 pre, 2 post
        if place == 1 {
            self.filter(cx, i, buf);
        }
        let r = dc_pole(5.0, cx.sr);
        if mode == DOWNSAMPLE {
            // hold each sample for sr / rate samples
            let step = 1.0 / (1.0 + drive * 63.0);
            for n in 0..N {
                self.hold_t += step;
                let take = self.hold_t >= 1.0;
                if take {
                    self.hold_t -= 1.0;
                }
                for ch in 0..2 {
                    if take {
                        self.hold[ch] = buf[ch][n];
                    }
                    let x = buf[ch][n];
                    buf[ch][n] = (x + (self.hold[ch] - x) * mix) * out;
                }
            }
        } else {
            for ch in 0..2 {
                let (up, down, prev) = (&mut self.up[ch], &mut self.down[ch], &mut self.prev[ch]);
                for n in 0..N {
                    let (a, b) = up[0].process(buf[ch][n]);
                    let mut half = [0.0f32; 2];
                    for (k, s) in [a, b].into_iter().enumerate() {
                        let (u0, u1) = up[1].process(s);
                        let w0 = u0 + (curve(mode, u0 * g + bias) - u0) * mix;
                        let w1 = u1 + (curve(mode, u1 * g + bias) - u1) * mix;
                        half[k] = down[1].process(prev[1], w0);
                        prev[1] = w1;
                    }
                    let y = down[0].process(prev[0], half[0]);
                    prev[0] = half[1];
                    buf[ch][n] = self.dc[ch].tick(y, r) * out;
                }
            }
        }
        if place == 2 {
            self.filter(cx, i, buf);
        }
        for d in self.dc.iter_mut() {
            d.flush();
        }
        self.filt.finish();
    }

    pub fn has_subnormal(&self) -> bool {
        self.filt.has_subnormal()
    }
}
