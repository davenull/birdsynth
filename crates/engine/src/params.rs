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

/// Current values of every parameter. `set` records a target; parameters
/// flagged `smooth` glide toward it once per sub-block (`step`), everything
/// else jumps. `norm` and `plain` report the current (smoothed) value.
pub struct ParamStore {
    target: [f32; COUNT],
    norm: [f32; COUNT],
    plain: [f32; COUNT],
    /// Parameters still gliding toward their target.
    moving: [u16; COUNT],
    n_moving: usize,
    is_moving: [bool; COUNT],
    coef: f32,
}

impl Default for ParamStore {
    fn default() -> Self {
        let mut s = ParamStore {
            target: [0.0; COUNT],
            norm: [0.0; COUNT],
            plain: [0.0; COUNT],
            moving: [0; COUNT],
            n_moving: 0,
            is_moving: [false; COUNT],
            coef: 1.0,
        };
        for (i, info) in INFO.iter().enumerate() {
            s.target[i] = info.default;
            s.norm[i] = info.default;
            s.plain[i] = info.to_plain(info.default);
        }
        s
    }
}

impl ParamStore {
    /// Smoothing time for `smooth` parameters, given the sample rate and the
    /// sub-block length.
    pub fn set_smoothing(&mut self, sr: f32, block: usize, ms: f32) {
        self.coef = 1.0 - (-(block as f32) / (ms * 0.001 * sr)).exp();
    }

    /// Set a normalized target. Unknown ids are ignored and reported as false.
    #[inline]
    pub fn set(&mut self, id: usize, n: f32) -> bool {
        let Some(info) = INFO.get(id) else { return false };
        let n = if n.is_nan() { info.default } else { n.clamp(0.0, 1.0) };
        self.target[id] = n;
        if info.flags & FLAG_SMOOTH != 0 && self.coef < 1.0 {
            if !self.is_moving[id] && self.norm[id] != n {
                self.is_moving[id] = true;
                self.moving[self.n_moving] = id as u16;
                self.n_moving += 1;
            }
        } else {
            self.norm[id] = n;
            self.plain[id] = info.to_plain(n);
        }
        true
    }

    /// Jump every smoothed parameter to its target (after a reset).
    pub fn snap(&mut self) {
        for i in 0..self.n_moving {
            let id = self.moving[i] as usize;
            self.norm[id] = self.target[id];
            self.plain[id] = INFO[id].to_plain(self.target[id]);
            self.is_moving[id] = false;
        }
        self.n_moving = 0;
    }

    /// Advance smoothing by one sub-block.
    pub fn step(&mut self) {
        let mut i = 0;
        while i < self.n_moving {
            let id = self.moving[i] as usize;
            let t = self.target[id];
            let mut v = self.norm[id] + (t - self.norm[id]) * self.coef;
            let done = (t - v).abs() < 1e-5;
            if done {
                v = t;
            }
            self.norm[id] = v;
            self.plain[id] = INFO[id].to_plain(v);
            if done {
                self.is_moving[id] = false;
                self.n_moving -= 1;
                self.moving[i] = self.moving[self.n_moving];
            } else {
                i += 1;
            }
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

    #[inline]
    pub fn target(&self, id: u16) -> f32 {
        self.target[id as usize]
    }

    /// Plain value with a normalized modulation offset added.
    #[inline]
    pub fn modded(&self, id: u16, offset: f32) -> f32 {
        INFO[id as usize].to_plain(self.norm[id as usize] + offset)
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

    #[test]
    fn smoothed_params_glide_and_settle() {
        let mut s = ParamStore::default();
        s.set_smoothing(48_000.0, 16, 10.0);
        let id = p::OSC_LEVEL[0];
        s.set(id as usize, 0.0);
        assert_eq!(s.norm(id), 0.75, "a smooth param doesn't jump");
        s.step();
        assert!(s.norm(id) < 0.75 && s.norm(id) > 0.0);
        for _ in 0..1000 {
            s.step();
        }
        assert_eq!(s.norm(id), 0.0);
        // un-smoothed params jump
        s.set(p::OSC_SEMI[0] as usize, 1.0);
        assert_eq!(s.plain(p::OSC_SEMI[0]), 12.0);
    }
}
