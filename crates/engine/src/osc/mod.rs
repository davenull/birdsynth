//! Wavetable oscillators: per-voice state and the per-sub-block setup that
//! turns resolved parameters into a kernel call.

pub mod kernel;
pub mod sub;
pub mod unison;

use wt_dsp::mip;
use wt_dsp::rng::Rng;
use wt_dsp::warp;

use kernel::Kernel;
use unison::{Layout, MAX_LANES, UniParams};

use crate::filter::MAX_N;

pub const PITCH_SEMITONES: u8 = 0;
pub const PITCH_HARMONICS: u8 = 1;
pub const PITCH_RATIO: u8 = 2;

/// Frequency multiplier of the Harmonics and Ratio pitch modes: semitone
/// value n selects harmonic n+1, or subharmonic 1/(1-n) below zero.
#[inline]
pub fn harmonic(semi: f32) -> f32 {
    let n = semi.round();
    if n >= 0.0 { n + 1.0 } else { 1.0 / (1.0 - n) }
}

// ------------------------------------------------------ cross-modulation

/// Modulation sources a cross-mod warp can read (in warp-mode order).
pub const SRC_A: u8 = 0;
pub const SRC_B: u8 = 1;
pub const SRC_C: u8 = 2;
pub const SRC_SUB: u8 = 3;
pub const SRC_NOISE: u8 = 4;
pub const SRC_F1: u8 = 5;
pub const SRC_F2: u8 = 6;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum XKind {
    /// Through-zero FM: the increment scales by 1 + depth·x.
    Fm,
    /// Phase distortion: the read phase moves by depth·x cycles.
    Pd,
    /// Phase distortion from the oscillator's own previous output.
    PdSelf,
    /// Amplitude modulation, unipolar: gain 1 - a + a·(x+1)/2.
    Am,
    /// Ring modulation: gain 1 - a + a·x.
    Rm,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XMod {
    pub kind: XKind,
    pub src: u8,
    pub amount: f32,
}

/// The cross-mod a warp mode (params/osc.toml numbering) asks for, if any.
pub fn xmod(mode: u8, amount: f32) -> Option<XMod> {
    if amount <= 0.0 {
        return None;
    }
    let (kind, src) = match mode {
        17..=23 => (XKind::Fm, mode - 17),
        24..=30 => (XKind::Pd, mode - 24),
        31 => (XKind::PdSelf, 0),
        32..=38 => (XKind::Am, mode - 32),
        39..=45 => (XKind::Rm, mode - 39),
        _ => return None,
    };
    Some(XMod { kind, src, amount })
}

pub const FM_DEPTH: f32 = 8.0;
pub const PD_DEPTH: f32 = 2.0;

/// How much faster a cross-mod can drive the phase (for mip selection).
pub fn xstretch(x: &XMod) -> f32 {
    match x.kind {
        XKind::Fm => 1.0 + FM_DEPTH * x.amount,
        XKind::Pd => 1.0 + 4.0 * PD_DEPTH * x.amount,
        XKind::PdSelf => 1.0 + 4.0 * x.amount,
        XKind::Am | XKind::Rm => 1.0,
    }
}

/// Does this warp create harmonics that oversampling helps with?
pub fn wants_oversampling(mode: u8, amount: f32) -> bool {
    amount > 0.0 && (matches!(mode, warp::SYNC | warp::WINDOW_SYNC) || (17..=58).contains(&mode))
}

// ------------------------------------------------------------ oscillator

/// Everything the oscillator needs this sub-block, already resolved per voice.
#[derive(Clone, Copy, Debug)]
pub struct OscSettings {
    /// Frequency in Hz at the end of the sub-block.
    pub hz: f32,
    pub pos: f32,
    pub smooth: bool,
    pub uni: UniParams,
    pub wt_spread: f32,
    pub warp_spread: f32,
    pub w1: (u8, f32),
    pub w2: (u8, f32),
    /// Extra phase speed from cross-modulation (1 without).
    pub xstretch: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct OscVoice {
    pub phase: [u32; MAX_LANES],
    /// Per-note random positions for the Random unison mode.
    random: [f32; MAX_LANES],
    layout: Layout,
    layout_for: Option<UniKey>,
    inc_prev: f32,
    pos_prev: f32,
    pub primed: bool,
}

/// What the cached layout was computed from.
#[derive(Clone, Copy, Debug, PartialEq)]
struct UniKey {
    voices: usize,
    detune: f32,
    blend: f32,
    width: f32,
    range: f32,
    mode: u8,
    stack: u8,
}

impl From<&UniParams> for UniKey {
    fn from(u: &UniParams) -> Self {
        UniKey { voices: u.voices, detune: u.detune, blend: u.blend, width: u.width, range: u.range, mode: u.mode, stack: u.stack }
    }
}

impl Default for OscVoice {
    fn default() -> Self {
        OscVoice { phase: [0; MAX_LANES], random: [0.5; MAX_LANES], layout: Layout::default(), layout_for: None, inc_prev: 0.0, pos_prev: 0.0, primed: false }
    }
}

impl OscVoice {
    /// Set up for a new note: start phases (fraction of a cycle plus a
    /// random share) or, with phase memory, the phases given.
    pub fn start(&mut self, phase: f32, rand: f32, memory: Option<&[u32; MAX_LANES]>, rng: &mut Rng) {
        *self = OscVoice::default();
        match memory {
            Some(m) => self.phase = *m,
            None => {
                for p in self.phase.iter_mut() {
                    let f = phase + rand * rng.next_f32();
                    *p = ((f - f.floor()) as f64 * 4_294_967_296.0) as u32;
                }
            }
        }
        for r in self.random.iter_mut() {
            *r = rng.next_f32();
        }
    }

    /// Build the kernel for the next `len` samples at `sr` (and move the
    /// oscillator's pitch and position ramps on).
    pub fn prepare<'a>(&mut self, s: &OscSettings, table: &'a [f32], frames: usize, sr: f32, len: usize) -> Kernel<'a> {
        let key = UniKey::from(&s.uni);
        if self.layout_for != Some(key) {
            self.layout = unison::layout(&s.uni, &self.random);
            self.layout_for = Some(key);
        }
        let lay = &self.layout;
        let inc1 = s.hz / sr;
        let inc0 = if self.primed { self.inc_prev } else { inc1 };
        let pos1 = s.pos.clamp(0.0, 1.0);
        let pos0 = if self.primed { self.pos_prev } else { pos1 };
        self.primed = true;
        self.inc_prev = inc1;
        self.pos_prev = pos1;
        let inc_step = if inc0 > 0.0 && inc0 != inc1 { (inc1 / inc0).powf(1.0 / len as f32) } else { 1.0 };

        let mut k = Kernel {
            len,
            table,
            frames,
            pick: mip::Pick { lo: 0, hi: 0, w: 0.0 },
            lanes: lay.n,
            inc0,
            inc_step,
            ratio: lay.ratio,
            gl: lay.gl,
            gr: lay.gr,
            fpos0: [0.0; MAX_LANES],
            fpos_step: [0.0; MAX_LANES],
            smooth: s.smooth,
            w1_mode: s.w1.0,
            w1_amt: [0.0; MAX_LANES],
            w2_mode: s.w2.0,
            w2_amt: [0.0; MAX_LANES],
            warped: false,
        };
        let span = (frames - 1) as f32;
        let (mut max_ratio, mut w1_max, mut w2_max) = (0.0f32, 0.0f32, 0.0f32);
        for l in 0..lay.n {
            let x = lay.spread[l];
            let off = s.wt_spread * x * 0.5;
            let f0 = (pos0 + off).clamp(0.0, 1.0) * span;
            let f1 = (pos1 + off).clamp(0.0, 1.0) * span;
            k.fpos0[l] = f0;
            k.fpos_step[l] = (f1 - f0) / len as f32;
            let woff = s.warp_spread * x * 0.5;
            k.w1_amt[l] = (s.w1.1 + woff).clamp(0.0, 1.0);
            k.w2_amt[l] = (s.w2.1 + woff).clamp(0.0, 1.0);
            max_ratio = max_ratio.max(lay.ratio[l]);
            w1_max = w1_max.max(k.w1_amt[l]);
            w2_max = w2_max.max(k.w2_amt[l]);
            k.warped |= warp::active(s.w1.0, k.w1_amt[l]) || warp::active(s.w2.0, k.w2_amt[l]);
        }
        // band-limit for the fastest the phase can move this sub-block
        let stretch = warp::stretch(s.w1.0, w1_max) * warp::stretch(s.w2.0, w2_max) * s.xstretch;
        k.pick = mip::pick(inc0.max(inc1) * max_ratio * stretch, sr);
        k
    }

    /// Render samples `start..len` (unleveled stereo) with the block kernels.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, s: &OscSettings, table: &[f32], frames: usize, sr: f32, start: usize, len: usize, scalar: bool, out: &mut [[f32; MAX_N]; 2]) {
        let k = self.prepare(s, table, frames, sr, len);
        let [l, r] = out;
        if scalar {
            kernel::render_scalar(&k, &mut self.phase, start, l, r);
        } else {
            kernel::render_simd(&k, &mut self.phase, start, l, r);
        }
    }
}
