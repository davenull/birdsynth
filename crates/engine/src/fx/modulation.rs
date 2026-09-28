//! Time-modulation effects: flanger, phaser, chorus and Hyper/Dimension.

use wt_dsp::math::{flush, sin_turns};

use super::Ctx;
use super::util::{Block, DelayLine, Lp1, ModLfo, N, lfo_rate, onepole_coef};
use crate::spec::params as p;

/// 0..1 triangle for a phase in cycles.
#[inline(always)]
fn tri(ph: f64) -> f32 {
    let f = (ph - ph.floor()) as f32;
    if f < 0.5 { 2.0 * f } else { 2.0 - 2.0 * f }
}

// ------------------------------------------------------------------ flanger

pub struct Flanger {
    line: [DelayLine; 2],
    lfo: ModLfo,
}

impl Flanger {
    pub fn new(sr: f32) -> Flanger {
        let len = (sr * 0.03) as usize + 8;
        Flanger { line: [DelayLine::new(len), DelayLine::new(len)], lfo: ModLfo::default() }
    }

    pub fn reset(&mut self) {
        self.line.iter_mut().for_each(DelayLine::clear);
        self.lfo.reset();
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let (hz, sync) = lfo_rate(cx.p(p::FX_FLANGER_RATE[i]), cx.p(p::FX_FLANGER_BPM[i]) >= 0.5, cx.p(p::FX_FLANGER_SYNC[i]), cx.bpm);
        let (p0, p1) = self.lfo.advance(hz, sync, cx.beat, N as f64 / cx.sr as f64);
        let depth = cx.p(p::FX_FLANGER_DEPTH[i]);
        let base = cx.p(p::FX_FLANGER_DELAY[i]);
        let fb = cx.p(p::FX_FLANGER_FEEDBACK[i]).clamp(-0.97, 0.97);
        let stereo = cx.p(p::FX_FLANGER_PHASE[i]) as f64 / 360.0;
        let mix = cx.p(p::FX_FLANGER_MIX[i]);
        let ms = cx.sr / 1000.0;
        for n in 0..N {
            let ph = p0 + (p1 - p0) * (n + 1) as f64 / N as f64;
            for ch in 0..2 {
                let d = (base + depth * 5.0 * tri(ph + ch as f64 * stereo)) * ms;
                let x = buf[ch][n];
                let y = self.line[ch].read_cubic(d.max(1.0));
                self.line[ch].write(x + fb * y);
                let wet = 0.5 * (x + y);
                buf[ch][n] = x + (wet - x) * mix;
            }
        }
    }
}

// ------------------------------------------------------------------- phaser

pub struct Phaser {
    ap: [[f32; 12]; 2],
    fbv: [f32; 2],
    gg: [f32; 2],
    lfo: ModLfo,
    primed: bool,
}

const STAGES: [usize; 5] = [2, 4, 6, 8, 12];

impl Default for Phaser {
    fn default() -> Self {
        Phaser::new()
    }
}

impl Phaser {
    pub fn new() -> Phaser {
        Phaser { ap: [[0.0; 12]; 2], fbv: [0.0; 2], gg: [0.0; 2], lfo: ModLfo::default(), primed: false }
    }

    pub fn reset(&mut self) {
        *self = Phaser::new();
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let (hz, sync) = lfo_rate(cx.p(p::FX_PHASER_RATE[i]), cx.p(p::FX_PHASER_BPM[i]) >= 0.5, cx.p(p::FX_PHASER_SYNC[i]), cx.bpm);
        let (_, p1) = self.lfo.advance(hz, sync, cx.beat, N as f64 / cx.sr as f64);
        let depth = cx.p(p::FX_PHASER_DEPTH[i]);
        let freq = cx.p(p::FX_PHASER_FREQ[i]);
        let fb = cx.p(p::FX_PHASER_FEEDBACK[i]).clamp(-0.95, 0.95);
        let stages = STAGES[(cx.p(p::FX_PHASER_STAGES[i]) as usize).min(4)];
        let spread = cx.p(p::FX_PHASER_SPREAD[i]) as f64 / 360.0;
        let mix = cx.p(p::FX_PHASER_MIX[i]);
        for ch in 0..2 {
            // sweep ±2 octaves at full depth; the coefficient ramps across the block
            let f = freq * (depth * 2.0 * sin_turns((p1 + ch as f64 * spread).fract() as f32)).exp2();
            let g = (std::f32::consts::PI * f.clamp(10.0, cx.sr * 0.45) / cx.sr).tan();
            let g1 = g / (1.0 + g);
            let g0 = if self.primed { self.gg[ch] } else { g1 };
            self.gg[ch] = g1;
            for n in 0..N {
                let gg = g0 + (g1 - g0) * (n + 1) as f32 / N as f32;
                let x = buf[ch][n];
                let mut u = x + fb * self.fbv[ch];
                for st in self.ap[ch].iter_mut().take(stages) {
                    let w = gg * (u - *st);
                    let lp = w + *st;
                    *st = lp + w;
                    u = 2.0 * lp - u;
                }
                self.fbv[ch] = u;
                let wet = 0.5 * (x + u);
                buf[ch][n] = x + (wet - x) * mix;
            }
            for st in self.ap[ch].iter_mut() {
                *st = flush(*st);
            }
            self.fbv[ch] = flush(self.fbv[ch]);
        }
        self.primed = true;
    }
}

// ------------------------------------------------------------------- chorus

pub struct Chorus {
    line: [DelayLine; 2],
    lp: [Lp1; 2],
    fbv: [f32; 2],
    lfo: ModLfo,
}

