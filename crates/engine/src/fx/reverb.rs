//! Reverb: one feedback-delay-network core, voiced five ways. Each
//! algorithm sets the delay lines (how many, how long), the input diffusion,
//! the damping and the movement:
//! - Hall: eight lines, smooth and even.
//! - Plate: eight short lines behind heavy diffusion: dense and bright.
//! - Vintage: four lines and a slightly coarse tank, like early digital units.
//! - Bloom: a long diffusion chain in front, so the tail swells in.
//! - Basin: sixteen long lines, darker: huge.
//!
//! Every line's feedback gain is set from its length so the tail falls 60 dB
//! in the Decay time; the Householder mixing matrix is orthogonal, so it
//! keeps that rate.

use wt_dsp::math::sin_turns;

use super::Ctx;
use super::util::{Block, DelayLine, Lp1, N, onepole_coef};
use crate::spec::params as p;

pub const HALL: u8 = 0;
pub const PLATE: u8 = 1;
pub const VINTAGE: u8 = 2;
pub const BLOOM: u8 = 3;
pub const BASIN: u8 = 4;

const MAX_LINES: usize = 16;
const MAX_DIFF: usize = 6;

struct Voicing {
    lines: &'static [f32],
    diffuse: &'static [f32],
    diffuse_g: f32,
    bloom: &'static [f32],
    damp: f32,
    movement_ms: f32,
    grain: bool,
}

const VOICINGS: [Voicing; 5] = [
    Voicing { lines: &[29.7, 37.1, 41.1, 43.7, 53.3, 59.9, 67.7, 73.1], diffuse: &[4.7, 3.6, 12.7, 9.3], diffuse_g: 0.7, bloom: &[], damp: 1.0, movement_ms: 0.35, grain: false },
    Voicing { lines: &[11.3, 13.7, 17.9, 19.3, 23.9, 26.1, 31.3, 35.9], diffuse: &[1.4, 2.1, 3.9, 5.6, 7.3, 9.9], diffuse_g: 0.75, bloom: &[], damp: 1.3, movement_ms: 0.2, grain: false },
    Voicing { lines: &[23.1, 31.7, 39.1, 47.9], diffuse: &[5.3, 7.9], diffuse_g: 0.6, bloom: &[], damp: 1.6, movement_ms: 0.1, grain: true },
    Voicing { lines: &[35.6, 44.5, 49.3, 52.4, 64.0, 71.9, 81.2, 87.7], diffuse: &[4.7, 3.6, 12.7, 9.3], diffuse_g: 0.7, bloom: &[13.1, 19.3, 26.9, 33.7, 41.3, 47.9], damp: 1.0, movement_ms: 0.5, grain: false },
    Voicing {
        lines: &[47.1, 53.9, 61.7, 67.3, 73.9, 79.1, 87.3, 93.7, 101.9, 109.3, 117.7, 125.1, 133.9, 141.3, 151.7, 163.1],
        diffuse: &[6.1, 8.9, 13.3, 17.1],
        diffuse_g: 0.7,
        bloom: &[],
        damp: 0.6,
        movement_ms: 0.6,
        grain: false,
    },
];

/// Delay-length multiplier for the Size control.
#[inline]
fn size_scale(size: f32) -> f32 {
    0.4 + 1.2 * size
}

/// A Schroeder allpass on a delay line.
struct Allpass {
    line: DelayLine,
}

impl Allpass {
    fn new(len: usize) -> Allpass {
        Allpass { line: DelayLine::new(len) }
    }

    #[inline(always)]
    fn tick(&mut self, x: f32, d: f32, g: f32) -> f32 {
        let tap = self.line.read(d);
        let v = x + g * tap;
        self.line.write(v);
        tap - g * v
    }
}

pub struct Reverb {
    pre: [DelayLine; 2],
    diff: [[Allpass; MAX_DIFF]; 2],
    bloom: [[Allpass; MAX_DIFF]; 2],
    lines: [DelayLine; MAX_LINES],
    damp: [Lp1; MAX_LINES],
    hp: [Lp1; 2],
    mphase: [f64; 4],
}

impl Reverb {
    pub fn new(sr: f32) -> Reverb {
        let ms = sr / 1000.0;
        let max_line = VOICINGS.iter().flat_map(|v| v.lines.iter()).fold(0.0f32, |m, &l| m.max(l));
        let line_len = ((max_line * size_scale(1.0) + 3.0) * ms) as usize + 8;
        let diff_len = ((VOICINGS.iter().flat_map(|v| v.diffuse.iter().chain(v.bloom.iter())).fold(0.0f32, |m, &l| m.max(l)) * size_scale(1.0)) * ms) as usize + 8;
        let pre_len = (0.26 * sr) as usize + 8;
        Reverb {
            pre: [DelayLine::new(pre_len), DelayLine::new(pre_len)],
            diff: std::array::from_fn(|_| std::array::from_fn(|_| Allpass::new(diff_len))),
            bloom: std::array::from_fn(|_| std::array::from_fn(|_| Allpass::new(diff_len))),
            lines: std::array::from_fn(|_| DelayLine::new(line_len)),
            damp: [Lp1::default(); MAX_LINES],
            hp: [Lp1::default(); 2],
            mphase: [0.0; 4],
        }
    }

