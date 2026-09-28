//! The voice filter. P1 has two cores: a TPT state-variable filter (12 and
//! 24 dB low/high/band/notch) and a TPT four-stage ladder. Both are exact
//! bilinear transforms of their analog prototypes (with the cutoff
//! prewarped), so `response` gives their true digital magnitude response.
//! Type numbers match `filter.*.type` in params/filter.toml.

use std::f32::consts::{PI, SQRT_2};

use wt_dsp::math::{fast_tanh, flush};

use crate::spec::protocol::SUB_BLOCK as N;

pub const LOW12: u8 = 0;
pub const LOW24: u8 = 1;
pub const HIGH12: u8 = 2;
pub const HIGH24: u8 = 3;
pub const BAND12: u8 = 4;
pub const BAND24: u8 = 5;
pub const NOTCH12: u8 = 6;
pub const NOTCH24: u8 = 7;
pub const LADDER6: u8 = 8;
pub const LADDER12: u8 = 9;
pub const LADDER18: u8 = 10;
pub const LADDER24: u8 = 11;
pub const LAST_TYPE: u8 = LADDER24;

#[inline]
fn is_ladder(t: u8) -> bool {
    (LADDER6..=LADDER24).contains(&t)
}

/// SVF damping: Butterworth (k = √2) at no resonance, nearly self-oscillating at full.
#[inline]
pub fn svf_k(res: f32) -> f32 {
    SQRT_2 * (1.0 - res) + 0.02 * res
}

/// Ladder feedback: 0 to just under the self-oscillation point (4).
#[inline]
pub fn ladder_k(res: f32) -> f32 {
    3.98 * res
}

/// Ladder output gain: part of the bass the resonance takes is given back.
#[inline]
pub fn ladder_comp(k: f32) -> f32 {
    1.0 + 0.5 * k
}

/// Prewarped integrator gain for a cutoff.
#[inline]
pub fn g_of(cutoff: f32, sr: f32) -> f32 {
    (PI * cutoff.clamp(5.0, sr * 0.49) / sr).tan()
}

