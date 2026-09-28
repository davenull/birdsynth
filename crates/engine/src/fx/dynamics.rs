//! Compressor: single-band (a peak detector in dB with attack and release,
//! and a soft-knee static curve), or three bands on Linkwitz-Riley
//! crossovers, each compressed downward above the threshold and upward
//! below it (the "OTT" sound).

use wt_dsp::filter::g_of;

use super::Ctx;
use super::util::{Block, Lr4, N, db};
use crate::spec::params as p;

const FLOOR_DB: f32 = -120.0;
const FLOOR: f32 = 1e-6;

/// Gain change (dB, ≤ 0) of a downward compressor with a soft knee.
#[inline]
pub fn static_curve(level_db: f32, threshold: f32, ratio: f32, knee: f32) -> f32 {
    let over = level_db - threshold;
    let slope = 1.0 / ratio.max(1.0) - 1.0;
    if knee > 0.0 && 2.0 * over.abs() <= knee {
        slope * (over + 0.5 * knee).powi(2) / (2.0 * knee)
    } else if over > 0.0 {
        slope * over
    } else {
        0.0
    }
}

/// Attack/release coefficients for times in ms.
#[inline]
fn coef(ms: f32, sr: f32) -> f32 {
    1.0 - (-1.0 / (ms.max(0.01) * 0.001 * sr)).exp()
}

pub struct Compressor {
    /// Peak-hold level per band (linear): instant attack, released over the release time.
    env: [f32; 3],
    /// Smoothed gain change per band (dB): attack and release act here, so a
    /// steady signal sits exactly on the static curve.
    smooth: [f32; 3],
    split1: [Lr4; 2],
    split2: [Lr4; 2],
    comp: [Lr4; 2],
    /// Latest gain change per band (dB), for the meters.
    pub gr: [f32; 3],
}

impl Default for Compressor {
    fn default() -> Self {
        Compressor::new()
    }
}

impl Compressor {
    pub fn new() -> Compressor {
        Compressor { env: [FLOOR; 3], smooth: [0.0; 3], split1: [Lr4::default(); 2], split2: [Lr4::default(); 2], comp: [Lr4::default(); 2], gr: [0.0; 3] }
    }

    pub fn reset(&mut self) {
        *self = Compressor::new();
    }

    /// Peak-hold level in dB: jumps to rises, falls over the release time.
    #[inline(always)]
    fn follow(env: &mut f32, x: f32, rel: f32) -> f32 {
        *env = if x > *env { x } else { (*env - *env * rel).max(FLOOR) };
        20.0 * env.log10()
    }

    /// Smooth a gain change: attack while reducing more, release while recovering.
    #[inline(always)]
    fn ballistics(g: &mut f32, target: f32, att: f32, rel: f32) -> f32 {
        let a = if target < *g { att } else { rel };
        *g += (target - *g) * a;
        *g
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let multiband = cx.p(p::FX_COMPRESSOR_MODE[i]) >= 0.5;
        let thr = cx.p(p::FX_COMPRESSOR_THRESHOLD[i]);
        let ratio = cx.p(p::FX_COMPRESSOR_RATIO[i]);
        let knee = cx.p(p::FX_COMPRESSOR_KNEE[i]);
        let makeup = cx.p(p::FX_COMPRESSOR_GAIN[i]);
        let mix = cx.p(p::FX_COMPRESSOR_MIX[i]);
        let mut att_ms = cx.p(p::FX_COMPRESSOR_ATTACK[i]);
        let mut rel_ms = cx.p(p::FX_COMPRESSOR_RELEASE[i]);
        if !multiband {
            let (att, rel) = (coef(att_ms, cx.sr), coef(rel_ms, cx.sr));
            for n in 0..N {
                let x = buf[0][n].abs().max(buf[1][n].abs());
                let env = Compressor::follow(&mut self.env[0], x, rel);
                let gr = Compressor::ballistics(&mut self.smooth[0], static_curve(env, thr, ratio, knee), att, rel);
                self.gr[0] = gr;
                let g = db(gr + makeup);
                for ch in 0..2 {
                    let d = buf[ch][n];
                    buf[ch][n] = d + (d * g - d) * mix;
                }
            }
            self.gr[1] = 0.0;
            self.gr[2] = 0.0;
            return;
        }
        let depth = cx.p(p::FX_COMPRESSOR_DEPTH[i]);
        let up = cx.p(p::FX_COMPRESSOR_UPWARD[i]);
        let down = cx.p(p::FX_COMPRESSOR_DOWNWARD[i]);
        let scale = 4f32.powf(2.0 * cx.p(p::FX_COMPRESSOR_TIME[i]) - 1.0);
        att_ms *= scale;
        rel_ms *= scale;
        let (att, rel) = (coef(att_ms, cx.sr), coef(rel_ms, cx.sr));
        let g1 = g_of(cx.p(p::FX_COMPRESSOR_XOVER1[i]), cx.sr);
        let g2 = g_of(cx.p(p::FX_COMPRESSOR_XOVER2[i]).max(cx.p(p::FX_COMPRESSOR_XOVER1[i]) * 1.1), cx.sr);
        let band_gain = [cx.p(p::FX_COMPRESSOR_LOW[i]), cx.p(p::FX_COMPRESSOR_MID[i]), cx.p(p::FX_COMPRESSOR_HIGH[i])];
        for n in 0..N {
            let mut bands = [[0.0f32; 2]; 3];
            for ch in 0..2 {
                let x = buf[ch][n];
                let (lo, rest) = self.split1[ch].split(x, g1);
                let (mid, high) = self.split2[ch].split(rest, g2);
                // keep the low band in phase with the others: an allpass at the second split
                let (a, b) = self.comp[ch].split(lo, g2);
                bands[0][ch] = a + b;
                bands[1][ch] = mid;
                bands[2][ch] = high;
            }
            let mut out = [0.0f32; 2];
            for b in 0..3 {
                let x = bands[b][0].abs().max(bands[b][1].abs());
                let env = Compressor::follow(&mut self.env[b], x, rel);
                let over = env - thr;
                let target = if over > 0.0 {
                    (1.0 / ratio - 1.0) * over * down
                } else if env > FLOOR_DB + 30.0 {
                    ((1.0 - 1.0 / ratio) * -over * up).min(24.0)
                } else {
                    0.0
                };
                let gr = Compressor::ballistics(&mut self.smooth[b], target * depth, att, rel);
                self.gr[b] = gr;
                let g = db(gr + band_gain[b] + makeup);
                for ch in 0..2 {
                    out[ch] += bands[b][ch] * g;
                }
            }
            for ch in 0..2 {
                let d = buf[ch][n];
                buf[ch][n] = d + (out[ch] - d) * mix;
            }
        }
        for s in self.split1.iter_mut().chain(self.split2.iter_mut()).chain(self.comp.iter_mut()) {
            s.flush();
        }
    }
}
