//! LFOs: drawn shapes, XY paths, chaotic attractors and sample-and-hold,
//! evaluated at control rate (once per 16-frame sub-block).
//!
//! Free-running LFOs synced to the tempo take their phase from the absolute
//! frame count, so they stay locked to the beat however long they run.

use wt_dsp::rng::Rng;

pub const MAX_POINTS: usize = 64;

pub const TYPE_NORMAL: u8 = 0;
pub const TYPE_PATH: u8 = 1;
pub const TYPE_LORENZ: u8 = 2;
pub const TYPE_ROSSLER: u8 = 3;
pub const TYPE_SH: u8 = 4;

pub const MODE_FREE: u8 = 0;
pub const MODE_RETRIG: u8 = 1;
pub const MODE_ENV: u8 = 2;

/// Beats per cycle for each `sync_rate` option ("8 bars" .. "1/64", in 4/4).
pub const SYNC_BEATS: [f64; 10] = [32.0, 16.0, 8.0, 4.0, 2.0, 1.0, 0.5, 0.25, 0.125, 0.0625];
/// Straight, dotted, triplet.
pub const SYNC_MOD: [f64; 3] = [1.0, 1.5, 2.0 / 3.0];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
    /// Bends the segment that starts here: >0 eases in, <0 eases out.
    pub c: f32,
}

/// A drawn curve (x in 0..1 ascending, y in -1..1, wrapping from the last
/// point back to the first) or, for paths, an ordered loop of XY points.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pts: [Point; MAX_POINTS],
    n: usize,
}

#[inline]
fn bend(t: f32, c: f32) -> f32 {
    if c > 0.0 {
        t.powf(1.0 + 7.0 * c)
    } else if c < 0.0 {
        1.0 - (1.0 - t).powf(1.0 - 7.0 * c)
    } else {
        t
    }
}

impl Shape {
    /// Serum-like default: a rounded triangle that reads as a sine.
    pub fn default_curve() -> Shape {
        let mut s = Shape { pts: [Point::default(); MAX_POINTS], n: 0 };
        s.set(&[
            Point { x: 0.0, y: 0.0, c: -0.3 },
            Point { x: 0.25, y: 1.0, c: 0.3 },
            Point { x: 0.5, y: 0.0, c: -0.3 },
            Point { x: 0.75, y: -1.0, c: 0.3 },
        ], false);
        s
    }

    /// Default path: a circle drawn as a rounded square.
    pub fn default_path() -> Shape {
        let mut s = Shape { pts: [Point::default(); MAX_POINTS], n: 0 };
        let mut pts = [Point::default(); 8];
        for (i, p) in pts.iter_mut().enumerate() {
            let a = std::f32::consts::TAU * i as f32 / 8.0;
            *p = Point { x: a.cos(), y: a.sin(), c: 0.0 };
        }
        s.set(&pts, true);
        s
    }

    /// Replace the points (curves sort by x; paths keep their order).
    pub fn set(&mut self, pts: &[Point], path: bool) {
        let n = pts.len().clamp(1, MAX_POINTS);
        for i in 0..n {
            let p = pts[i];
            self.pts[i] = Point {
                x: if path { p.x.clamp(-1.0, 1.0) } else { p.x.clamp(0.0, 1.0) },
                y: p.y.clamp(-1.0, 1.0),
                c: p.c.clamp(-1.0, 1.0),
            };
        }
        self.n = n;
        if !path {
            // insertion sort: n is small and this runs only when the shape changes
            for i in 1..n {
                let mut j = i;
                while j > 0 && self.pts[j - 1].x > self.pts[j].x {
                    self.pts.swap(j - 1, j);
                    j -= 1;
                }
            }
        }
    }

    pub fn points(&self) -> &[Point] {
        &self.pts[..self.n]
    }

    /// Value of a drawn curve at x in [0, 1).
    pub fn eval(&self, x: f32) -> f32 {
        let n = self.n;
        if n == 1 {
            return self.pts[0].y;
        }
        // find the segment [a, b) containing x, wrapping past the last point
        let mut i = n - 1;
        for k in 0..n {
            if self.pts[k].x > x {
                i = if k == 0 { n - 1 } else { k - 1 };
                break;
            }
        }
        let a = self.pts[i];
        let b = self.pts[(i + 1) % n];
        let (ax, mut bx) = (a.x, b.x);
        let mut xx = x;
        if bx <= ax {
            bx += 1.0; // wrap segment
            if xx < ax {
                xx += 1.0;
            }
        }
        let t = if bx > ax { ((xx - ax) / (bx - ax)).clamp(0.0, 1.0) } else { 0.0 };
        a.y + (b.y - a.y) * bend(t, a.c)
    }

