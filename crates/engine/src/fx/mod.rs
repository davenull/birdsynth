//! The FX racks: Main (the mix) and two buses (fed by each source's sends),
//! each an ordered chain of effect modules. There are fourteen module types
//! with four instances each; a chain lists instances by type × 256 +
//! instance. Splitters split the signal into bands and run a chain per band
//! (band chains can't hold another splitter).
//!
//! Modules read their parameters, with the global modulation, once per
//! sub-block. Switching a module off fades its wet signal out over 5 ms
//! (and resets it once silent, so it starts clean next time); a module that
//! didn't run in the previous sub-block (just added to a chain) is reset
//! before it runs. When a chain's order changes, the chain fades to its dry
//! input, swaps, and fades back, so reordering never clicks.

pub mod bode;
pub mod convolve;
pub mod delay;
pub mod distortion;
pub mod dynamics;
pub mod eq;
pub mod modulation;
pub mod reverb;
pub mod util;
pub mod utility;

use crate::modmatrix::{Matrix, NONE};
use crate::params::ParamStore;
use crate::spec::params as p;
use crate::spec::protocol::MAX_MOD_SLOTS;
use crate::tables::AssetBuf;

use util::{Block, N};

pub const HYPER: u8 = 0;
pub const DISTORTION: u8 = 1;
pub const FLANGER: u8 = 2;
pub const PHASER: u8 = 3;
pub const CHORUS: u8 = 4;
pub const DELAY: u8 = 5;
pub const COMPRESSOR: u8 = 6;
pub const REVERB: u8 = 7;
pub const EQ: u8 = 8;
pub const FILTER: u8 = 9;
pub const BODE: u8 = 10;
pub const CONVOLVE: u8 = 11;
pub const UTILITY: u8 = 12;
pub const SPLITTER: u8 = 13;
pub const TYPES: usize = 14;
pub const INSTANCES: usize = 4;

pub const MAIN: usize = 0;
pub const BUS1: usize = 1;
pub const BUS2: usize = 2;
/// Main, the two buses, then three band chains per splitter.
pub const CHAINS: usize = 3 + INSTANCES * 3;
pub const MAX_CHAIN: usize = 16;

/// Bypass and reorder fade time.
const FADE_MS: f32 = 5.0;

/// A chain entry: module type and instance.
#[inline]
pub fn entry(ty: u8, inst: u8) -> u16 {
    (ty as u16) << 8 | inst as u16
}

/// The band chain index for a splitter band.
#[inline]
pub fn band_chain(inst: usize, band: usize) -> usize {
    3 + inst * 3 + band
}

/// Each type's On parameter, by instance.
fn enable_id(ty: u8, i: usize) -> u16 {
    match ty {
        HYPER => p::FX_HYPER_ENABLE[i],
        DISTORTION => p::FX_DISTORTION_ENABLE[i],
        FLANGER => p::FX_FLANGER_ENABLE[i],
        PHASER => p::FX_PHASER_ENABLE[i],
        CHORUS => p::FX_CHORUS_ENABLE[i],
        DELAY => p::FX_DELAY_ENABLE[i],
        COMPRESSOR => p::FX_COMPRESSOR_ENABLE[i],
        REVERB => p::FX_REVERB_ENABLE[i],
        EQ => p::FX_EQ_ENABLE[i],
        FILTER => p::FX_FILTER_ENABLE[i],
        BODE => p::FX_BODE_ENABLE[i],
        CONVOLVE => p::FX_CONVOLVE_ENABLE[i],
        UTILITY => p::FX_UTILITY_ENABLE[i],
        _ => p::FX_SPLITTER_ENABLE[i],
    }
}

/// What the effects see for one sub-block.
pub struct Ctx<'a> {
    pub sr: f32,
    pub bpm: f32,
    /// Song position at the start of the sub-block, in beats.
    pub beat: f64,
    /// The newest voice's pitch (semitones), for key tracking.
    pub note: f32,
    params: &'a ParamStore,
    matrix: &'a Matrix,
    off: &'a [f32; MAX_MOD_SLOTS],
}

