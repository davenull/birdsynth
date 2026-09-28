//! Utility (gain, pan, width, mono bass, polarity, DC) and the Splitter's
//! band splitting (the band chains themselves run in fx/mod.rs).

use wt_dsp::filter::g_of;
use wt_dsp::math::balance;

use super::Ctx;
use super::util::{Block, DcBlock, Lr4, N, db, dc_pole};
use crate::spec::params as p;

pub struct Utility {
    split: [Lr4; 2],
    dc: [DcBlock; 2],
}

impl Default for Utility {
    fn default() -> Self {
        Utility::new()
    }
}

impl Utility {
    pub fn new() -> Utility {
        Utility { split: [Lr4::default(); 2], dc: [DcBlock::default(); 2] }
    }

    pub fn reset(&mut self) {
        *self = Utility::new();
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let gain = db(cx.p(p::FX_UTILITY_GAIN[i]));
        let (pl, pr) = balance(cx.p(p::FX_UTILITY_PAN[i]));
        let width = cx.p(p::FX_UTILITY_WIDTH[i]);
        let mono = cx.p(p::FX_UTILITY_MONO_BASS[i]) >= 0.5;
        let g = g_of(cx.p(p::FX_UTILITY_BASS_FREQ[i]), cx.sr);
        let inv = [cx.p(p::FX_UTILITY_INVERT_L[i]) >= 0.5, cx.p(p::FX_UTILITY_INVERT_R[i]) >= 0.5];
        let dc = cx.p(p::FX_UTILITY_DC[i]) >= 0.5;
        let r = dc_pole(5.0, cx.sr);
        for n in 0..N {
            let (mut l, mut rr) = (buf[0][n], buf[1][n]);
            if mono {
                let (lo_l, hi_l) = self.split[0].split(l, g);
                let (lo_r, hi_r) = self.split[1].split(rr, g);
                let lo = 0.5 * (lo_l + lo_r);
                l = lo + hi_l;
                rr = lo + hi_r;
            }
            let (mid, side) = (0.5 * (l + rr), 0.5 * (l - rr) * width);
            let mut out = [(mid + side) * gain * pl, (mid - side) * gain * pr];
            for ch in 0..2 {
                if inv[ch] {
                    out[ch] = -out[ch];
                }
                if dc {
                    out[ch] = self.dc[ch].tick(out[ch], r);
                }
                buf[ch][n] = out[ch];
            }
        }
        for s in self.split.iter_mut() {
            s.flush();
        }
        for d in self.dc.iter_mut() {
            d.flush();
        }
    }
}

pub const LOW_HIGH: u8 = 0;
pub const LOW_MID_HIGH: u8 = 1;
pub const MID_SIDE: u8 = 2;

pub struct Splitter {
    s1: [Lr4; 2],
    s2: [Lr4; 2],
    comp: [Lr4; 2],
}

impl Default for Splitter {
    fn default() -> Self {
        Splitter::new()
    }
}

impl Splitter {
    pub fn new() -> Splitter {
        Splitter { s1: [Lr4::default(); 2], s2: [Lr4::default(); 2], comp: [Lr4::default(); 2] }
    }

    pub fn reset(&mut self) {
        *self = Splitter::new();
    }

    /// How many bands the current mode makes.
    pub fn bands(cx: &Ctx, i: usize) -> usize {
        if cx.p(p::FX_SPLITTER_MODE[i]) as u8 == LOW_MID_HIGH { 3 } else { 2 }
    }

    /// Split `buf` into bands (they add back up flat).
    pub fn split(&mut self, cx: &Ctx, i: usize, buf: &Block, out: &mut [Block; 3]) {
        let mode = cx.p(p::FX_SPLITTER_MODE[i]) as u8;
        let f1 = cx.p(p::FX_SPLITTER_FREQ1[i]);
        let f2 = cx.p(p::FX_SPLITTER_FREQ2[i]).max(f1 * 1.1);
        let (g1, g2) = (g_of(f1, cx.sr), g_of(f2, cx.sr));
        for n in 0..N {
            match mode {
                MID_SIDE => {
                    let (m, s) = (0.5 * (buf[0][n] + buf[1][n]), 0.5 * (buf[0][n] - buf[1][n]));
                    out[0][0][n] = m;
                    out[0][1][n] = m;
                    out[1][0][n] = s;
                    out[1][1][n] = -s;
                }
                LOW_MID_HIGH => {
                    for ch in 0..2 {
                        let (lo, rest) = self.s1[ch].split(buf[ch][n], g1);
                        let (mid, high) = self.s2[ch].split(rest, g2);
                        let (a, b) = self.comp[ch].split(lo, g2);
                        out[0][ch][n] = a + b;
                        out[1][ch][n] = mid;
                        out[2][ch][n] = high;
                    }
                }
                _ => {
                    for ch in 0..2 {
                        let (lo, hi) = self.s1[ch].split(buf[ch][n], g1);
                        out[0][ch][n] = lo;
                        out[1][ch][n] = hi;
                    }
                }
            }
        }
        for s in self.s1.iter_mut().chain(self.s2.iter_mut()).chain(self.comp.iter_mut()) {
            s.flush();
        }
    }

    /// Add the bands back up with their gains.
    pub fn join(cx: &Ctx, i: usize, bands: &[Block; 3], n_bands: usize, buf: &mut Block) {
        let g = [db(cx.p(p::FX_SPLITTER_BAND1[i])), db(cx.p(p::FX_SPLITTER_BAND2[i])), db(cx.p(p::FX_SPLITTER_BAND3[i]))];
        for ch in 0..2 {
            for n in 0..N {
                let mut y = 0.0;
                for b in 0..n_bands {
                    y += bands[b][ch][n] * g[b];
                }
                buf[ch][n] = y;
            }
        }
    }
}
