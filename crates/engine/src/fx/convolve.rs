//! Convolve: plays the sound through an impulse response with no latency.
//!
//! The response is split three ways, per channel:
//! - the head, taps [0, 64), as a direct FIR;
//! - level 1, taps [64, 2048), as 31 partitions of 64 (FFT 128), computed
//!   at every 64-sample boundary from the input so far;
//! - level 2, taps [2048, end), as partitions of 1024 (FFT 2048). A level-2
//!   output block only needs input from two blocks back, so its work is
//!   spread over the preceding block, one sub-block at a time: no spikes.
//!
//! The tools worker prepares everything that depends on the response (the
//! head and both levels' partition spectra, already scaled for the inverse
//! FFT) and uploads it as one asset, with room for level 2's input history
//! behind it, so loading an IR allocates nothing here.
//!
//! The asset layout is `wt_dsp::conv` (shared with the tools, which prepare it).

use wt_dsp::conv::{B1, B2, BINS1, BINS2, HEAD, P1};
pub use wt_dsp::conv::{channel_len, partitions2};
use wt_dsp::fft::{Complex as Complex32, RealFft};

use super::Ctx;
use super::util::{Block, DelayLine, Lp1, N, onepole_coef};
use crate::spec::params as p;
use crate::tables::AssetBuf;

pub use wt_dsp::conv::prepare;

const SLICES: usize = B2 / N;

struct Chan {
    /// Input history: 2 × B2 samples, doubled so any window is contiguous.
    hist: Box<[f32]>,
    fdl1: Box<[Complex32]>,
    /// Level-1 output for the current 64-sample block.
    y1: [f32; B1],
    /// Level-2 outputs: the block playing now, and the one being built.
    y2: Box<[f32]>,
    y2_next: Box<[f32]>,
    acc2: Box<[Complex32]>,
}

impl Chan {
    fn new() -> Chan {
        Chan {
            hist: vec![0.0; 4 * B2].into_boxed_slice(),
            fdl1: vec![Complex32::default(); P1 * BINS1].into_boxed_slice(),
            y1: [0.0; B1],
            y2: vec![0.0; B2].into_boxed_slice(),
            y2_next: vec![0.0; B2].into_boxed_slice(),
            acc2: vec![Complex32::default(); BINS2].into_boxed_slice(),
        }
    }

    fn clear(&mut self) {
        self.hist.fill(0.0);
        self.fdl1.fill(Complex32::default());
        self.y1 = [0.0; B1];
        self.y2.fill(0.0);
        self.y2_next.fill(0.0);
        self.acc2.fill(Complex32::default());
    }
}

pub struct Convolve {
    f1: RealFft,
    f2: RealFft,
    ir: Option<AssetBuf>,
    taps: usize,
    p2: usize,
    ch: [Chan; 2],
    /// Samples since the response was loaded (block positions follow it).
    n: usize,
    /// Write position in the input history.
    hpos: usize,
    fdl1_head: usize,
    fdl2_head: usize,
    // FFT scratch
    t1: Box<[f32]>,
    s1: Box<[Complex32]>,
    t2: Box<[f32]>,
    s2: Box<[Complex32]>,
    pre: [DelayLine; 2],
    lo: [Lp1; 2],
    hi: [Lp1; 2],
}

impl Convolve {
    pub fn new(sr: f32) -> Convolve {
        let pre_len = (0.26 * sr) as usize + 8;
        Convolve {
            f1: RealFft::new(2 * B1),
            f2: RealFft::new(2 * B2),
            ir: None,
            taps: 0,
            p2: 0,
            ch: [Chan::new(), Chan::new()],
            n: 0,
            hpos: 0,
            fdl1_head: 0,
            fdl2_head: 0,
            t1: vec![0.0; 2 * B1].into_boxed_slice(),
            s1: vec![Complex32::default(); BINS1].into_boxed_slice(),
            t2: vec![0.0; 2 * B2].into_boxed_slice(),
            s2: vec![Complex32::default(); BINS2].into_boxed_slice(),
            pre: [DelayLine::new(pre_len), DelayLine::new(pre_len)],
            lo: [Lp1::default(); 2],
            hi: [Lp1::default(); 2],
        }
    }