    /// Position on a closed path at t in [0, 1): equal time per segment.
    pub fn eval_path(&self, t: f32) -> (f32, f32) {
        let n = self.n;
        let x = t * n as f32;
        let i = (x as usize).min(n - 1);
        let u = bend(x - i as f32, self.pts[i].c);
        let a = self.pts[i];
        let b = self.pts[(i + 1) % n];
        (a.x + (b.x - a.x) * u, a.y + (b.y - a.y) * u)
    }
}

/// An LFO's settings for one sub-block, already resolved.
#[derive(Clone, Copy, Debug)]
pub struct LfoSettings {
    pub kind: u8,
    pub mode: u8,
    /// Cycles per second.
    pub hz: f32,
    /// Beats per cycle when synced (free-running phase then follows the beat).
    pub sync_beats: Option<f64>,
    pub rise: f32,
    pub delay: f32,
    pub smooth: f32,
    /// Start phase, 0..1.
    pub phase0: f32,
    pub reverse: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct LfoState {
    phase: f64,
    /// Seconds since the note (or the last trigger).
    time: f32,
    x: f32,
    y: f32,
    step: i64,
    hold: f32,
    chaos: [f64; 3],
    rng: Rng,
    done: bool,
    primed: bool,
}

impl Default for LfoState {
    fn default() -> Self {
        LfoState { phase: 0.0, time: 0.0, x: 0.0, y: 0.0, step: i64::MIN, hold: 0.0, chaos: [0.1, 0.0, 0.0], rng: Rng::new(0xC0FFEE), done: false, primed: false }
    }
}

impl LfoState {
    /// Restart (note-on in Retrig/Env mode). `seed` varies S&H per voice.
    pub fn trigger(&mut self, s: &LfoSettings, seed: u32) {
        let rng = Rng::new(seed | 1);
        *self = LfoState { phase: s.phase0 as f64, rng, chaos: [0.1 + (seed % 97) as f64 * 1e-3, 0.0, 0.0], ..LfoState::default() };
    }

    /// Where the LFO is in its cycle, 0..1.
    pub fn phase(&self) -> f32 {
        (self.phase - self.phase.floor()) as f32
    }

    /// Advance by `dt` seconds. `beat` is the absolute beat position (for
    /// free-running synced LFOs). Returns the raw (x, y) outputs, -1..1,
    /// before rise and delay.
    pub fn advance(&mut self, s: &LfoSettings, shape: &Shape, path: &Shape, dt: f32, beat: f64) -> (f32, f32) {
        self.time += dt;
        if s.mode == MODE_FREE {
            match s.sync_beats {
                Some(b) => self.phase = beat / b + s.phase0 as f64,
                None => self.phase += s.hz as f64 * dt as f64,
            }
        } else if self.time > s.delay && !self.done {
            self.phase += s.hz as f64 * dt as f64;
            if s.mode == MODE_ENV && self.phase - s.phase0 as f64 >= 1.0 {
                self.phase = s.phase0 as f64 + 0.999_999;
                self.done = true;
            }
        }
        let p = (self.phase - self.phase.floor()) as f32;
        let q = if s.reverse { 1.0 - p } else { p };
        let (x, y) = match s.kind {
            TYPE_PATH => path.eval_path(q.min(0.999_999)),
            TYPE_SH => {
                let step = self.phase.floor() as i64;
                if step != self.step {
                    self.step = step;
                    self.hold = self.rng.next_f32() * 2.0 - 1.0;
                }
                (self.hold, 0.0)
            }
            TYPE_LORENZ | TYPE_ROSSLER => self.chaos(s, dt),
            _ => (shape.eval(q), 0.0),
        };
        // smoothing: a one-pole whose time grows with the Smooth control
        if !self.primed || s.smooth <= 0.0 {
            self.x = x;
            self.y = y;
            self.primed = true;
        } else {
            let tau = s.smooth * s.smooth * 0.5;
            let a = 1.0 - (-dt / tau.max(1e-4)).exp();
            self.x += (x - self.x) * a;
            self.y += (y - self.y) * a;
        }
        (self.x, self.y)
    }

    /// Amplitude from delay and rise (0 during the delay, then a linear rise).
    pub fn amp(&self, s: &LfoSettings) -> f32 {
        if self.time < s.delay {
            0.0
        } else if s.rise > 0.0 {
            ((self.time - s.delay) / s.rise).min(1.0)
        } else {
            1.0
        }
    }

