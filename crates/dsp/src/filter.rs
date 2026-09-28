//! Filters: the voice filters (and the FX filter) on a handful of shared
//! cores. The linear cores are zero-delay-feedback (TPT) structures, exact
//! bilinear transforms of their analog prototypes with the cutoff prewarped,
//! so `response` gives their true digital magnitude response; the nonlinear
//! ones (saturating ladders, Sallen-Key, dirty combs) match it at small
//! signal levels.
//!
//! Type numbers are the `filter.*.type` options in params/filter.toml, in the
//! order of `NAMES` (a test keeps the two in step). New types only ever go at
//! the end.

use std::f32::consts::{PI, SQRT_2};

use crate::math::{fast_tanh, flush, sin_turns};

/// Longest block `tick` is prepared for: a 16-sample sub-block at 4×.
pub const MAX_BLOCK: usize = 64;
/// Delay memory per channel (combs, diffusor): 11.7 Hz lowest comb at 48 kHz.
pub const DELAY_LEN: usize = 4096;

pub const NAMES: [&str; COUNT] = [
    "Low 12",
    "Low 24",
    "High 12",
    "High 24",
    "Band 12",
    "Band 24",
    "Notch 12",
    "Notch 24",
    "Ladder Low 6",
    "Ladder Low 12",
    "Ladder Low 18",
    "Ladder Low 24",
    "Low 6",
    "High 6",
    "Low 18",
    "High 18",
    "Low 36",
    "High 36",
    "Peak 12",
    "Peak 24",
    "Allpass 12",
    "Allpass 24",
    "Low Shelf",
    "High Shelf",
    "Bell",
    "Tilt",
    "Low-Band-High 12",
    "Low-Band-High 24",
    "Low-High 12",
    "Band-Notch 12",
    "Ladder High 24",
    "Ladder Band 12",
    "Dirty Ladder 24",
    "Smooth Ladder 24",
    "Acid 18",
    "Acid 24",
    "Diode 12",
    "Diode 24",
    "SK Low 12",
    "SK High 12",
    "SK Band 12",
    "Scream Low 12",
    "Scream High 12",
    "Comb +",
    "Comb -",
    "Flange +",
    "Flange -",
    "Dist Comb +",
    "Dist Comb -",
    "Allpass Comb",
    "Phaser 4 +",
    "Phaser 4 -",
    "Phaser 8 +",
    "Phaser 8 -",
    "Phaser 12 +",
    "Phaser 12 -",
    "Formant",
    "Formant Deep",
    "Formant Bright",
    "Ring Mod",
    "Sample & Hold",
    "Diffusor",
    "Band Reject",
];
pub const COUNT: usize = 63;

pub const LOW12: u8 = 0;
pub const LOW24: u8 = 1;
pub const HIGH12: u8 = 2;
pub const HIGH24: u8 = 3;
pub const BAND12: u8 = 4;
pub const BAND24: u8 = 5;
pub const NOTCH12: u8 = 6;
pub const NOTCH24: u8 = 7;
pub const LADDER6: u8 = 8;
pub const LADDER24: u8 = 11;
pub const LAST_TYPE: u8 = (COUNT - 1) as u8;