impl<'a> Ctx<'a> {
    pub fn new(sr: f32, bpm: f32, beat: f64, note: f32, params: &'a ParamStore, matrix: &'a Matrix, off: &'a [f32; MAX_MOD_SLOTS]) -> Ctx<'a> {
        Ctx { sr, bpm, beat, note, params, matrix, off }
    }

    /// A parameter's value with the global modulation applied.
    #[inline]
    pub fn p(&self, id: u16) -> f32 {
        match self.matrix.dest_of[id as usize] {
            NONE => self.params.plain(id),
            d => self.params.modded(id, self.off[d as usize]),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Chain {
    refs: [u16; MAX_CHAIN],
    n: usize,
    next: [u16; MAX_CHAIN],
    next_n: usize,
    pending: bool,
    /// 1 = the chain's output, 0 = its dry input (during a reorder).
    gain: f32,
}

impl Default for Chain {
    fn default() -> Self {
        Chain { refs: [0; MAX_CHAIN], n: 0, next: [0; MAX_CHAIN], next_n: 0, pending: false, gain: 1.0 }
    }
}

#[derive(Clone, Copy, Debug)]
struct Slot {
    /// Wet share, faded toward the On switch.
    wet: f32,
    /// Sub-block counter when it last ran (MAX: never).
    ran: u64,
    /// Reset since it last faded to silence.
    idle: bool,
}

impl Default for Slot {
    fn default() -> Self {
        Slot { wet: 0.0, ran: u64::MAX, idle: true }
    }
}

pub struct Fx {
    sr: f32,
    hyper: Vec<modulation::Hyper>,
    distortion: Vec<distortion::Distortion>,
    flanger: Vec<modulation::Flanger>,
    phaser: Vec<modulation::Phaser>,
    chorus: Vec<modulation::Chorus>,
    delay: Vec<delay::Delay>,
    compressor: Vec<dynamics::Compressor>,
    reverb: Vec<reverb::Reverb>,
    eq: Vec<eq::Eq>,
    filter: Vec<eq::FxFilter>,
    bode: Vec<bode::Bode>,
    convolve: Vec<convolve::Convolve>,
    utility: Vec<utility::Utility>,
    splitter: Vec<utility::Splitter>,
    chains: [Chain; CHAINS],
    slots: [[Slot; INSTANCES]; TYPES],
    tick: u64,
}

impl Fx {
    /// Built on the heap: the modules' memory never passes through the stack.
    pub fn new(sr: f32) -> Box<Fx> {
        Box::new(Fx {
            sr,
            hyper: (0..INSTANCES).map(|_| modulation::Hyper::new(sr)).collect(),
            distortion: (0..INSTANCES).map(|_| distortion::Distortion::new()).collect(),
            flanger: (0..INSTANCES).map(|_| modulation::Flanger::new(sr)).collect(),
            phaser: (0..INSTANCES).map(|_| modulation::Phaser::new()).collect(),
            chorus: (0..INSTANCES).map(|_| modulation::Chorus::new(sr)).collect(),
            delay: (0..INSTANCES).map(|_| delay::Delay::new(sr)).collect(),
            compressor: (0..INSTANCES).map(|_| dynamics::Compressor::new()).collect(),
            reverb: (0..INSTANCES).map(|_| reverb::Reverb::new(sr)).collect(),
            eq: (0..INSTANCES).map(|_| eq::Eq::new()).collect(),
            filter: (0..INSTANCES).map(|_| eq::FxFilter::new()).collect(),
            bode: (0..INSTANCES).map(|_| bode::Bode::new()).collect(),
            convolve: (0..INSTANCES).map(|_| convolve::Convolve::new(sr)).collect(),
            utility: (0..INSTANCES).map(|_| utility::Utility::new()).collect(),
            splitter: (0..INSTANCES).map(|_| utility::Splitter::new()).collect(),
            chains: [Chain::default(); CHAINS],
            slots: [[Slot::default(); INSTANCES]; TYPES],
            tick: 0,
        })
    }

    /// Set a chain's modules (invalid entries, and splitters inside band chains, are dropped).
    pub fn set_chain(&mut self, chain: usize, refs: &[u16]) -> bool {
        let Some(c) = self.chains.get_mut(chain) else { return false };
        let mut next = [0u16; MAX_CHAIN];
        let mut n = 0;
        for &r in refs.iter() {
            let (ty, inst) = ((r >> 8) as u8, (r & 0xff) as usize);
            if ty as usize >= TYPES || inst >= INSTANCES || (chain >= 3 && ty == SPLITTER) || n == MAX_CHAIN {
                continue;
            }
            next[n] = r;
            n += 1;
        }
        if c.n == 0 && !c.pending {
            // nothing playing through it yet: no fade needed
            c.refs = next;
            c.n = n;
        } else {
            c.next = next;
            c.next_n = n;
            c.pending = true;
        }
        true
    }

    pub fn chain(&self, chain: usize) -> &[u16] {
        let c = &self.chains[chain];
        &c.refs[..c.n]
    }

    pub fn load_ir(&mut self, inst: usize, asset: Option<AssetBuf>, taps: usize) -> bool {
        match self.convolve.get_mut(inst) {
            Some(c) => c.load(asset, taps),
            None => false,
        }
    }

    pub fn ir_taps(&self, inst: usize) -> usize {
        self.convolve[inst].taps()
    }

    /// Gain reduction (dB) of compressor `inst`, band `b`.
    pub fn gain_reduction(&self, inst: usize, b: usize) -> f32 {
        self.compressor[inst].gr[b]
    }

    /// Clear every effect's state (the engine's Reset).
    pub fn reset(&mut self) {
        for ty in 0..TYPES as u8 {
            for i in 0..INSTANCES {
                self.reset_module(ty, i);
            }
        }
        for c in self.chains.iter_mut() {
            if c.pending {
                c.refs = c.next;
                c.n = c.next_n;
                c.pending = false;
            }
            c.gain = 1.0;
        }
    }

    fn reset_module(&mut self, ty: u8, i: usize) {
        match ty {
            HYPER => self.hyper[i].reset(),
            DISTORTION => self.distortion[i].reset(),
            FLANGER => self.flanger[i].reset(),
            PHASER => self.phaser[i].reset(),
            CHORUS => self.chorus[i].reset(),
            DELAY => self.delay[i].reset(),
            COMPRESSOR => self.compressor[i].reset(),
            REVERB => self.reverb[i].reset(),
            EQ => self.eq[i].reset(),
            FILTER => self.filter[i].reset(),
            BODE => self.bode[i].reset(),
            CONVOLVE => self.convolve[i].reset(),
            UTILITY => self.utility[i].reset(),
            _ => self.splitter[i].reset(),
        }
    }

    /// Run the racks on one sub-block of the buses. Returns what goes to the
    /// master: the Main rack's output plus any bus routed straight there.
    pub fn process(&mut self, cx: &Ctx, main: &mut Block, bus1: &mut Block, bus2: &mut Block) {
        self.tick += 1;
        self.run_chain(cx, BUS1, bus1);
        self.run_chain(cx, BUS2, bus2);
        let to_master = [cx.p(p::RACK_BUS1_TO) >= 0.5, cx.p(p::RACK_BUS2_TO) >= 0.5];
        for ch in 0..2 {
            for n in 0..N {
                if !to_master[0] {
                    main[ch][n] += bus1[ch][n];
                }
                if !to_master[1] {
                    main[ch][n] += bus2[ch][n];
                }
            }
        }
        self.run_chain(cx, MAIN, main);
        for ch in 0..2 {
            for n in 0..N {
                if to_master[0] {
                    main[ch][n] += bus1[ch][n];
                }
                if to_master[1] {
                    main[ch][n] += bus2[ch][n];
                }
            }
        }
    }

    fn run_chain(&mut self, cx: &Ctx, chain: usize, buf: &mut Block) {
        let c = self.chains[chain];
        let step = N as f32 / (FADE_MS * 0.001 * self.sr);
        let target = if c.pending { 0.0 } else { 1.0 };
        let g0 = c.gain;
        let g1 = if target > g0 {
            (g0 + step).min(1.0)
        } else if target < g0 {
            (g0 - step).max(0.0)
        } else {
            g0
        };
        let dry = if g0 < 1.0 || g1 < 1.0 { Some(*buf) } else { None };
        // after a swap the input fades in too, so delays and reverbs record a smooth start
        let fading_in = !c.pending && g0 < 1.0;
        let ramp = |n: usize| g0 + (g1 - g0) * (n + 1) as f32 / N as f32;
        if fading_in {
            for ch in buf.iter_mut() {
                for (n, v) in ch.iter_mut().enumerate() {
                    *v *= ramp(n);
                }
            }
        }
        for &r in &c.refs[..c.n] {
            self.run_module(cx, r, buf);
        }
        if let Some(dry) = dry {
            for ch in 0..2 {
                for n in 0..N {
                    let g = ramp(n);
                    buf[ch][n] = if fading_in { dry[ch][n] * (1.0 - g) + buf[ch][n] } else { dry[ch][n] + (buf[ch][n] - dry[ch][n]) * g };
                }
            }
        }
        let c = &mut self.chains[chain];
        c.gain = g1;
        if c.pending && g1 == 0.0 {
            c.refs = c.next;
            c.n = c.next_n;
            c.pending = false;
            // the new order starts from clean state (tails recorded in the old order would jump)
            let refs = c.refs;
            for &r in &refs[..c.n] {
                self.reset_module((r >> 8) as u8, (r & 0xff) as usize);
            }
        }
    }

    fn run_module(&mut self, cx: &Ctx, r: u16, buf: &mut Block) {
        let (ty, i) = ((r >> 8) as u8, (r & 0xff) as usize);
        let on = cx.p(enable_id(ty, i)) >= 0.5;
        let step = N as f32 / (FADE_MS * 0.001 * self.sr);
        let tick = self.tick;
        let s = self.slots[ty as usize][i];
        // not run last sub-block (just added): start from clean state
        let fresh = s.ran.wrapping_add(1) != tick;
        let w0 = if fresh { if on { 1.0 } else { 0.0 } } else { s.wet };
        let w1 = if on { (w0 + step).min(1.0) } else { (w0 - step).max(0.0) };
        self.slots[ty as usize][i].ran = tick;
        self.slots[ty as usize][i].wet = w1;
        if w0 == 0.0 && w1 == 0.0 {
            if !s.idle {
                self.reset_module(ty, i);
                self.slots[ty as usize][i].idle = true;
            }
            return;
        }
        if fresh && !s.idle {
            self.reset_module(ty, i);
        }
        self.slots[ty as usize][i].idle = false;
        let dry = *buf;
        match ty {
            HYPER => self.hyper[i].process(cx, i, buf),
            DISTORTION => self.distortion[i].process(cx, i, buf),
            FLANGER => self.flanger[i].process(cx, i, buf),
            PHASER => self.phaser[i].process(cx, i, buf),
            CHORUS => self.chorus[i].process(cx, i, buf),
            DELAY => self.delay[i].process(cx, i, buf),
            COMPRESSOR => self.compressor[i].process(cx, i, buf),
            REVERB => self.reverb[i].process(cx, i, buf),
            EQ => self.eq[i].process(cx, i, buf),
            FILTER => self.filter[i].process(cx, i, buf),
            BODE => self.bode[i].process(cx, i, buf),
            CONVOLVE => self.convolve[i].process(cx, i, buf),
            UTILITY => self.utility[i].process(cx, i, buf),
            _ => self.run_splitter(cx, i, buf),
        }
        if w0 < 1.0 || w1 < 1.0 {
            for ch in 0..2 {
                for n in 0..N {
                    let g = w0 + (w1 - w0) * (n + 1) as f32 / N as f32;
                    buf[ch][n] = dry[ch][n] + (buf[ch][n] - dry[ch][n]) * g;
                }
            }
        }
    }

    fn run_splitter(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let mut bands = [[[0.0f32; N]; 2]; 3];
        self.splitter[i].split(cx, i, buf, &mut bands);
        let n = utility::Splitter::bands(cx, i);
        for (b, band) in bands.iter_mut().enumerate().take(n) {
            self.run_chain(cx, band_chain(i, b), band);
        }
        utility::Splitter::join(cx, i, &bands, n, buf);
    }

    /// Any subnormal value in the effects with feedback (tests).
    pub fn has_subnormal(&self) -> bool {
        self.delay.iter().any(delay::Delay::has_subnormal)
            || self.reverb.iter().any(reverb::Reverb::has_subnormal)
            || self.distortion.iter().any(distortion::Distortion::has_subnormal)
            || self.filter.iter().any(eq::FxFilter::has_subnormal)
    }
}

#[cfg(test)]
mod tests;