    fn chaos(&mut self, s: &LfoSettings, dt: f32) -> (f32, f32) {
        // scale so 1 Hz is roughly one orbit a second
        let (k, max_step) = if s.kind == TYPE_LORENZ { (0.7, 0.004) } else { (6.0, 0.02) };
        let span = (s.hz as f64 * dt as f64 * k).min(1.0);
        let steps = (span / max_step).ceil().max(1.0) as usize;
        let h = span / steps as f64;
        let [mut x, mut y, mut z] = self.chaos;
        for _ in 0..steps {
            if s.kind == TYPE_LORENZ {
                let (sigma, rho, beta) = (10.0, 28.0, 8.0 / 3.0);
                x += h * sigma * (y - x);
                y += h * (x * (rho - z) - y);
                z += h * (x * y - beta * z);
            } else {
                let (a, b, c) = (0.2, 0.2, 5.7);
                x += h * (-y - z);
                y += h * (x + a * y);
                z += h * (b + z * (x - c));
            }
        }
        if !(x.is_finite() && y.is_finite() && z.is_finite()) {
            (x, y, z) = (0.1, 0.0, 0.0);
        }
        self.chaos = [x, y, z];
        if s.kind == TYPE_LORENZ {
            ((x / 20.0).clamp(-1.0, 1.0) as f32, ((z - 25.0) / 25.0).clamp(-1.0, 1.0) as f32)
        } else {
            ((x / 11.0).clamp(-1.0, 1.0) as f32, (y / 11.0).clamp(-1.0, 1.0) as f32)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(hz: f32) -> LfoSettings {
        LfoSettings { kind: TYPE_NORMAL, mode: MODE_RETRIG, hz, sync_beats: None, rise: 0.0, delay: 0.0, smooth: 0.0, phase0: 0.0, reverse: false }
    }

    #[test]
    fn default_curve_hits_its_points() {
        let s = Shape::default_curve();
        assert!((s.eval(0.0)).abs() < 1e-6);
        assert!((s.eval(0.25) - 1.0).abs() < 1e-6);
        assert!((s.eval(0.75) + 1.0).abs() < 1e-6);
        assert!(s.eval(0.999) < 0.0 && s.eval(0.999) > -0.1);
    }

    #[test]
    fn period_is_accurate() {
        // count cycles of a 3.7 Hz LFO over 60 s at a 16-sample control rate
        let sr = 48_000.0;
        let dt = 16.0 / sr;
        let s = settings(3.7);
        let (shape, path) = (Shape::default_curve(), Shape::default_path());
        let mut st = LfoState::default();
        st.trigger(&s, 1);
        let steps = (60.0 / dt) as usize;
        for _ in 0..steps {
            st.advance(&s, &shape, &path, dt, 0.0);
        }
        let cycles = st.phase + 0.0;
        let want = 3.7 * steps as f64 * dt as f64;
        assert!((cycles / want - 1.0).abs() < 1e-3, "{cycles} vs {want}");
    }

    #[test]
    fn synced_free_lfos_follow_the_beat() {
        let mut s = settings(0.0);
        s.mode = MODE_FREE;
        s.sync_beats = Some(1.0);
        let (shape, path) = (Shape::default_curve(), Shape::default_path());
        let mut st = LfoState::default();
        st.advance(&s, &shape, &path, 0.01, 1234.25);
        assert!((st.phase() - 0.25).abs() < 1e-6);
    }

    #[test]
    fn env_mode_plays_once() {
        let mut s = settings(10.0);
        s.mode = MODE_ENV;
        let (shape, path) = (Shape::default_curve(), Shape::default_path());
        let mut st = LfoState::default();
        st.trigger(&s, 1);
        for _ in 0..1000 {
            st.advance(&s, &shape, &path, 0.001, 0.0);
        }
        assert!(st.done);
        let (x, _) = st.advance(&s, &shape, &path, 0.001, 0.0);
        assert!(x < 0.0 && x > -0.1, "holds near the end of the shape: {x}");
    }

    #[test]
    fn chaos_stays_bounded() {
        for kind in [TYPE_LORENZ, TYPE_ROSSLER] {
            let mut s = settings(50.0);
            s.kind = kind;
            let (shape, path) = (Shape::default_curve(), Shape::default_path());
            let mut st = LfoState::default();
            st.trigger(&s, 3);
            let mut spread = (f32::MAX, f32::MIN);
            for _ in 0..100_000 {
                let (x, _) = st.advance(&s, &shape, &path, 16.0 / 48_000.0, 0.0);
                assert!(x.is_finite() && x.abs() <= 1.0);
                spread = (spread.0.min(x), spread.1.max(x));
            }
            assert!(spread.1 - spread.0 > 0.5, "chaos should wander: {spread:?}");
        }
    }

    #[test]
    fn rise_and_delay() {
        let mut s = settings(1.0);
        s.delay = 0.1;
        s.rise = 0.2;
        let mut st = LfoState::default();
        st.trigger(&s, 1);
        let (shape, path) = (Shape::default_curve(), Shape::default_path());
        st.advance(&s, &shape, &path, 0.05, 0.0);
        assert_eq!(st.amp(&s), 0.0);
        st.advance(&s, &shape, &path, 0.15, 0.0);
        assert!((st.amp(&s) - 0.5).abs() < 1e-5);
        st.advance(&s, &shape, &path, 1.0, 0.0);
        assert_eq!(st.amp(&s), 1.0);
    }
}