    pub fn reset(&mut self) {
        self.pre.iter_mut().for_each(DelayLine::clear);
        for a in self.diff.iter_mut().chain(self.bloom.iter_mut()).flatten() {
            a.line.clear();
        }
        self.lines.iter_mut().for_each(DelayLine::clear);
        self.damp = [Lp1::default(); MAX_LINES];
        self.hp = [Lp1::default(); 2];
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let algo = (cx.p(p::FX_REVERB_ALGO[i]) as usize).min(VOICINGS.len() - 1);
        let v = &VOICINGS[algo];
        let sr = cx.sr;
        let ms = sr / 1000.0;
        let scale = size_scale(cx.p(p::FX_REVERB_SIZE[i]));
        let rt60 = cx.p(p::FX_REVERB_DECAY[i]).max(0.05);
        let pre = (cx.p(p::FX_REVERB_PREDELAY[i]) * ms).max(0.0);
        let freeze = cx.p(p::FX_REVERB_FREEZE[i]) >= 0.5;
        // damping at the top of its range is off entirely
        let damp_f = cx.p(p::FX_REVERB_DAMP[i]);
        let a_damp = if freeze || damp_f >= 19_900.0 { 1.0 } else { onepole_coef((damp_f * v.damp).min(sr * 0.45), sr) };
        let lo_f = cx.p(p::FX_REVERB_LOWCUT[i]);
        let a_hp = if lo_f > 20.5 { onepole_coef(lo_f, sr) } else { 0.0 };
        let width = cx.p(p::FX_REVERB_WIDTH[i]);
        let depth = cx.p(p::FX_REVERB_MOVEMENT[i]) * v.movement_ms * ms;
        let mix = cx.p(p::FX_REVERB_MIX[i]);
        let n = v.lines.len();
        let mut len = [0.0f32; MAX_LINES];
        let mut gain = [0.0f32; MAX_LINES];
        for k in 0..n {
            // whole samples: a fractional read is a low-pass on every pass round the loop
            len[k] = (v.lines[k] * scale * ms).round();
            // the loop is the read delay plus the movement offset plus the sample between read and write
            let lap = len[k] + if depth > 0.0 { depth } else { 0.0 } + 1.0;
            gain[k] = if freeze { 1.0 } else { 10f32.powf(-3.0 * lap / (sr * rt60)) };
        }
        let dg = v.diffuse_g;
        let dl: [f32; MAX_DIFF] = std::array::from_fn(|k| v.diffuse.get(k).map_or(0.0, |d| d * scale * ms));
        let bl: [f32; MAX_DIFF] = std::array::from_fn(|k| v.bloom.get(k).map_or(0.0, |d| d * scale * ms));
        let rates = [0.31, 0.43, 0.57, 0.71];
        let dt = N as f64 / sr as f64;
        let m0 = self.mphase;
        for (k, ph) in self.mphase.iter_mut().enumerate() {
            *ph = (*ph + rates[k] * dt).fract();
        }
        let inject = 1.0 / (n as f32 / 2.0).sqrt();
        let out_scale = 0.7 / (n as f32 / 2.0).sqrt();
        let house = 2.0 / n as f32;
        for s in 0..N {
            let t = (s + 1) as f64 / N as f64;
            let mut input = [0.0f32; 2];
            for ch in 0..2 {
                self.pre[ch].write(buf[ch][s]);
                let mut x = if freeze { 0.0 } else { self.pre[ch].read(pre) };
                for (k, a) in self.diff[ch].iter_mut().enumerate().take(v.diffuse.len()) {
                    x = a.tick(x, dl[k], dg);
                }
                for (k, a) in self.bloom[ch].iter_mut().enumerate().take(v.bloom.len()) {
                    x = a.tick(x, bl[k], 0.5);
                }
                input[ch] = x * inject;
            }
            // read, damp, mix (Householder), feed back
            let mut o = [0.0f32; MAX_LINES];
            let mut sum = 0.0;
            for k in 0..n {
                let m = if k < 4 { depth * sin_turns(((m0[k] + (self.mphase[k] - m0[k]).rem_euclid(1.0) * t).fract()) as f32) } else { 0.0 };
                let raw = if depth > 0.0 { self.lines[k].read_cubic(len[k] + m + depth) } else { self.lines[k].read(len[k]) };
                o[k] = self.damp[k].tick(raw, a_damp);
                sum += o[k];
            }
            let fold = house * sum;
            let (mut l, mut r) = (0.0f32, 0.0f32);
            for k in 0..n {
                let w = gain[k] * (o[k] - fold) + input[k % 2];
                self.lines[k].write(w);
                let sign = if (k / 2) % 2 == 0 { 1.0 } else { -1.0 };
                if k % 2 == 0 {
                    l += sign * o[k];
                } else {
                    r += sign * o[k];
                }
            }
            let (mut l, mut r) = (l * out_scale, r * out_scale);
            if v.grain {
                // an early converter's resolution, on the output (in the loop it would cut the tail short)
                l = (l * 4096.0).round() * (1.0 / 4096.0);
                r = (r * 4096.0).round() * (1.0 / 4096.0);
            }
            let (mid, side) = (0.5 * (l + r), 0.5 * (l - r) * width);
            let wet = [mid + side, mid - side];
            for ch in 0..2 {
                let w = wet[ch] - self.hp[ch].tick(wet[ch], a_hp);
                let x = buf[ch][s];
                buf[ch][s] = x + (w - x) * mix;
            }
        }
        for f in self.damp.iter_mut().chain(self.hp.iter_mut()) {
            f.flush();
        }
    }

    pub fn has_subnormal(&self) -> bool {
        self.lines.iter().chain(self.pre.iter()).any(DelayLine::has_subnormal) || self.diff.iter().chain(self.bloom.iter()).flatten().any(|a| a.line.has_subnormal())
    }
}