    pub fn taps(&self) -> usize {
        self.taps
    }

    /// Take a prepared response (see the module docs); `taps` 0 or a bad size removes it.
    pub fn load(&mut self, asset: Option<AssetBuf>, taps: usize) -> bool {
        let ok = asset.as_ref().is_some_and(|a| a.bytes() == 2 * channel_len(taps) * 4);
        self.ir = if ok { asset } else { None };
        self.taps = if ok { taps } else { 0 };
        self.p2 = partitions2(self.taps);
        self.reset();
        ok || taps == 0
    }

    pub fn reset(&mut self) {
        for c in self.ch.iter_mut() {
            c.clear();
        }
        if let Some(a) = self.ir.as_mut() {
            // zero the level-2 input history stored in the asset
            let cl = channel_len(self.taps);
            let fdl = HEAD + P1 * BINS1 * 2 + self.p2 * BINS2 * 2;
            let v = a.as_f32_mut();
            for c in 0..2 {
                v[c * cl + fdl..(c + 1) * cl].fill(0.0);
            }
        }
        self.n = 0;
        self.hpos = 0;
        self.fdl1_head = 0;
        self.fdl2_head = 0;
        self.pre.iter_mut().for_each(DelayLine::clear);
        self.lo = [Lp1::default(); 2];
        self.hi = [Lp1::default(); 2];
    }

    /// The last `len` input samples of channel `c`, oldest first.
    fn window(hist: &[f32], hpos: usize, len: usize) -> &[f32] {
        // hist holds 2·B2 samples twice over; hpos is where the next sample goes
        let start = hpos + 2 * B2 - len;
        &hist[start..start + len]
    }