/// Section each type is listed under in the UI.
pub fn group(kind: u8) -> &'static str {
    match kind {
        0..=7 | 12..=21 => "Normal",
        22..=25 => "EQ",
        26..=29 => "Multi",
        8..=11 | 30..=37 => "Ladders",
        38..=42 => "Sallen-Key",
        43..=55 => "Flanges",
        56..=58 => "Formant",
        _ => "Misc",
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Out {
    Low,
    High,
    Band,
    Notch,
    Peak,
    Allpass,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Eq {
    LowShelf,
    HighShelf,
    Bell,
    Tilt,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Morph {
    LowBandHigh,
    LowHigh,
    BandNotch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Flavor {
    Clean,
    Dirty,
    Smooth,
    Acid,
    Diode,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Tap {
    Low(usize),
    High,
    Band,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Comb {
    Feedback(bool),
    Forward(bool),
    Dist(bool),
    Allpass,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Core {
    Svf(Out, u8),
    OnePole { high: bool, twelve: bool },
    Eq(Eq),
    Morph(Morph, u8),
    Ladder(Flavor, Tap),
    Sk(Out, bool),
    Comb(Comb),
    Phaser(u8, bool),
    Formant(u8),
    Ring,
    Hold,
    Diffusor,
    Reject,
}

fn core(kind: u8) -> Core {
    use Core::*;
    match kind {
        0 => Svf(Out::Low, 1),
        1 => Svf(Out::Low, 2),
        2 => Svf(Out::High, 1),
        3 => Svf(Out::High, 2),
        4 => Svf(Out::Band, 1),
        5 => Svf(Out::Band, 2),
        6 => Svf(Out::Notch, 1),
        7 => Svf(Out::Notch, 2),
        8..=11 => Ladder(Flavor::Clean, Tap::Low((kind - 7) as usize)),
        12 => OnePole { high: false, twelve: false },
        13 => OnePole { high: true, twelve: false },
        14 => OnePole { high: false, twelve: true },
        15 => OnePole { high: true, twelve: true },
        16 => Svf(Out::Low, 3),
        17 => Svf(Out::High, 3),
        18 => Svf(Out::Peak, 1),
        19 => Svf(Out::Peak, 2),
        20 => Svf(Out::Allpass, 1),
        21 => Svf(Out::Allpass, 2),
        22 => Eq(self::Eq::LowShelf),
        23 => Eq(self::Eq::HighShelf),
        24 => Eq(self::Eq::Bell),
        25 => Eq(self::Eq::Tilt),
        26 => Morph(self::Morph::LowBandHigh, 1),
        27 => Morph(self::Morph::LowBandHigh, 2),
        28 => Morph(self::Morph::LowHigh, 1),
        29 => Morph(self::Morph::BandNotch, 1),
        30 => Ladder(Flavor::Clean, Tap::High),
        31 => Ladder(Flavor::Clean, Tap::Band),
        32 => Ladder(Flavor::Dirty, Tap::Low(4)),
        33 => Ladder(Flavor::Smooth, Tap::Low(4)),
        34 => Ladder(Flavor::Acid, Tap::Low(3)),
        35 => Ladder(Flavor::Acid, Tap::Low(4)),
        36 => Ladder(Flavor::Diode, Tap::Low(2)),
        37 => Ladder(Flavor::Diode, Tap::Low(4)),
        38 => Sk(Out::Low, false),
        39 => Sk(Out::High, false),
        40 => Sk(Out::Band, false),
        41 => Sk(Out::Low, true),
        42 => Sk(Out::High, true),
        43 => Comb(self::Comb::Feedback(false)),
        44 => Comb(self::Comb::Feedback(true)),
        45 => Comb(self::Comb::Forward(false)),
        46 => Comb(self::Comb::Forward(true)),
        47 => Comb(self::Comb::Dist(false)),
        48 => Comb(self::Comb::Dist(true)),
        49 => Comb(self::Comb::Allpass),
        50 => Phaser(4, false),
        51 => Phaser(4, true),
        52 => Phaser(8, false),
        53 => Phaser(8, true),
        54 => Phaser(12, false),
        55 => Phaser(12, true),
        56 => Formant(0),
        57 => Formant(1),
        58 => Formant(2),
        59 => Ring,
        60 => Hold,
        61 => Diffusor,
        _ => Reject,
    }
}

// ---------------------------------------------------------------- tuning

/// SVF damping: Butterworth (k = √2) at no resonance, nearly self-oscillating at full.
#[inline]
pub fn svf_k(res: f32) -> f32 {
    SQRT_2 * (1.0 - res) + 0.02 * res
}

/// Ladder feedback for a flavor: 0 to just under self-oscillation.
#[inline]
fn ladder_k(flavor: Flavor, res: f32) -> f32 {
    match flavor {
        Flavor::Smooth => 3.6 * res,
        Flavor::Diode => 4.4 * res,
        _ => 3.98 * res,
    }
}

/// Ladder low-pass gain: part of the bass the resonance takes is given back.
#[inline]
fn ladder_comp(flavor: Flavor, k: f32) -> f32 {
    match flavor {
        Flavor::Smooth => 1.0 + 0.4 * k,
        _ => 1.0 + 0.5 * k,
    }
}

/// Per-stage cutoff ratios of a ladder flavor (diode-like ladders stagger them).
#[inline]
fn ladder_ratios(flavor: Flavor) -> [f32; 4] {
    match flavor {
        Flavor::Acid => [0.8, 1.0, 1.0, 1.25],
        Flavor::Diode => [0.5, 1.0, 1.0, 1.0],
        _ => [1.0; 4],
    }
}

/// Prewarped integrator gain for a cutoff.
#[inline]
pub fn g_of(cutoff: f32, sr: f32) -> f32 {
    (PI * cutoff.clamp(5.0, sr * 0.49) / sr).tan()
}

/// EQ gain from the resonance control: -18 dB at 0, flat at 0.5, +18 dB at 1.
#[inline]
pub fn eq_gain_db(res: f32) -> f32 {
    (res - 0.5) * 36.0
}

/// EQ Q from the var control.
#[inline]
fn eq_q(eq: Eq, var: f32) -> f32 {
    match eq {
        Eq::Bell => 0.3 + 5.7 * var * var,
        _ => 0.4 + 0.8 * var,
    }
}

/// Formant sets: (F1, F2, F3, F4) in Hz for A, E, I, O, U, and band gains.
const FORMANTS: [[[f32; 4]; 5]; 3] = [
    [[800.0, 1150.0, 2900.0, 0.0], [350.0, 2000.0, 2800.0, 0.0], [270.0, 2140.0, 2950.0, 0.0], [450.0, 800.0, 2830.0, 0.0], [325.0, 700.0, 2530.0, 0.0]],
    [[600.0, 1040.0, 2250.0, 0.0], [400.0, 1620.0, 2400.0, 0.0], [250.0, 1750.0, 2600.0, 0.0], [400.0, 750.0, 2400.0, 0.0], [350.0, 600.0, 2400.0, 0.0]],
    [[800.0, 1150.0, 2900.0, 3900.0], [350.0, 2000.0, 2800.0, 3600.0], [270.0, 2140.0, 2950.0, 3900.0], [450.0, 800.0, 2830.0, 3800.0], [325.0, 700.0, 2700.0, 3800.0]],
];
const FORMANT_GAIN: [f32; 4] = [1.0, 0.6, 0.3, 0.2];

/// Formant frequencies for a set, morphing through the vowels with `var`
/// and shifted by `shift` (the cutoff over 1 kHz).
fn formant_freqs(set: u8, var: f32, shift: f32) -> ([f32; 4], usize) {
    let v = &FORMANTS[set as usize];
    let x = var.clamp(0.0, 1.0) * 4.0;
    let i = (x as usize).min(3);
    let t = x - i as f32;
    let mut f = [0.0; 4];
    for b in 0..4 {
        f[b] = (v[i][b] + (v[i + 1][b] - v[i][b]) * t) * shift;
    }
    (f, if set == 2 { 4 } else { 3 })
}

/// Formant band Q from resonance.
#[inline]
fn formant_k(res: f32) -> f32 {
    1.0 / (4.0 + 20.0 * res)
}

/// Phaser stage frequency ratios: spread over `var` × 2 octaves around the cutoff.
fn phaser_ratio(stage: usize, stages: u8, var: f32) -> f32 {
    let n = stages as f32;
    let x = if stages > 1 { stage as f32 / (n - 1.0) - 0.5 } else { 0.0 };
    (x * 2.0 * var).exp2()
}

/// Band Reject: the gap between the low-pass and high-pass edges, in octaves each side.
#[inline]
fn reject_width(var: f32) -> f32 {
    0.1 + 1.9 * var
}

#[derive(Clone, Copy, Debug)]
pub struct FilterParams {
    pub kind: u8,
    pub cutoff: f32,
    pub res: f32,
    pub drive: f32,
    pub mix: f32,
    /// The type's extra control: morph position, vowel, EQ Q, spread, colour.
    pub var: f32,
    /// Cutoff offset between the channels, in octaves (-1..1).
    pub stereo: f32,
}

impl FilterParams {
    pub fn new(kind: u8, cutoff: f32, res: f32) -> FilterParams {
        FilterParams { kind, cutoff, res, drive: 0.0, mix: 1.0, var: 0.5, stereo: 0.0 }
    }
}

// ----------------------------------------------------------------- cores

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
struct OnePole {
    s: f32,
}

impl OnePole {
    /// One sample; `gg` is g/(1+g). Returns the low-pass output.
    #[inline(always)]
    fn tick(&mut self, x: f32, gg: f32) -> f32 {
        let v = gg * (x - self.s);
        let y = v + self.s;
        self.s = y + v;
        y
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Ladder {
    s: [f32; 4],
}

impl Ladder {
    /// One sample through four TPT one-poles (per-stage gains `gg`) with
    /// zero-delay feedback k. Returns the first stage's input and the four
    /// stage outputs.
    #[inline(always)]
    fn tick(&mut self, x: f32, gg: [f32; 4], k: f32, flavor: Flavor) -> (f32, [f32; 4]) {
        let s = &mut self.s;
        let b = [1.0 - gg[0], 1.0 - gg[1], 1.0 - gg[2], 1.0 - gg[3]];
        // y4 = Gp·u + S with u = x - k·y4, solved for y4
        let gp = gg[0] * gg[1] * gg[2] * gg[3];
        let st = gg[1] * gg[2] * gg[3] * b[0] * s[0] + gg[2] * gg[3] * b[1] * s[1] + gg[3] * b[2] * s[2] + b[3] * s[3];
        let y4 = (gp * x + st) / (1.0 + k * gp);
        // the saturation sits where the feedback meets the input; it is
        // transparent at small levels, so the linear response is exact there
        let u = match flavor {
            Flavor::Smooth => x - k * y4,
            Flavor::Dirty => 0.5 * fast_tanh(2.0 * (x - k * y4)),
            _ => fast_tanh(x - k * y4),
        };
        let mut v_in = u;
        let mut out = [0.0; 4];
        for i in 0..4 {
            let v = gg[i] * (v_in - s[i]);
            let y = v + s[i];
            s[i] = y + v;
            out[i] = y;
            v_in = y;
        }
        (u, out)
    }

    fn flush(&mut self) {
        for v in self.s.iter_mut() {
            *v = flush(*v);
        }
    }
}

/// Per-sub-block coefficients (from `FilterState::prepare`).
#[derive(Clone, Copy, Debug)]
pub struct Coeffs {
    core: Core,
    g0: [f32; 2],
    dg: [f32; 2],
    /// Second cutoff per channel (Band Reject's high-pass edge).
    h0: [f32; 2],
    dh: [f32; 2],
    k: f32,
    comp: f32,
    drive: f32,
    dnorm: f32,
    mix: f32,
    var: f32,
    res: f32,
    ratios: [f32; 4],
    /// Both channels share a cutoff (no stereo offset): coefficients are computed once.
    mono: bool,
    /// The ladder's stages all share a cutoff.
    uniform: bool,
    // EQ
    eq_a: f32,
    eq_k: f32,
    // combs (delay in samples, ramped)
    d0: [f32; 2],
    dd: [f32; 2],
    fb: f32,
    damp: f32,
    // phaser stage gains per channel, block-constant
    pg: [[f32; 12]; 2],
    pfb: f32,
    // formant band gains per channel, block-constant
    fg: [[f32; 4]; 2],
    bands: usize,
    // ring/hold increments per channel
    inc: [f32; 2],
}

/// Per-voice filter state (stereo).
#[derive(Clone, Copy, Debug)]
pub struct FilterState {
    svf: [[Svf; 3]; 2],
    one: [OnePole; 2],
    ladder: [Ladder; 2],
    ap: [[f32; 12]; 2],
    fbv: [f32; 2],
    form: [[Svf; 4]; 2],
    phase: [f32; 2],
    hold: [f32; 2],
    smooth: [f32; 2],
    damp: [f32; 2],
    delay: [[f32; DELAY_LEN]; 2],
    wpos: usize,
    /// The delay memory may hold old audio (cleared before a delay type uses it).
    dirty: bool,
    /// Nothing to clear (reset since the last block).
    clean: bool,
    g_prev: [f32; 2],
    h_prev: [f32; 2],
    d_prev: [f32; 2],
    kind_prev: u8,
    sr_prev: f32,
    primed: bool,
}

impl Default for FilterState {
    fn default() -> Self {
        FilterState {
            svf: [[Svf::default(); 3]; 2],
            one: [OnePole::default(); 2],
            ladder: [Ladder::default(); 2],
            ap: [[0.0; 12]; 2],
            fbv: [0.0; 2],
            form: [[Svf::default(); 4]; 2],
            phase: [0.0; 2],
            hold: [0.0; 2],
            smooth: [0.0; 2],
            damp: [0.0; 2],
            delay: [[0.0; DELAY_LEN]; 2],
            wpos: 0,
            dirty: false,
            clean: true,
            g_prev: [0.0; 2],
            h_prev: [0.0; 2],
            d_prev: [0.0; 2],
            kind_prev: 0,
            sr_prev: 0.0,
            primed: false,
        }
    }
}

#[inline]
fn uses_delay(c: Core) -> bool {
    matches!(c, Core::Comb(_) | Core::Diffusor)
}

/// Comb delay in samples for a cutoff (the comb's fundamental).
#[inline]
fn comb_delay(cutoff: f32, sr: f32) -> f32 {
    (sr / cutoff.max(1.0)).clamp(2.0, (DELAY_LEN - 3) as f32)
}

/// Diffusor base delay (samples); each of the four stages takes a quarter of the memory.
#[inline]
fn diffuse_delay(cutoff: f32, sr: f32) -> f32 {
    (sr / cutoff.max(1.0)).clamp(4.0, (DELAY_LEN / 4 - 3) as f32)
}

const DIFFUSE: [f32; 4] = [1.0, 0.7, 0.53, 0.37];

impl FilterState {
    /// Forget the signal (cheap: delay memory is cleared only when a delay type next runs).
    pub fn reset(&mut self) {
        if self.clean {
            return;
        }
        let delay_dirty = self.dirty || self.wpos != 0 || uses_delay(core(self.kind_prev));
        self.svf = [[Svf::default(); 3]; 2];
        self.one = [OnePole::default(); 2];
        self.ladder = [Ladder::default(); 2];
        self.ap = [[0.0; 12]; 2];
        self.fbv = [0.0; 2];
        self.form = [[Svf::default(); 4]; 2];
        self.phase = [0.0; 2];
        self.hold = [0.0; 2];
        self.smooth = [0.0; 2];
        self.damp = [0.0; 2];
        self.wpos = 0;
        self.dirty = delay_dirty;
        self.primed = false;
        self.clean = true;
    }

    /// Coefficients for the next `len` samples at sample rate `sr`.
    pub fn prepare(&mut self, p: &FilterParams, sr: f32, len: usize) -> Coeffs {
        let kind = p.kind.min(LAST_TYPE);
        let c = core(kind);
        let r = p.stereo.clamp(-1.0, 1.0).exp2().sqrt(); // half the offset each side
        let fc = [p.cutoff / r, p.cutoff * r];
        let reject = reject_width(p.var).exp2();
        let g1 = [g_of(fc[0], sr), g_of(fc[1], sr)];
        let (lo, hi) = if c == Core::Reject { ([fc[0] / reject, fc[1] / reject], [fc[0] * reject, fc[1] * reject]) } else { (fc, fc) };
        let g1 = if c == Core::Reject { [g_of(lo[0], sr), g_of(lo[1], sr)] } else { g1 };
        let h1 = [g_of(hi[0], sr), g_of(hi[1], sr)];
        let d1 = if c == Core::Diffusor { [diffuse_delay(fc[0], sr), diffuse_delay(fc[1], sr)] } else { [comb_delay(fc[0], sr), comb_delay(fc[1], sr)] };
        if !self.primed || kind != self.kind_prev || sr != self.sr_prev {
            self.reset();
            if uses_delay(c) && self.dirty {
                self.delay = [[0.0; DELAY_LEN]; 2];
                self.dirty = false;
            }
            self.primed = true;
            self.clean = false;
            self.kind_prev = kind;
            self.sr_prev = sr;
            self.g_prev = g1;
            self.h_prev = h1;
            self.d_prev = d1;
        }
        let (g0, h0, d0) = (self.g_prev, self.h_prev, self.d_prev);
        self.g_prev = g1;
        self.h_prev = h1;
        self.d_prev = d1;
        let n = len as f32;
        let drive = if p.drive > 0.0 { 1.0 + 7.0 * p.drive } else { 0.0 };
        let res = p.res.clamp(0.0, 1.0);
        let var = p.var.clamp(0.0, 1.0);
        let (k, comp, ratios) = match c {
            Core::Ladder(f, tap) => {
                let k = ladder_k(f, res);
                let comp = if matches!(tap, Tap::Low(_)) { ladder_comp(f, k) } else { 1.0 };
                (k, comp, ladder_ratios(f))
            }
            Core::Sk(_, scream) => (svf_k(res) * if scream { 0.5 } else { 1.0 }, 1.0, [1.0; 4]),
            _ => (svf_k(res), 1.0, [1.0; 4]),
        };
        let mut co = Coeffs {
            core: c,
            g0,
            dg: [(g1[0] - g0[0]) / n, (g1[1] - g0[1]) / n],
            h0,
            dh: [(h1[0] - h0[0]) / n, (h1[1] - h0[1]) / n],
            k,
            comp,
            drive,
            dnorm: if drive > 0.0 { 1.0 / fast_tanh(drive) } else { 1.0 },
            mix: p.mix.clamp(0.0, 1.0),
            var,
            res,
            ratios,
            mono: p.stereo == 0.0,
            uniform: ratios == [1.0; 4],
            eq_a: 1.0,
            eq_k: SQRT_2,
            d0,
            dd: [(d1[0] - d0[0]) / n, (d1[1] - d0[1]) / n],
            fb: 0.0,
            damp: 0.0,
            pg: [[0.0; 12]; 2],
            pfb: 0.0,
            fg: [[0.0; 4]; 2],
            bands: 0,
            inc: [fc[0] / sr, fc[1] / sr],
        };
        match c {
            Core::Eq(e) => {
                co.eq_a = 10f32.powf(eq_gain_db(res) / 40.0);
                co.eq_k = 1.0 / eq_q(e, var);
            }
            Core::Comb(kind) => {
                let sign = match kind {
                    Comb::Feedback(neg) | Comb::Forward(neg) | Comb::Dist(neg) => {
                        if neg {
                            -1.0
                        } else {
                            1.0
                        }
                    }
                    Comb::Allpass => 1.0,
                };
                co.fb = sign
                    * match kind {
                        Comb::Forward(_) => 0.5 + 0.5 * res,
                        Comb::Allpass => 0.3 + 0.6 * res,
                        _ => 0.98 * res,
                    };
                // loop damping: var 0 is dark, 1 bright
                co.damp = 0.15 + 0.85 * var;
            }
            Core::Diffusor => co.fb = 0.3 + 0.6 * res,
            Core::Phaser(stages, neg) => {
                for ch in 0..2 {
                    for s in 0..stages as usize {
                        let g = g_of(fc[ch] * phaser_ratio(s, stages, var), sr);
                        co.pg[ch][s] = g / (1.0 + g);
                    }
                }
                co.pfb = if neg { -0.9 * res } else { 0.9 * res };
            }
            Core::Formant(set) => {
                for ch in 0..2 {
                    let (f, bands) = formant_freqs(set, var, fc[ch] / 1000.0);
                    co.bands = bands;
                    for b in 0..bands {
                        co.fg[ch][b] = g_of(f[b], sr);
                    }
                }
                co.k = formant_k(res);
            }
            _ => {}
        }
        co
    }

    #[inline(always)]
    fn read(&self, ch: usize, base: usize, size: usize, d: f32) -> f32 {
        let pos = self.wpos as f32 - d;
        let pos = if pos < 0.0 { pos + size as f32 } else { pos };
        let i0 = pos as usize;
        let t = pos - i0 as f32;
        let a = self.delay[ch][base + i0 % size];
        let b = self.delay[ch][base + (i0 + 1) % size];
        a + (b - a) * t
    }

    /// One stereo sample (index `i` within the prepared block).
    #[inline(always)]
    pub fn tick(&mut self, c: &Coeffs, i: usize, x: [f32; 2]) -> [f32; 2] {
        let t = (i + 1) as f32;
        let g0 = c.g0[0] + c.dg[0] * t;
        let g = [g0, if c.mono { g0 } else { c.g0[1] + c.dg[1] * t }];
        let v = if c.drive > 0.0 { [fast_tanh(x[0] * c.drive) * c.dnorm, fast_tanh(x[1] * c.drive) * c.dnorm] } else { x };
        // per-sample coefficients, once for both channels when they share a cutoff
        let both = |f: &dyn Fn(f32) -> f32| {
            let a = f(g[0]);
            [a, if c.mono { a } else { f(g[1]) }]
        };
        let y: [f32; 2] = match c.core {
            Core::Svf(o, stages) => {
                let a1 = both(&|g| 1.0 / (1.0 + g * (g + c.k)));
                let a1b = if stages > 1 { both(&|g| 1.0 / (1.0 + g * (g + SQRT_2))) } else { [0.0; 2] };
                let mut y = [0.0; 2];
                for ch in 0..2 {
                    let st = &mut self.svf[ch];
                    let mut u = pick(o, st[0].tick(v[ch], g[ch], c.k, a1[ch]), v[ch], c.k);
                    for s in st.iter_mut().take(stages as usize).skip(1) {
                        u = pick(o, s.tick(u, g[ch], SQRT_2, a1b[ch]), u, SQRT_2);
                    }
                    y[ch] = u;
                }
                y
            }
            Core::Ladder(f, tap) => {
                let gg_of = |g: f32| -> [f32; 4] {
                    if c.uniform {
                        let v = g / (1.0 + g);
                        [v; 4]
                    } else {
                        std::array::from_fn(|s| {
                            let gs = g * c.ratios[s];
                            gs / (1.0 + gs)
                        })
                    }
                };
                let gg0 = gg_of(g[0]);
                let gg = [gg0, if c.mono { gg0 } else { gg_of(g[1]) }];
                let mut y = [0.0; 2];
                for ch in 0..2 {
                    let (u, s) = self.ladder[ch].tick(v[ch], gg[ch], c.k, f);
                    let o = match tap {
                        Tap::Low(n) => s[n - 1],
                        Tap::High => u - 4.0 * s[0] + 6.0 * s[1] - 4.0 * s[2] + s[3],
                        Tap::Band => 4.0 * (s[1] - 2.0 * s[2] + s[3]),
                    };
                    y[ch] = o * c.comp;
                }
                y
            }
            _ => {
                let mut y = [0.0; 2];
                for ch in 0..2 {
                    y[ch] = self.tick_other(c, ch, g[ch], v[ch], t);
                }
                y
            }
        };
        if let Core::Comb(_) = c.core {
            self.wpos = (self.wpos + 1) % DELAY_LEN;
        } else if c.core == Core::Diffusor {
            self.wpos = (self.wpos + 1) % (DELAY_LEN / 4);
        }
        [x[0] + (y[0] - x[0]) * c.mix, x[1] + (y[1] - x[1]) * c.mix]
    }

    /// The less common cores, one channel at a time.
    #[inline(always)]
    fn tick_other(&mut self, c: &Coeffs, ch: usize, g: f32, v: f32, t: f32) -> f32 {
        match c.core {
            Core::OnePole { high, twelve } => {
                let lp = self.one[ch].tick(v, g / (1.0 + g));
                let y1 = if high { v - lp } else { lp };
                if twelve {
                    let a1 = 1.0 / (1.0 + g * (g + c.k));
                    pick(if high { Out::High } else { Out::Low }, self.svf[ch][0].tick(y1, g, c.k, a1), y1, c.k)
                } else {
                    y1
                }
            }
            Core::Eq(e) => {
                let a = c.eq_a;
                let shelf = |s: &mut Svf, x: f32, g: f32, k: f32, low: bool, a: f32| -> f32 {
                    let gs = if low { g / a.sqrt() } else { g * a.sqrt() };
                    let a1 = 1.0 / (1.0 + gs * (gs + k));
                    let (lp, bp, _) = s.tick(x, gs, k, a1);
                    if low { x + k * (a - 1.0) * bp + (a * a - 1.0) * lp } else { a * a * x + k * (1.0 - a) * a * bp + (1.0 - a * a) * lp }
                };
                let st = &mut self.svf[ch];
                match e {
                    Eq::LowShelf => shelf(&mut st[0], v, g, c.eq_k, true, a),
                    Eq::HighShelf => shelf(&mut st[0], v, g, c.eq_k, false, a),
                    Eq::Bell => {
                        let k = c.eq_k / a;
                        let a1 = 1.0 / (1.0 + g * (g + k));
                        let (_, bp, _) = st[0].tick(v, g, k, a1);
                        v + k * (a * a - 1.0) * bp
                    }
                    Eq::Tilt => {
                        let y = shelf(&mut st[0], v, g, c.eq_k, true, 1.0 / a.sqrt());
                        shelf(&mut st[1], y, g, c.eq_k, false, a.sqrt())
                    }
                }
            }
            Core::Morph(m, stages) => {
                let a1 = 1.0 / (1.0 + g * (g + c.k));
                let st = &mut self.svf[ch];
                let mut y = morph(m, c.var, st[0].tick(v, g, c.k, a1), v, c.k);
                if stages > 1 {
                    let a1b = 1.0 / (1.0 + g * (g + SQRT_2));
                    y = morph(m, c.var, st[1].tick(y, g, SQRT_2, a1b), y, SQRT_2);
                }
                y
            }
            Core::Sk(o, scream) => {
                let a1 = 1.0 / (1.0 + g * (g + c.k));
                let s = &mut self.svf[ch][0];
                let x = if scream { 2.0 * v } else { v };
                let r = s.tick(x, g, c.k, a1);
                // the band integrator saturates: resonance limits itself like an OTA filter
                let lim = if scream { 1.0 } else { 2.0 };
                s.ic1 = lim * fast_tanh(s.ic1 / lim);
                let y = pick(o, r, x, c.k);
                if scream { 0.5 * y } else { y }
            }
            Core::Comb(kind) => {
                let d = c.d0[ch] + c.dd[ch] * t;
                let tap = self.read(ch, 0, DELAY_LEN, d);
                let (w, y) = match kind {
                    Comb::Feedback(_) => {
                        self.damp[ch] += (tap - self.damp[ch]) * c.damp;
                        let w = v + c.fb * self.damp[ch];
                        (w, w * (1.0 - c.fb.abs()).sqrt())
                    }
                    Comb::Dist(_) => {
                        self.damp[ch] += (tap - self.damp[ch]) * c.damp;
                        let w = v + c.fb * fast_tanh(1.5 * self.damp[ch]);
                        (w, w * (1.0 - c.fb.abs()).sqrt())
                    }
                    Comb::Forward(_) => (v, 0.5 * (v + c.fb * tap)),
                    Comb::Allpass => {
                        let w = v + c.fb * tap;
                        (w, tap - c.fb * w)
                    }
                };
                self.delay[ch][self.wpos] = flush(w);
                y
            }
            Core::Phaser(stages, _) => {
                let mut u = v + c.pfb * self.fbv[ch];
                for s in 0..stages as usize {
                    let gg = c.pg[ch][s];
                    let st = &mut self.ap[ch][s];
                    let w = gg * (u - *st);
                    let lp = w + *st;
                    *st = lp + w;
                    u = 2.0 * lp - u;
                }
                self.fbv[ch] = u;
                0.5 * (v + u)
            }
            Core::Formant(_) => {
                let mut y = 0.0;
                for b in 0..c.bands {
                    let gb = c.fg[ch][b];
                    let a1 = 1.0 / (1.0 + gb * (gb + c.k));
                    let (_, bp, _) = self.form[ch][b].tick(v, gb, c.k, a1);
                    y += FORMANT_GAIN[b] * c.k * bp;
                }
                2.0 * y
            }
            Core::Ring => {
                let s = sin_turns(self.phase[ch]);
                let sq = fast_tanh(4.0 * s);
                let wave = s + (sq - s) * c.var;
                self.phase[ch] += c.inc[ch];
                self.phase[ch] -= self.phase[ch].floor();
                v * (c.res + (1.0 - c.res) * wave)
            }
            Core::Hold => {
                self.phase[ch] += c.inc[ch];
                if self.phase[ch] >= 1.0 {
                    self.phase[ch] -= self.phase[ch].floor();
                    self.hold[ch] = v;
                }
                let a = 1.0 - 0.97 * c.res;
                self.smooth[ch] += (self.hold[ch] - self.smooth[ch]) * a;
                self.smooth[ch]
            }
            Core::Diffusor => {
                let base = c.d0[ch] + c.dd[ch] * t;
                let size = DELAY_LEN / 4;
                let mut u = v;
                for (q, r) in DIFFUSE.iter().enumerate() {
                    let d = (base * r).max(2.0);
                    let tap = self.read(ch, q * size, size, d);
                    let w = u + c.fb * tap;
                    self.delay[ch][q * size + self.wpos % size] = flush(w);
                    u = tap - c.fb * w;
                }
                u
            }
            Core::Reject => {
                let h = c.h0[ch] + c.dh[ch] * t;
                let st = &mut self.svf[ch];
                let a1 = 1.0 / (1.0 + g * (g + c.k));
                let a1h = 1.0 / (1.0 + h * (h + c.k));
                st[0].tick(v, g, c.k, a1).0 + st[1].tick(v, h, c.k, a1h).2
            }
            Core::Svf(..) | Core::Ladder(..) => unreachable!("handled in tick"),
        }
    }

    /// Flush tiny state values to zero (once per sub-block).
    pub fn finish(&mut self) {
        for l in self.ladder.iter_mut() {
            l.flush();
        }
        for ch in 0..2 {
            for s in self.svf[ch].iter_mut().chain(self.form[ch].iter_mut()) {
                s.flush();
            }
            self.one[ch].s = flush(self.one[ch].s);
            for a in self.ap[ch].iter_mut() {
                *a = flush(*a);
            }
            self.fbv[ch] = flush(self.fbv[ch]);
            self.damp[ch] = flush(self.damp[ch]);
            self.smooth[ch] = flush(self.smooth[ch]);
        }
    }

    /// Filter `buf` (stereo, samples `start..len`) in place.
    pub fn process(&mut self, p: &FilterParams, sr: f32, buf: &mut [[f32; MAX_BLOCK]; 2], start: usize, len: usize) {
        let c = self.prepare(p, sr, len);
        for i in start..len {
            let y = self.tick(&c, i, [buf[0][i], buf[1][i]]);
            buf[0][i] = y[0];
            buf[1][i] = y[1];
        }
        self.finish();
    }

    /// Any subnormal value left in the state? (Tests: denormals are slow in wasm.)
    pub fn has_subnormal(&self) -> bool {
        let sub = |v: f32| v != 0.0 && v.abs() < f32::MIN_POSITIVE;
        self.svf.iter().flatten().chain(self.form.iter().flatten()).any(|s| sub(s.ic1) || sub(s.ic2))
            || self.ladder.iter().any(|l| l.s.iter().any(|&v| sub(v)))
            || self.ap.iter().flatten().any(|&v| sub(v))
            || self.delay.iter().flatten().any(|&v| sub(v))
            || [self.fbv, self.damp, self.smooth].iter().flatten().any(|&v| sub(v))
    }
}

/// Choose an SVF output: low, normalized band (unity peak), high, notch, peak or allpass.
#[inline(always)]
fn pick(o: Out, (lp, bp, hp): (f32, f32, f32), x: f32, k: f32) -> f32 {
    match o {
        Out::Low => lp,
        Out::High => hp,
        Out::Band => k * bp,
        Out::Notch => x - k * bp,
        Out::Peak => lp - hp,
        Out::Allpass => x - 2.0 * k * bp,
    }
}

/// Morphing outputs: `var` sweeps low → band → high, low → high, or band → notch.
#[inline(always)]
fn morph(m: Morph, var: f32, (lp, bp, hp): (f32, f32, f32), x: f32, k: f32) -> f32 {
    let band = k * bp;
    match m {
        Morph::LowBandHigh => {
            let w = 2.0 * var;
            if w < 1.0 { lp + (band - lp) * w } else { band + (hp - band) * (w - 1.0) }
        }
        Morph::LowHigh => lp + (hp - lp) * var,
        Morph::BandNotch => band + ((x - band) - band) * var,
    }
}

// -------------------------------------------------------------- response

/// Magnitude response (linear) of a filter type at frequency `f`: exact for
/// the linear cores, small-signal for the saturating ones, flat for the
/// ones that aren't filters in the linear sense (ring mod, S&H, diffusor).
pub fn response(kind: u8, cutoff: f32, res: f32, var: f32, sr: f32, f: f32) -> f32 {
    use num::C;
    let kind = kind.min(LAST_TYPE);
    let c = core(kind);
    let res = res.clamp(0.0, 1.0);
    let var = var.clamp(0.0, 1.0);
    let tanw = (PI * f.clamp(0.0, sr * 0.4999) / sr).tan();
    // bilinear transform with the cutoff prewarped: s = j·tan(πf/sr)/tan(πfc/sr)
    let s_at = |fc: f32| C::new(0.0, tanw / g_of(fc, sr));
    let s = s_at(cutoff);
    let one = C::new(1.0, 0.0);
    let svf = |s: C, k: f32| -> (C, C, C) {
        let den = s.mul(s).add(s.scale(k)).add(one);
        (one.div(den), s.div(den), s.mul(s).div(den))
    };
    let out = |o: Out, (lp, bp, hp): (C, C, C), k: f32| -> C {
        match o {
            Out::Low => lp,
            Out::High => hp,
            Out::Band => bp.scale(k),
            Out::Notch => one.sub(bp.scale(k)),
            Out::Peak => lp.sub(hp),
            Out::Allpass => one.sub(bp.scale(2.0 * k)),
        }
    };
    let k = svf_k(res);
    let h = match c {
        Core::Svf(o, stages) => {
            let mut h = out(o, svf(s, k), k);
            for _ in 1..stages {
                h = h.mul(out(o, svf(s, SQRT_2), SQRT_2));
            }
            h
        }
        Core::OnePole { high, twelve } => {
            let lp = one.div(one.add(s));
            let h1 = if high { one.sub(lp) } else { lp };
            if twelve { h1.mul(out(if high { Out::High } else { Out::Low }, svf(s, k), k)) } else { h1 }
        }
        Core::Eq(e) => {
            let a = 10f32.powf(eq_gain_db(res) / 40.0);
            let kq = 1.0 / eq_q(e, var);
            let shelf = |low: bool, a: f32| -> C {
                // the shelves run the SVF at a shifted cutoff: s' = s·√A (low) or s/√A (high)
                let ss = if low { s.scale(a.sqrt()) } else { s.scale(1.0 / a.sqrt()) };
                let (lp, bp, _) = svf(ss, kq);
                if low { one.add(bp.scale(kq * (a - 1.0))).add(lp.scale(a * a - 1.0)) } else { one.scale(a * a).add(bp.scale(kq * (1.0 - a) * a)).add(lp.scale(1.0 - a * a)) }
            };
            match e {
                Eq::LowShelf => shelf(true, a),
                Eq::HighShelf => shelf(false, a),
                Eq::Bell => {
                    let kb = kq / a;
                    let (_, bp, _) = svf(s, kb);
                    one.add(bp.scale(kb * (a * a - 1.0)))
                }
                Eq::Tilt => shelf(true, 1.0 / a.sqrt()).mul(shelf(false, a.sqrt())),
            }
        }
        Core::Morph(m, stages) => {
            let mo = |(lp, bp, hp): (C, C, C), k: f32| -> C {
                let band = bp.scale(k);
                match m {
                    Morph::LowBandHigh => {
                        let w = 2.0 * var;
                        if w < 1.0 { lp.add(band.sub(lp).scale(w)) } else { band.add(hp.sub(band).scale(w - 1.0)) }
                    }
                    Morph::LowHigh => lp.add(hp.sub(lp).scale(var)),
                    Morph::BandNotch => band.add(one.sub(band).sub(band).scale(var)),
                }
            };
            let mut h = mo(svf(s, k), k);
            if stages > 1 {
                h = h.mul(mo(svf(s, SQRT_2), SQRT_2));
            }
            h
        }
        Core::Ladder(fl, tap) => {
            let kl = ladder_k(fl, res);
            let r = ladder_ratios(fl);
            let g: [C; 4] = std::array::from_fn(|i| one.div(one.add(s_at(cutoff * r[i]))));
            let gp = g[0].mul(g[1]).mul(g[2]).mul(g[3]);
            let u = one.div(one.add(gp.scale(kl)));
            let y1 = u.mul(g[0]);
            let y2 = y1.mul(g[1]);
            let y3 = y2.mul(g[2]);
            let y4 = y3.mul(g[3]);
            match tap {
                Tap::Low(n) => [y1, y2, y3, y4][n - 1].scale(ladder_comp(fl, kl)),
                Tap::High => u.sub(y1.scale(4.0)).add(y2.scale(6.0)).sub(y3.scale(4.0)).add(y4),
                Tap::Band => y2.sub(y3.scale(2.0)).add(y4).scale(4.0),
            }
        }
        Core::Sk(o, scream) => {
            let ks = k * if scream { 0.5 } else { 1.0 };
            out(o, svf(s, ks), ks)
        }
        Core::Comb(kind) => {
            let d = comb_delay(cutoff, sr) as f64;
            let w = std::f64::consts::TAU * f as f64 / sr as f64;
            let z = C::from_f64((w * d).cos(), -(w * d).sin()); // e^{-jωD}
            let sign = match kind {
                Comb::Feedback(neg) | Comb::Forward(neg) | Comb::Dist(neg) => {
                    if neg {
                        -1.0
                    } else {
                        1.0
                    }
                }
                Comb::Allpass => 1.0,
            };
            match kind {
                Comb::Forward(_) => one.add(z.scale(sign * (0.5 + 0.5 * res))).scale(0.5),
                Comb::Allpass => one,
                _ => {
                    // the loop's one-pole damping, as a one-sample smoother
                    let a = (0.15 + 0.85 * var) as f64;
                    let zm = C::from_f64(w.cos(), -w.sin());
                    let damp = C::from_f64(a, 0.0).div(one.sub(zm.scale((1.0 - a) as f32)));
                    let fb = sign * 0.98 * res;
                    one.div(one.sub(z.mul(damp).scale(fb))).scale((1.0 - fb.abs()).sqrt())
                }
            }
        }
        Core::Phaser(stages, neg) => {
            let mut a = one;
            for st in 0..stages as usize {
                let si = s_at(cutoff * phaser_ratio(st, stages, var));
                a = a.mul(one.sub(si).div(one.add(si)));
            }
            let w = std::f64::consts::TAU * f as f64 / sr as f64;
            let zm = C::from_f64(w.cos(), -w.sin());
            let fb = if neg { -0.9 * res } else { 0.9 * res };
            // u = v + fb·z⁻¹·u_chain, u_chain = A·u  →  y = ½(v + A·v / (1 - fb·z⁻¹·A))
            let chain = a.div(one.sub(zm.mul(a).scale(fb)));
            one.add(chain).scale(0.5)
        }
        Core::Formant(set) => {
            let (fr, bands) = formant_freqs(set, var, cutoff / 1000.0);
            let kf = formant_k(res);
            let mut h = C::new(0.0, 0.0);
            for b in 0..bands {
                let (_, bp, _) = svf(s_at(fr[b]), kf);
                h = h.add(bp.scale(FORMANT_GAIN[b] * kf * 2.0));
            }
            h
        }
        Core::Ring | Core::Hold | Core::Diffusor => one,
        Core::Reject => {
            let wr = reject_width(var).exp2();
            let (lp, _, _) = svf(s_at(cutoff / wr), k);
            let (_, _, hp) = svf(s_at(cutoff * wr), k);
            lp.add(hp)
        }
    };
    h.abs()
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
        pub fn from_f64(re: f64, im: f64) -> C {
            C { re, im }
        }
        pub fn add(self, o: C) -> C {
            C { re: self.re + o.re, im: self.im + o.im }
        }
        pub fn sub(self, o: C) -> C {
            C { re: self.re - o.re, im: self.im - o.im }
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
    const N: usize = 16;

    /// Measured magnitude at `f` from a steady sine through the filter.
    fn measure(p: &FilterParams, f: f32) -> f32 {
        let mut st = FilterState::default();
        let amp = 1e-3; // stay in the linear region of the saturating cores
        let blocks = (SR as usize * 2) / N;
        let (mut dot_s, mut dot_c, mut count) = (0.0f64, 0.0f64, 0usize);
        for b in 0..blocks {
            let mut buf = [[0.0f32; MAX_BLOCK]; 2];
            for i in 0..N {
                let t = (b * N + i) as f64 / SR as f64;
                let v = (amp * (std::f64::consts::TAU * f as f64 * t).sin()) as f32;
                buf[0][i] = v;
                buf[1][i] = v;
            }
            st.process(p, SR, &mut buf, 0, N);
            if b >= blocks / 2 {
                for i in 0..N {
                    let t = (b * N + i) as f64 / SR as f64;
                    let ph = std::f64::consts::TAU * f as f64 * t;
                    dot_s += buf[0][i] as f64 * ph.sin();
                    dot_c += buf[0][i] as f64 * ph.cos();
                    count += 1;
                }
            }
        }
        ((2.0 * (dot_s * dot_s + dot_c * dot_c).sqrt() / count as f64) / amp) as f32
    }

    /// Types whose response is analytic at small signal levels (combs read a
    /// fractionally interpolated delay; ring mod, S&H and the diffusor aren't
    /// linear time-invariant filters).
    fn analytic(kind: u8) -> bool {
        !matches!(core(kind), Core::Comb(_) | Core::Ring | Core::Hold | Core::Diffusor)
    }

    #[test]
    fn measured_response_matches_the_analytic_one() {
        let db = |x: f32| 20.0 * x.max(1e-9).log10();
        for kind in (0..=LAST_TYPE).filter(|&k| analytic(k)) {
            for &(cutoff, res, var) in &[(200.0f32, 0.0f32, 0.5f32), (1000.0, 0.5, 0.25), (5000.0, 0.9, 0.8)] {
                for &f in &[60.0f32, 330.0, 1000.0, 2400.0, 7000.0, 15000.0] {
                    let p = FilterParams { var, ..FilterParams::new(kind, cutoff, res) };
                    let want = response(kind, cutoff, res, var, SR, f);
                    if db(want) < -60.0 {
                        continue; // below the measurement's noise floor
                    }
                    let got = measure(&p, f);
                    assert!((db(got) - db(want)).abs() < 0.3, "{} fc {cutoff} res {res} var {var} f {f}: {:.2} dB vs {:.2} dB", NAMES[kind as usize], db(got), db(want));
                }
            }
        }
    }

    #[test]
    fn combs_notch_and_peak_where_they_should() {
        // a feedback comb at 500 Hz peaks at multiples of 500 Hz; a feed-forward (-) one notches at DC and 500 Hz
        let pk = FilterParams::new(43, 500.0, 0.9);
        let at = |p: &FilterParams, f: f32| measure(p, f);
        assert!(at(&pk, 1000.0) > 4.0 * at(&pk, 750.0), "comb+ peaks at harmonics");
        let ff = FilterParams::new(46, 500.0, 1.0);
        assert!(at(&ff, 500.0) < 0.05 && at(&ff, 250.0) > 0.9, "flange- notches: {} {}", at(&ff, 500.0), at(&ff, 250.0));
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
                let cutoff = 60.0 * (1.0 + 300.0 * (0.5 + 0.5 * (t * std::f32::consts::TAU).sin()));
                let p = FilterParams { drive: 0.5, var: 0.3, stereo: 0.4, ..FilterParams::new(kind, cutoff, 1.0) };
                let mut buf = [[0.0f32; MAX_BLOCK]; 2];
                for i in 0..N {
                    let ph = ((b * N + i) as f32 * 110.0 / SR).fract();
                    buf[0][i] = 2.0 * ph - 1.0;
                    buf[1][i] = buf[0][i];
                }
                st.process(&p, SR, &mut buf, 0, N);
                for &v in &buf[0][..N] {
                    assert!(v.is_finite(), "{} went non-finite", NAMES[kind as usize]);
                    peak = peak.max(v.abs());
                }
            }
            assert!(peak < 100.0, "{} peaked at {peak}", NAMES[kind as usize]);
        }
    }

    /// Cutoff jumping to a random value every sub-block (audio-rate modulation
    /// through the matrix), at full resonance and drive, at 4× oversampling too.
    #[test]
    fn every_type_is_stable_under_audio_rate_modulation() {
        let mut rng = crate::rng::Rng::new(77);
        for kind in 0..=LAST_TYPE {
            for &(sr, len) in &[(SR, N), (SR * 4.0, N * 4)] {
                let mut st = FilterState::default();
                let mut peak = 0.0f32;
                for b in 0..(10.0 * SR) as usize / N {
                    let cutoff = 20.0 * 1000f32.powf(rng.next_f32());
                    let p = FilterParams { drive: 1.0, var: rng.next_f32(), stereo: rng.next_f32() * 2.0 - 1.0, ..FilterParams::new(kind, cutoff, 1.0) };
                    let mut buf = [[0.0f32; MAX_BLOCK]; 2];
                    for i in 0..len {
                        let ph = ((b * len + i) as f32 * 220.0 / sr).fract();
                        buf[0][i] = if ph < 0.5 { 1.0 } else { -1.0 };
                        buf[1][i] = -buf[0][i];
                    }
                    st.process(&p, sr, &mut buf, 0, len);
                    for ch in &buf {
                        for &v in &ch[..len] {
                            assert!(v.is_finite(), "{} went non-finite at {sr} Hz", NAMES[kind as usize]);
                            peak = peak.max(v.abs());
                        }
                    }
                }
                assert!(peak < 100.0, "{} peaked at {peak} under modulation", NAMES[kind as usize]);
            }
        }
    }

    #[test]
    fn silence_leaves_no_subnormals() {
        for kind in 0..=LAST_TYPE {
            let mut st = FilterState::default();
            let p = FilterParams::new(kind, 800.0, 0.95);
            for b in 0..(60.0 * SR) as usize / N {
                let mut buf = [[0.0f32; MAX_BLOCK]; 2];
                if b == 0 {
                    buf[0][0] = 1.0;
                    buf[1][0] = 1.0;
                }
                st.process(&p, SR, &mut buf, 0, N);
            }
            assert!(!st.has_subnormal(), "{} left subnormal state after 60 s of silence", NAMES[kind as usize]);
        }
    }

    #[test]
    fn reset_is_cheap_and_clears_delays_before_reuse() {
        let mut st = FilterState::default();
        let p = FilterParams::new(43, 300.0, 0.9);
        let mut buf = [[0.5f32; MAX_BLOCK]; 2];
        st.process(&p, SR, &mut buf, 0, N);
        st.reset();
        st.reset(); // second reset does nothing
        let mut quiet = [[0.0f32; MAX_BLOCK]; 2];
        st.process(&p, SR, &mut quiet, 0, N);
        assert!(quiet[0][..N].iter().all(|&v| v == 0.0), "old comb audio leaked after a reset");
    }

    #[test]
    fn names_and_types_line_up() {
        assert_eq!(NAMES.len(), COUNT);
        assert_eq!(NAMES[LADDER24 as usize], "Ladder Low 24");
        for k in 0..=LAST_TYPE {
            let _ = core(k);
            assert!(!group(k).is_empty());
        }
    }
}