#[derive(Clone, Copy, Debug)]
pub struct FilterParams {
    pub kind: u8,
    pub cutoff: f32,
    pub res: f32,
    pub drive: f32,
    pub mix: f32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// One sample. Returns (low, band, high).
    #[inline(always)]
    fn tick(&mut self, x: f32, g: f32, k: f32, a1: f32) -> (f32, f32, f32) {
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1, x - k * v1 - v2)
    }

    fn flush(&mut self) {
        self.ic1 = flush(self.ic1);
        self.ic2 = flush(self.ic2);
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Ladder {
    s: [f32; 4],
}

impl Ladder {
    /// One sample through four TPT one-poles with zero-delay feedback k.
    /// `gg` is g/(1+g). Returns the four stage outputs.
    #[inline(always)]
    fn tick(&mut self, x: f32, gg: f32, k: f32) -> [f32; 4] {
        let b = 1.0 - gg;
        let s = &mut self.s;
        // y4 = G^4 u + S, with u = x - k y4, solved for y4
        let g2 = gg * gg;
        let g4 = g2 * g2;
        let st = gg * g2 * b * s[0] + g2 * b * s[1] + gg * b * s[2] + b * s[3];
        let y4 = (g4 * x + st) / (1.0 + k * g4);
        // the saturation sits where the feedback meets the input; it is
        // transparent at small levels, so the linear response is exact there
        let mut u = fast_tanh(x - k * y4);
        let mut out = [0.0; 4];
        for i in 0..4 {
            let v = gg * (u - s[i]);
            let y = v + s[i];
            s[i] = y + v;
            out[i] = y;
            u = y;
        }
        out
    }

    fn flush(&mut self) {
        for v in self.s.iter_mut() {
            *v = flush(*v);
        }
    }
}

/// Per-voice filter state (stereo).
#[derive(Clone, Copy, Debug, Default)]
pub struct FilterState {
    svf: [[Svf; 2]; 2], // [channel][stage]
    ladder: [Ladder; 2],
    g_prev: f32,
    kind_prev: u8,
    primed: bool,
}

impl FilterState {
    pub fn reset(&mut self) {
        *self = FilterState::default();
    }

    /// Filter `buf` (stereo, from sample `start`) in place.
    pub fn process(&mut self, p: &FilterParams, sr: f32, buf: &mut [[f32; N]; 2], start: usize) {
        if !self.primed || p.kind != self.kind_prev {
            self.reset();
            self.primed = true;
            self.kind_prev = p.kind;
            self.g_prev = g_of(p.cutoff, sr);
        }
        let g1 = g_of(p.cutoff, sr);
        let g0 = self.g_prev;
        self.g_prev = g1;
        let dg = (g1 - g0) / N as f32;
        let mix = p.mix.clamp(0.0, 1.0);
        let drive = if p.drive > 0.0 { 1.0 + 7.0 * p.drive } else { 0.0 };
        let dnorm = if drive > 0.0 { 1.0 / fast_tanh(drive) } else { 1.0 };

        if is_ladder(p.kind) {
            let k = ladder_k(p.res);
            let comp = ladder_comp(k);
            let tap = (p.kind - LADDER6) as usize;
            for i in start..N {
                let g = g0 + dg * (i + 1) as f32;
                let gg = g / (1.0 + g);
                for ch in 0..2 {
                    let dry = buf[ch][i];
                    let x = if drive > 0.0 { fast_tanh(dry * drive) * dnorm } else { dry };
                    let y = self.ladder[ch].tick(x, gg, k)[tap] * comp;
                    buf[ch][i] = dry + (y - dry) * mix;
                }
            }
            for l in self.ladder.iter_mut() {
                l.flush();
            }
        } else {
            let k = svf_k(p.res);
            let four = matches!(p.kind, LOW24 | HIGH24 | BAND24 | NOTCH24);
            for i in start..N {
                let g = g0 + dg * (i + 1) as f32;
                let a1 = 1.0 / (1.0 + g * (g + k));
                let a1b = 1.0 / (1.0 + g * (g + SQRT_2));
                for ch in 0..2 {
                    let dry = buf[ch][i];
                    let x = if drive > 0.0 { fast_tanh(dry * drive) * dnorm } else { dry };
                    let [s1, s2] = &mut self.svf[ch];
                    let y1 = pick(p.kind, s1.tick(x, g, k, a1), x, k);
                    let y = if four { pick(p.kind, s2.tick(y1, g, SQRT_2, a1b), y1, SQRT_2) } else { y1 };
                    buf[ch][i] = dry + (y - dry) * mix;
                }
            }
            for ch in self.svf.iter_mut() {
                for s in ch.iter_mut() {
                    s.flush();
                }
            }
        }
    }
}

/// Choose an SVF output: low, normalized band (unity peak), high or notch.
#[inline(always)]
fn pick(kind: u8, (lp, bp, hp): (f32, f32, f32), x: f32, k: f32) -> f32 {
    match kind {
        LOW12 | LOW24 => lp,
        HIGH12 | HIGH24 => hp,
        BAND12 | BAND24 => k * bp,
        _ => x - k * bp, // notch
    }
}

/// Exact magnitude response (linear) of a filter type at frequency `f`.
pub fn response(kind: u8, cutoff: f32, res: f32, sr: f32, f: f32) -> f32 {
    use num::C;
    // bilinear transform with the cutoff prewarped: s = j·tan(πf/sr)/tan(πfc/sr)
    let w = (PI * f.clamp(0.0, sr * 0.4999) / sr).tan() / g_of(cutoff, sr);
    let s = C::new(0.0, w);
    let one = C::new(1.0, 0.0);
    if is_ladder(kind) {
        let k = ladder_k(res);
        let g = one.div(one.add(s));
        let g4 = g.mul(g).mul(g).mul(g);
        let n = (kind - LADDER6 + 1) as u32;
        let mut gn = one;
        for _ in 0..n {
            gn = gn.mul(g);
        }
        return gn.div(one.add(g4.scale(k))).abs() * ladder_comp(k);
    }
    let svf = |k: f32, kind: u8| -> C {
        let den = s.mul(s).add(s.scale(k)).add(one);
        let num = match kind {
            LOW12 | LOW24 => one,
            HIGH12 | HIGH24 => s.mul(s),
            BAND12 | BAND24 => s.scale(k),
            _ => s.mul(s).add(one),
        };
        num.div(den)
    };
    let h1 = svf(svf_k(res), kind).abs();
    if matches!(kind, LOW24 | HIGH24 | BAND24 | NOTCH24) { h1 * svf(SQRT_2, kind).abs() } else { h1 }
}

/// Just enough complex arithmetic for `response`.
mod num {
    #[derive(Clone, Copy)]
    pub struct C {
        re: f64,
        im: f64,
    }
    impl C {
        pub fn new(re: f32, im: f32) -> C {
            C { re: re as f64, im: im as f64 }
        }
        pub fn add(self, o: C) -> C {
            C { re: self.re + o.re, im: self.im + o.im }
        }
        pub fn mul(self, o: C) -> C {
            C { re: self.re * o.re - self.im * o.im, im: self.re * o.im + self.im * o.re }
        }
        pub fn scale(self, k: f32) -> C {
            C { re: self.re * k as f64, im: self.im * k as f64 }
        }
        pub fn div(self, o: C) -> C {
            let d = o.re * o.re + o.im * o.im;
            C { re: (self.re * o.re + self.im * o.im) / d, im: (self.im * o.re - self.re * o.im) / d }
        }
        pub fn abs(self) -> f32 {
            (self.re * self.re + self.im * self.im).sqrt() as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    /// Measured magnitude at `f` from a steady sine through the filter.
    fn measure(kind: u8, cutoff: f32, res: f32, f: f32) -> f32 {
        let mut st = FilterState::default();
        let p = FilterParams { kind, cutoff, res, drive: 0.0, mix: 1.0 };
        let amp = 1e-3; // stay in the linear region of the ladder's saturation
        let blocks = (SR as usize * 2) / N;
        let (mut peak, mut dot_s, mut dot_c, mut count) = (0.0f64, 0.0f64, 0.0f64, 0usize);
        for b in 0..blocks {
            let mut buf = [[0.0f32; N]; 2];
            for i in 0..N {
                let t = (b * N + i) as f64 / SR as f64;
                let v = (amp as f64 * (std::f64::consts::TAU * f as f64 * t).sin()) as f32;
                buf[0][i] = v;
                buf[1][i] = v;
            }
            st.process(&p, SR, &mut buf, 0);
            if b >= blocks / 2 {
                for i in 0..N {
                    let t = (b * N + i) as f64 / SR as f64;
                    let ph = std::f64::consts::TAU * f as f64 * t;
                    dot_s += buf[0][i] as f64 * ph.sin();
                    dot_c += buf[0][i] as f64 * ph.cos();
                    count += 1;
                    peak = peak.max(buf[0][i].abs() as f64);
                }
            }
        }
        let _ = peak;
        ((2.0 * (dot_s * dot_s + dot_c * dot_c).sqrt() / count as f64) / amp as f64) as f32
    }

    #[test]
    fn measured_response_matches_the_analytic_one() {
        let db = |x: f32| 20.0 * x.max(1e-9).log10();
        for kind in 0..=LAST_TYPE {
            for &(cutoff, res) in &[(200.0f32, 0.0f32), (1000.0, 0.5), (5000.0, 0.9)] {
                for &f in &[60.0f32, 330.0, 1000.0, 2400.0, 7000.0, 15000.0] {
                    let want = response(kind, cutoff, res, SR, f);
                    if db(want) < -60.0 {
                        continue; // below the measurement's noise floor
                    }
                    let got = measure(kind, cutoff, res, f);
                    assert!((db(got) - db(want)).abs() < 0.3, "type {kind} fc {cutoff} res {res} f {f}: {:.2} dB vs {:.2} dB", db(got), db(want));
                }
            }
        }
    }

    #[test]
    fn stays_finite_at_full_resonance_for_a_minute() {
        for kind in 0..=LAST_TYPE {
            let mut st = FilterState::default();
            let mut peak = 0.0f32;
            let blocks = SR as usize * 60 / N;
            for b in 0..blocks {
                // a loud saw, with the cutoff swept up and down each second
                let t = b as f32 * N as f32 / SR;
                let cutoff = 60.0 * (1.0 + 300.0 * (0.5 + 0.5 * (t * 6.28).sin()));
                let p = FilterParams { kind, cutoff, res: 1.0, drive: 0.5, mix: 1.0 };
                let mut buf = [[0.0f32; N]; 2];
                for i in 0..N {
                    let ph = ((b * N + i) as f32 * 110.0 / SR).fract();
                    buf[0][i] = 2.0 * ph - 1.0;
                    buf[1][i] = buf[0][i];
                }
                st.process(&p, SR, &mut buf, 0);
                for v in buf[0] {
                    assert!(v.is_finite(), "type {kind} went non-finite");
                    peak = peak.max(v.abs());
                }
            }
            assert!(peak < 100.0, "type {kind} peaked at {peak}");
        }
    }
}