    fn level1(&mut self, c: usize, ir: &[f32]) {
        let w = Convolve::window(&self.ch[c].hist, self.hpos, 2 * B1);
        self.t1.copy_from_slice(w);
        self.f1.forward(&self.t1, &mut self.s1);
        let head = self.fdl1_head;
        let ch = &mut self.ch[c];
        ch.fdl1[head * BINS1..(head + 1) * BINS1].copy_from_slice(&self.s1);
        let h1 = &ir[HEAD..HEAD + P1 * BINS1 * 2];
        let mut acc = [Complex32::default(); BINS1];
        for p in 0..P1 {
            // FDL slot holding X_{k-1-p}
            let slot = (head + P1 - p) % P1;
            let x = &ch.fdl1[slot * BINS1..(slot + 1) * BINS1];
            let h = &h1[p * BINS1 * 2..(p + 1) * BINS1 * 2];
            for b in 0..BINS1 {
                acc[b].mac(x[b], Complex32::new(h[2 * b], h[2 * b + 1]));
            }
        }
        self.s1.copy_from_slice(&acc);
        self.f1.inverse(&self.s1, &mut self.t1);
        ch.y1.copy_from_slice(&self.t1[B1..]);
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let pre = cx.p(p::FX_CONVOLVE_PREDELAY[i]) * cx.sr / 1000.0;
        // the cuts switch off entirely at the ends of their ranges
        let lo_f = cx.p(p::FX_CONVOLVE_LOWCUT[i]);
        let hi_f = cx.p(p::FX_CONVOLVE_HIGHCUT[i]);
        let a_lo = if lo_f > 20.5 { onepole_coef(lo_f, cx.sr) } else { 0.0 };
        let a_hi = if hi_f < 19_900.0 { onepole_coef(hi_f.min(cx.sr * 0.45), cx.sr) } else { 1.0 };
        let width = cx.p(p::FX_CONVOLVE_WIDTH[i]);
        let mix = cx.p(p::FX_CONVOLVE_MIX[i]);
        let Some(mut ir_buf) = self.ir.take() else {
            // no response: the wet signal is silence
            for ch in buf.iter_mut() {
                for v in ch.iter_mut() {
                    *v *= 1.0 - mix;
                }
            }
            return;
        };
        let cl = channel_len(self.taps);
        let p2 = self.p2;
        let mut wet = [[0.0f32; N]; 2];
        // --- block-boundary work (sub-blocks are aligned: 64 and 1024 are multiples of 16)
        let slice = (self.n % B2) / N;
        let new_l2 = self.n.is_multiple_of(B2);
        if self.n.is_multiple_of(B1) && self.n > 0 {
            self.fdl1_head = (self.fdl1_head + 1) % P1;
            for c in 0..2 {
                let ir = &ir_buf.as_f32()[c * cl..(c + 1) * cl];
                self.level1(c, ir);
            }
        }
        if p2 > 0 {
            let v = ir_buf.as_f32_mut();
            for c in 0..2 {
                let base = c * cl;
                let h2 = base + HEAD + P1 * BINS1 * 2;
                let fdl = h2 + p2 * BINS2 * 2;
                let ch = &mut self.ch[c];
                if new_l2 && self.n > 0 {
                    // the block being built becomes the one playing; start the next
                    std::mem::swap(&mut ch.y2, &mut ch.y2_next);
                    let w = Convolve::window(&ch.hist, self.hpos, 2 * B2);
                    self.t2.copy_from_slice(w);
                    self.f2.forward(&self.t2, &mut self.s2);
                    if c == 0 {
                        self.fdl2_head = (self.fdl2_head + 1) % p2;
                    }
                    let slot = self.fdl2_head;
                    for b in 0..BINS2 {
                        v[fdl + (slot * BINS2 + b) * 2] = self.s2[b].re;
                        v[fdl + (slot * BINS2 + b) * 2 + 1] = self.s2[b].im;
                    }
                    ch.acc2.fill(Complex32::default());
                }
                // this sub-block's share of the partitions
                let (q0, q1) = (slice * p2 / SLICES, (slice + 1) * p2 / SLICES);
                for q in q0..q1 {
                    let slot = (self.fdl2_head + p2 - q) % p2;
                    for b in 0..BINS2 {
                        let xo = fdl + (slot * BINS2 + b) * 2;
                        let ho = h2 + (q * BINS2 + b) * 2;
                        ch.acc2[b].mac(Complex32::new(v[xo], v[xo + 1]), Complex32::new(v[ho], v[ho + 1]));
                    }
                }
                if slice == SLICES - 1 {
                    self.s2.copy_from_slice(&ch.acc2);
                    self.f2.inverse(&self.s2, &mut self.t2);
                    ch.y2_next.copy_from_slice(&self.t2[B2..]);
                }
            }
        }
        // --- per sample: history, head FIR, and the two levels' outputs
        let off1 = self.n % B1;
        let off2 = self.n % B2;
        for c in 0..2 {
            let ir = &ir_buf.as_f32()[c * cl..(c + 1) * cl];
            let head = &ir[..HEAD];
            let ch = &mut self.ch[c];
            let mut pos = self.hpos;
            for s in 0..N {
                let x = buf[c][s];
                ch.hist[pos] = x;
                ch.hist[pos + 2 * B2] = x;
                pos = (pos + 1) % (2 * B2);
                // head: newest sample first
                let w = &ch.hist[pos + 2 * B2 - HEAD..pos + 2 * B2];
                let mut acc = 0.0;
                for t in 0..HEAD {
                    acc += head[t] * w[HEAD - 1 - t];
                }
                let y = acc + ch.y1[off1 + s] + if p2 > 0 { ch.y2[off2 + s] } else { 0.0 };
                wet[c][s] = y;
            }
        }
        self.hpos = (self.hpos + N) % (2 * B2);
        self.n += N;
        self.ir = Some(ir_buf);
        // --- wet path: pre-delay, low and high cuts, width, mix
        for s in 0..N {
            let mut w = [0.0f32; 2];
            for c in 0..2 {
                self.pre[c].write(wet[c][s]);
                let d = if pre > 0.0 { self.pre[c].read(pre) } else { wet[c][s] };
                let h = self.hi[c].tick(d, a_hi);
                w[c] = h - self.lo[c].tick(h, a_lo);
            }
            let (m, sd) = (0.5 * (w[0] + w[1]), 0.5 * (w[0] - w[1]) * width);
            let out = [m + sd, m - sd];
            for c in 0..2 {
                let x = buf[c][s];
                buf[c][s] = x + (out[c] - x) * mix;
            }
        }
        for f in self.lo.iter_mut().chain(self.hi.iter_mut()) {
            f.flush();
        }
    }
}