impl Chorus {
    pub fn new(sr: f32) -> Chorus {
        let len = (sr * 0.06) as usize + 8;
        Chorus { line: [DelayLine::new(len), DelayLine::new(len)], lp: [Lp1::default(); 2], fbv: [0.0; 2], lfo: ModLfo::default() }
    }

    pub fn reset(&mut self) {
        self.line.iter_mut().for_each(DelayLine::clear);
        self.lp = [Lp1::default(); 2];
        self.fbv = [0.0; 2];
        self.lfo.reset();
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let (hz, sync) = lfo_rate(cx.p(p::FX_CHORUS_RATE[i]), cx.p(p::FX_CHORUS_BPM[i]) >= 0.5, cx.p(p::FX_CHORUS_SYNC[i]), cx.bpm);
        let (p0, p1) = self.lfo.advance(hz, sync, cx.beat, N as f64 / cx.sr as f64);
        let d1 = cx.p(p::FX_CHORUS_DELAY1[i]);
        let d2 = cx.p(p::FX_CHORUS_DELAY2[i]);
        let depth = cx.p(p::FX_CHORUS_DEPTH[i]) * 5.0;
        let fb = cx.p(p::FX_CHORUS_FEEDBACK[i]);
        let a = onepole_coef(cx.p(p::FX_CHORUS_LPF[i]), cx.sr);
        let mix = cx.p(p::FX_CHORUS_MIX[i]);
        let ms = cx.sr / 1000.0;
        for n in 0..N {
            let ph = p0 + (p1 - p0) * (n + 1) as f64 / N as f64;
            for ch in 0..2 {
                let q = (ph + ch as f64 * 0.25).fract() as f32;
                let m1 = 0.5 + 0.5 * sin_turns(q);
                let m2 = 0.5 + 0.5 * sin_turns((q + 0.5).fract());
                let t1 = self.line[ch].read_cubic((d1 + depth * m1) * ms);
                let t2 = self.line[ch].read_cubic((d2 + depth * m2) * ms);
                let w = self.lp[ch].tick(0.5 * (t1 + t2), a);
                let x = buf[ch][n];
                self.line[ch].write(x + fb * w);
                buf[ch][n] = x + (w - x) * mix;
            }
        }
        for l in self.lp.iter_mut() {
            l.flush();
        }
    }
}

// ---------------------------------------------------------- hyper/dimension

const HYPER_VOICES: usize = 8;

pub struct Hyper {
    line: [DelayLine; 2],
    dim: [DelayLine; 2],
    phase: [f64; HYPER_VOICES],
}

impl Hyper {
    pub fn new(sr: f32) -> Hyper {
        let len = (sr * 0.05) as usize + 8;
        let mut phase = [0.0; HYPER_VOICES];
        for (k, v) in phase.iter_mut().enumerate() {
            *v = k as f64 * 0.37;
        }
        Hyper { line: [DelayLine::new(len), DelayLine::new(len)], dim: [DelayLine::new(len), DelayLine::new(len)], phase }
    }

    pub fn reset(&mut self) {
        self.line.iter_mut().chain(self.dim.iter_mut()).for_each(DelayLine::clear);
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let rate = cx.p(p::FX_HYPER_RATE[i]) as f64;
        let detune = cx.p(p::FX_HYPER_DETUNE[i]);
        let voices = (cx.p(p::FX_HYPER_VOICES[i]) as usize).clamp(2, HYPER_VOICES);
        let hmix = cx.p(p::FX_HYPER_HYPER_MIX[i]);
        let size = cx.p(p::FX_HYPER_SIZE[i]);
        let dmix = cx.p(p::FX_HYPER_DIM_MIX[i]);
        let ms = cx.sr / 1000.0;
        let dt = N as f64 / cx.sr as f64;
        let p0 = self.phase;
        for (k, ph) in self.phase.iter_mut().enumerate() {
            *ph += rate * (1.0 + 0.13 * k as f64) * dt;
            if *ph > 1e6 {
                *ph -= ph.floor();
            }
        }
        // pan each copy across the field, and normalize the sum
        let norm = 1.0 / (voices as f32).sqrt();
        let dim_d = [(5.0 + 20.0 * size) * ms, (7.0 + 23.0 * size) * ms];
        for n in 0..N {
            let t = (n + 1) as f64 / N as f64;
            let (x0, x1) = (buf[0][n], buf[1][n]);
            self.line[0].write(x0);
            self.line[1].write(x1);
            let (mut hl, mut hr) = (0.0f32, 0.0f32);
            for k in 0..voices {
                let ph = p0[k] + (self.phase[k] - p0[k]) * t;
                let d = (7.0 + detune * 3.0 * sin_turns((ph - ph.floor()) as f32)) * ms;
                let pan = k as f32 / (voices - 1) as f32; // 0 left .. 1 right
                hl += self.line[0].read_cubic(d) * (1.0 - pan);
                hr += self.line[1].read_cubic(d) * pan;
            }
            let (mut y0, mut y1) = (x0 + hl * norm * hmix, x1 + hr * norm * hmix);
            // Dimension: each side hears the other, a little later
            self.dim[0].write(y0);
            self.dim[1].write(y1);
            let c0 = self.dim[1].read(dim_d[0]);
            let c1 = self.dim[0].read(dim_d[1]);
            y0 += dmix * 0.6 * (c0 - 0.5 * c1);
            y1 += dmix * 0.6 * (c1 - 0.5 * c0);
            buf[0][n] = y0;
            buf[1][n] = y1;
        }
    }
}
