//! Parameter metadata and the engine's parameter values.
//!
//! Hosts send normalized values (0..1). Each parameter's curve maps that to
//! its plain unit; the TypeScript side mirrors this mapping exactly (a test
//! compares the two).

use crate::spec::params::{COUNT, INFO};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Lin,
    Exp,
    /// Linear in dB; normalized 0 means silence (-inf dB).
    Db,
    Int,
    Bool,
    Pow(f32),
    Enum(u8),
}

pub const FLAG_MOD: u8 = 1;
pub const FLAG_SMOOTH: u8 = 2;

#[derive(Clone, Copy, Debug)]
pub struct ParamInfo {
    pub key: &'static str,
    pub curve: Curve,
    pub min: f32,
    pub max: f32,
    /// Normalized default.
    pub default: f32,
    pub flags: u8,
}

impl ParamInfo {
    /// Map a normalized value to the parameter's plain unit.
    pub fn to_plain(&self, n: f32) -> f32 {
        let n = if n.is_nan() { self.default } else { n.clamp(0.0, 1.0) };
        let span = self.max - self.min;
        match self.curve {
            Curve::Lin => self.min + span * n,
            Curve::Exp => self.min * (self.max / self.min).powf(n),
            Curve::Pow(k) => self.min + span * n.powf(k),
            Curve::Db => {
                if n <= 0.0 {
                    f32::NEG_INFINITY
                } else {
                    self.min + span * n
                }
            }
            // floor(x + 0.5) rather than round(): it matches JavaScript's Math.round for negatives
            Curve::Int => (self.min + span * n + 0.5).floor(),
            Curve::Bool => {
                if n >= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            Curve::Enum(count) => (n * (count as f32 - 1.0) + 0.5).floor(),
        }
    }
}

/// Current normalized and plain values of every parameter.
pub struct ParamStore {
    norm: [f32; COUNT],
    plain: [f32; COUNT],
}

impl Default for ParamStore {
    fn default() -> Self {
        let mut s = ParamStore { norm: [0.0; COUNT], plain: [0.0; COUNT] };
        for (i, info) in INFO.iter().enumerate() {
            s.norm[i] = info.default;
            s.plain[i] = info.to_plain(info.default);
        }
        s
    }
}

impl ParamStore {
    /// Set a normalized value. Unknown ids are ignored and reported as false.
    #[inline]
    pub fn set(&mut self, id: usize, n: f32) -> bool {
        match INFO.get(id) {
            Some(info) => {
                let n = if n.is_nan() { info.default } else { n.clamp(0.0, 1.0) };
                self.norm[id] = n;
                self.plain[id] = info.to_plain(n);
                true
            }
            None => false,
        }
    }

    #[inline]
    pub fn plain(&self, id: u16) -> f32 {
        self.plain[id as usize]
    }

    #[inline]
    pub fn norm(&self, id: u16) -> f32 {
        self.norm[id as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::params as p;

    #[test]
    fn defaults_round_trip() {
        let s = ParamStore::default();
        assert!((s.plain(p::MASTER_VOLUME) + 6.0).abs() < 1e-4);
        assert_eq!(s.plain(p::VOICE_POLYPHONY), 8.0);
        assert_eq!(s.plain(p::OSC_ENABLE[0]), 1.0);
        assert_eq!(s.plain(p::OSC_ENABLE[1]), 0.0);
        assert!((s.plain(p::ENV_ATTACK[0]) - 0.5).abs() < 1e-3);
        assert!((s.plain(p::ENV_DECAY[0]) - 1000.0).abs() < 0.1);
    }

    #[test]
    fn curves() {
        let db = INFO[p::MASTER_VOLUME as usize];
        assert_eq!(db.to_plain(0.0), f32::NEG_INFINITY);
        assert_eq!(db.to_plain(1.0), 6.0);
        let semi = INFO[p::OSC_SEMI[0] as usize];
        assert_eq!(semi.to_plain(0.0), -12.0);
        assert_eq!(semi.to_plain(0.5), 0.0);
        assert_eq!(semi.to_plain(1.0), 12.0);
        assert_eq!(semi.to_plain(0.75), 6.0);
        // a midpoint between two steps rounds up, like Math.round
        let half = ParamInfo { key: "t", curve: Curve::Int, min: -1.0, max: 0.0, default: 0.0, flags: 0 };
        assert_eq!(half.to_plain(0.5), 0.0);
        let mut s = ParamStore::default();
        assert!(!s.set(COUNT, 0.5));
        assert!(s.set(p::OSC_LEVEL[0] as usize, 2.0));
        assert_eq!(s.plain(p::OSC_LEVEL[0]), 1.0);
    }
}
