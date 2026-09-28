//! EQ (three bands) and the FX Filter, both on the voice filter's cores, so
//! their responses are the exact ones `wt_dsp::filter::response` gives.

use wt_dsp::filter::{self as flt, FilterParams, FilterState, MAX_BLOCK};

use super::Ctx;
use super::util::{Block, N};
use crate::spec::params as p;

/// Filter parameters for an EQ band: `ty` 0 shelf, 1 peak, 2 cut.
pub fn band_params(low: bool, ty: u8, freq: f32, q: f32, gain_db: f32) -> FilterParams {
    let res_for_gain = gain_db / 36.0 + 0.5;
    match ty {
        0 => FilterParams { var: q, ..FilterParams::new(if low { 22 } else { 23 }, freq, res_for_gain) },
        1 => FilterParams { var: q, ..FilterParams::new(24, freq, res_for_gain) },
        _ => FilterParams::new(if low { flt::HIGH12 } else { flt::LOW12 }, freq, 0.9 * q),
    }
}

fn run(st: &mut FilterState, fp: &FilterParams, sr: f32, buf: &mut Block) {
    let mut b = [[0.0f32; MAX_BLOCK]; 2];
    for ch in 0..2 {
        b[ch][..N].copy_from_slice(&buf[ch]);
    }
    st.process(fp, sr, &mut b, 0, N);
    for ch in 0..2 {
        buf[ch].copy_from_slice(&b[ch][..N]);
    }
}

pub struct Eq {
    band: Vec<FilterState>,
}

impl Default for Eq {
    fn default() -> Self {
        Eq::new()
    }
}

impl Eq {
    pub fn new() -> Eq {
        Eq { band: vec![FilterState::default(); 3] }
    }

    pub fn reset(&mut self) {
        self.band.iter_mut().for_each(FilterState::reset);
    }

    /// The three bands' filter settings for instance `i`.
    pub fn settings(cx: &Ctx, i: usize) -> [FilterParams; 3] {
        [
            band_params(true, cx.p(p::FX_EQ_LOW_TYPE[i]) as u8, cx.p(p::FX_EQ_LOW_FREQ[i]), cx.p(p::FX_EQ_LOW_Q[i]), cx.p(p::FX_EQ_LOW_GAIN[i])),
            band_params(true, 1, cx.p(p::FX_EQ_MID_FREQ[i]), cx.p(p::FX_EQ_MID_Q[i]), cx.p(p::FX_EQ_MID_GAIN[i])),
            band_params(false, cx.p(p::FX_EQ_HIGH_TYPE[i]) as u8, cx.p(p::FX_EQ_HIGH_FREQ[i]), cx.p(p::FX_EQ_HIGH_Q[i]), cx.p(p::FX_EQ_HIGH_GAIN[i])),
        ]
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let s = Eq::settings(cx, i);
        for (st, fp) in self.band.iter_mut().zip(s.iter()) {
            run(st, fp, cx.sr, buf);
        }
    }
}

pub struct FxFilter {
    st: Box<FilterState>,
}

impl Default for FxFilter {
    fn default() -> Self {
        FxFilter::new()
    }
}

impl FxFilter {
    pub fn new() -> FxFilter {
        FxFilter { st: Box::default() }
    }

    pub fn reset(&mut self) {
        self.st.reset();
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let fp = FilterParams {
            kind: cx.p(p::FX_FILTER_TYPE[i]) as u8,
            cutoff: cx.p(p::FX_FILTER_CUTOFF[i]),
            res: cx.p(p::FX_FILTER_RES[i]),
            drive: cx.p(p::FX_FILTER_DRIVE[i]),
            mix: cx.p(p::FX_FILTER_MIX[i]),
            var: cx.p(p::FX_FILTER_VAR[i]),
            stereo: cx.p(p::FX_FILTER_STEREO[i]),
        };
        run(&mut self.st, &fp, cx.sr, buf);
    }

    pub fn has_subnormal(&self) -> bool {
        self.st.has_subnormal()
    }
}
