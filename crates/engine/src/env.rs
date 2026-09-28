//! AHDSR envelope, evaluated at control rate.
//!
//! `advance(n)` moves the envelope n samples forward and returns the level
//! at the end; the voice ramps linearly between those control points.

use crate::spec::params as p;
use crate::params::ParamStore;

/// Shape of the decay and release segments: level follows (1 - x)^k, which
/// falls fast and then eases in, like an analogue envelope. (P2 exposes curves.)
const DECAY_CURVE: f32 = 3.0;
const RELEASE_CURVE: f32 = 3.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Stage {
    #[default]
    Idle,
    Attack,
    Hold,
    Decay,
    Sustain,
    Release,
}

/// Segment times in samples, and the sustain level.
#[derive(Clone, Copy, Debug)]
pub struct EnvTimes {
    pub attack: f32,
    pub hold: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl EnvTimes {
    /// Read envelope `index` (0 = Env 1) from the parameters.
    pub fn from_params(params: &ParamStore, index: usize, sr: f32) -> Self {
        let ms = sr * 0.001;
        EnvTimes {
            attack: params.plain(p::ENV_ATTACK[index]) * ms,
            hold: params.plain(p::ENV_HOLD[index]) * ms,
            decay: params.plain(p::ENV_DECAY[index]) * ms,
            sustain: params.plain(p::ENV_SUSTAIN[index]),
            release: params.plain(p::ENV_RELEASE[index]) * ms,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Env {
    pub stage: Stage,
    /// Samples spent in the current stage.
    t: f32,
    pub level: f32,
    /// Level when the current attack or release began.
    from: f32,
}

impl Env {
    /// Start the attack from the current level (0 for a fresh voice).
    pub fn trigger(&mut self) {
        self.stage = Stage::Attack;
        self.t = 0.0;
        self.from = self.level;
    }

    pub fn release(&mut self) {
        if !matches!(self.stage, Stage::Idle | Stage::Release) {
            self.stage = Stage::Release;
            self.t = 0.0;
            self.from = self.level;
        }
    }

    /// Advance `n` samples and return the level reached.
    pub fn advance(&mut self, mut n: f32, e: &EnvTimes) -> f32 {
        loop {
            match self.stage {
                Stage::Idle => {
                    self.level = 0.0;
                    return 0.0;
                }
                Stage::Attack => {
                    if self.t + n < e.attack {
                        self.t += n;
                        self.level = self.from + (1.0 - self.from) * (self.t / e.attack);
                        return self.level;
                    }
                    n -= (e.attack - self.t).max(0.0);
                    self.stage = Stage::Hold;
                    self.t = 0.0;
                    self.level = 1.0;
                }
                Stage::Hold => {
                    if self.t + n < e.hold {
                        self.t += n;
                        return 1.0;
                    }
                    n -= (e.hold - self.t).max(0.0);
                    self.stage = Stage::Decay;
                    self.t = 0.0;
                }
                Stage::Decay => {
                    if self.t + n < e.decay {
                        self.t += n;
                        let x = 1.0 - self.t / e.decay;
                        self.level = e.sustain + (1.0 - e.sustain) * x.powf(DECAY_CURVE);
                        return self.level;
                    }
                    n -= (e.decay - self.t).max(0.0);
                    self.stage = Stage::Sustain;
                    self.t = 0.0;
                }
                Stage::Sustain => {
                    self.level = e.sustain;
                    return self.level;
                }
                Stage::Release => {
                    if self.t + n < e.release {
                        self.t += n;
                        let x = 1.0 - self.t / e.release;
                        self.level = self.from * x.powf(RELEASE_CURVE);
                        return self.level;
                    }
                    self.stage = Stage::Idle;
                    self.t = 0.0;
                    self.level = 0.0;
                    return 0.0;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn times(a: f32, h: f32, d: f32, s: f32, r: f32) -> EnvTimes {
        EnvTimes { attack: a, hold: h, decay: d, sustain: s, release: r }
    }

    #[test]
    fn attack_reaches_full_on_time() {
        let e = times(480.0, 0.0, 480.0, 0.5, 480.0);
        let mut env = Env::default();
        env.trigger();
        let mut t = 0;
        while env.stage == Stage::Attack {
            env.advance(16.0, &e);
            t += 16;
        }
        assert_eq!(t, 480);
        assert_eq!(env.level, 1.0);
    }

    #[test]
    fn decays_to_sustain_then_releases_to_idle() {
        let e = times(0.0, 32.0, 320.0, 0.25, 160.0);
        let mut env = Env::default();
        env.trigger();
        assert_eq!(env.advance(16.0, &e), 1.0); // instant attack, holding
        for _ in 0..40 {
            env.advance(16.0, &e);
        }
        assert_eq!(env.stage, Stage::Sustain);
        assert_eq!(env.level, 0.25);
        env.release();
        let mut t = 0;
        while env.stage != Stage::Idle {
            let before = env.level;
            let after = env.advance(16.0, &e);
            assert!(after <= before);
            t += 16;
        }
        assert_eq!(t, 160);
    }

    #[test]
    fn release_during_attack_starts_from_the_current_level() {
        let e = times(1000.0, 0.0, 0.0, 1.0, 100.0);
        let mut env = Env::default();
        env.trigger();
        env.advance(500.0, &e);
        assert!((env.level - 0.5).abs() < 1e-6);
        env.release();
        let l = env.advance(1.0, &e);
        assert!(l < 0.5 && l > 0.45, "{l}");
    }
}
