//! The modulation matrix: up to 64 slots, each adding a scaled source to a
//! parameter's normalized value, per voice.
//!
//! Each distinct destination gets a small index, so a voice keeps one
//! offset per destination (not per parameter) and unmodulated parameters
//! cost nothing.

use crate::spec::params::COUNT;
use crate::spec::protocol::{MAX_MOD_SLOTS, source};

pub const BIPOLAR: u8 = 1;
pub const BYPASS: u8 = 2;
/// dest_of value for a parameter no slot modulates.
pub const NONE: u8 = u8::MAX;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Slot {
    pub source: u8,
    pub aux: u8,
    pub flags: u8,
    pub dest: u16,
    pub amount: f32,
}

impl Slot {
    fn live(&self) -> bool {
        self.source != source::NONE && self.flags & BYPASS == 0 && (self.dest as usize) < COUNT
    }
}

/// Is a source bipolar (-1..1) by nature? The rest are 0..1.
pub fn bipolar_source(s: u8) -> bool {
    matches!(s, source::NOTE | source::PITCH_BEND)
        || (source::LFO_1..=source::LFO_10_Y).contains(&s)
        || s == source::MPE_X
        || (source::OSC_A..=source::FILTER_2).contains(&s)
}

/// Per-voice source values, indexed by source number.
pub type Sources = [f32; source::COUNT];

pub struct Matrix {
    slots: [Slot; MAX_MOD_SLOTS],
    /// Destination index of each parameter, or NONE.
    pub dest_of: [u8; COUNT],
    /// Parameter id of each destination index.
    pub dest_param: [u16; MAX_MOD_SLOTS],
    slot_dest: [u8; MAX_MOD_SLOTS],
    pub dests: usize,
    live: usize,
}

impl Default for Matrix {
    fn default() -> Self {
        Matrix {
            slots: [Slot::default(); MAX_MOD_SLOTS],
            dest_of: [NONE; COUNT],
            dest_param: [0; MAX_MOD_SLOTS],
            slot_dest: [NONE; MAX_MOD_SLOTS],
            dests: 0,
            live: 0,
        }
    }
}

impl Matrix {
    pub fn set(&mut self, i: usize, slot: Slot) -> bool {
        if i >= MAX_MOD_SLOTS {
            return false;
        }
        self.slots[i] = slot;
        self.rebuild();
        true
    }

    pub fn clear(&mut self) {
        self.slots = [Slot::default(); MAX_MOD_SLOTS];
        self.rebuild();
    }

    pub fn slot(&self, i: usize) -> Slot {
        self.slots[i]
    }

    /// Live (routed, not bypassed) slots.
    pub fn live(&self) -> usize {
        self.live
    }

    /// Does any slot use this source? Lets voices skip computing unused sources.
    pub fn uses(&self, s: u8) -> bool {
        self.slots.iter().any(|sl| sl.live() && (sl.source == s || sl.aux == s))
    }

    fn rebuild(&mut self) {
        for d in 0..self.dests {
            self.dest_of[self.dest_param[d] as usize] = NONE;
        }
        self.dests = 0;
        self.live = 0;
        for i in 0..MAX_MOD_SLOTS {
            let s = self.slots[i];
            self.slot_dest[i] = NONE;
            if !s.live() {
                continue;
            }
            self.live += 1;
            let p = s.dest as usize;
            if self.dest_of[p] == NONE {
                self.dest_of[p] = self.dests as u8;
                self.dest_param[self.dests] = s.dest;
                self.dests += 1;
            }
            self.slot_dest[i] = self.dest_of[p];
        }
    }

    /// Accumulate normalized offsets for one voice into `out[..dests]`.
    pub fn eval(&self, src: &Sources, out: &mut [f32; MAX_MOD_SLOTS]) {
        out[..self.dests].fill(0.0);
        for (i, s) in self.slots.iter().enumerate() {
            let d = self.slot_dest[i];
            if d == NONE {
                continue;
            }
            let raw = src[s.source as usize];
            let v = match (s.flags & BIPOLAR != 0, bipolar_source(s.source)) {
                (true, true) | (false, false) => raw,
                (true, false) => 2.0 * raw - 1.0,
                (false, true) => 0.5 * (raw + 1.0),
            };
            let aux = if s.aux == source::NONE { 1.0 } else { src[s.aux as usize] };
            out[d as usize] += s.amount * v * aux;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::params as p;

    #[test]
    fn slots_share_destinations_and_sum() {
        let mut m = Matrix::default();
        let cut = p::FILTER_CUTOFF[0];
        m.set(0, Slot { source: source::ENV_2, dest: cut, amount: 0.5, ..Slot::default() });
        m.set(3, Slot { source: source::VELOCITY, dest: cut, amount: 0.25, ..Slot::default() });
        m.set(4, Slot { source: source::NOTE, dest: p::OSC_LEVEL[0], amount: 1.0, ..Slot::default() });
        assert_eq!(m.dests, 2);
        assert_eq!(m.live(), 3);
        let mut src = [0.0; source::COUNT];
        src[source::ENV_2 as usize] = 1.0;
        src[source::VELOCITY as usize] = 0.5;
        src[source::NOTE as usize] = 0.0; // middle C: bipolar 0 -> unipolar slot sees 0.5
        let mut out = [0.0; MAX_MOD_SLOTS];
        m.eval(&src, &mut out);
        assert_eq!(out[m.dest_of[cut as usize] as usize], 0.5 + 0.125);
        assert_eq!(out[m.dest_of[p::OSC_LEVEL[0] as usize] as usize], 0.5);
    }

    #[test]
    fn bipolar_and_bypass() {
        let mut m = Matrix::default();
        m.set(0, Slot { source: source::ENV_1, dest: 0, amount: 1.0, flags: BIPOLAR, ..Slot::default() });
        let mut src = [0.0; source::COUNT];
        let mut out = [0.0; MAX_MOD_SLOTS];
        m.eval(&src, &mut out);
        assert_eq!(out[0], -1.0, "a unipolar source at 0 is -1 in a bipolar slot");
        src[source::ENV_1 as usize] = 1.0;
        m.eval(&src, &mut out);
        assert_eq!(out[0], 1.0);
        m.set(0, Slot { flags: BYPASS, ..m.slot(0) });
        assert_eq!(m.dests, 0);
        assert_eq!(m.dest_of[0], NONE);
        m.clear();
        assert_eq!(m.live(), 0);
    }
}
