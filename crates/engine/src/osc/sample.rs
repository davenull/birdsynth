//! Recordings the oscillators play (the Sample, Granular and Multisample
//! types), and the sample player.
//!
//! A recording arrives as several copies: full rate, then each half as long
//! (low-passed by the tools before halving), so a note or grain pitched up
//! by k octaves reads copy k and stays free of aliasing. Channels are
//! planar within each copy.
//!
//! The player reads with 4-point Hermite interpolation and handles the loop
//! modes: Off (once), Forward (with a crossfade that blends the end of the
//! loop into the audio before its start), Ping-Pong, Reverse, and Tailed
//! (loops while the key is held, then plays on to the end).

use crate::spec::protocol::SAMPLE_LEVELS;
use crate::tables::AssetBuf;

pub const LOOP_OFF: u8 = 0;
pub const LOOP_FORWARD: u8 = 1;
pub const LOOP_PINGPONG: u8 = 2;
pub const LOOP_REVERSE: u8 = 3;
pub const LOOP_TAILED: u8 = 4;

/// Slices a recording can carry (the rest are ignored).
pub const MAX_SLICES: usize = 128;

/// Floats a recording of `frames` × `channels` with `levels` copies takes (without slices).
pub fn level_floats(frames: usize, channels: usize, levels: usize) -> usize {
    (0..levels).map(|l| level_len(frames, l) * channels).sum()
}

/// Frames in copy `l` (each copy is half the one before, rounded up).
pub fn level_len(frames: usize, l: usize) -> usize {
    frames.div_ceil(1 << l).max(1)
}

pub struct Recording {
    buf: AssetBuf,
    pub frames: usize,
    pub channels: usize,
    pub levels: usize,
    pub rate: f32,
    offset: [usize; SAMPLE_LEVELS],
    slices: usize,
    slice_at: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct BadRecording;

impl Recording {
    /// Check the layout and take the asset. On error the asset is freed.
    pub fn new(buf: AssetBuf, frames: usize, channels: usize, levels: usize, rate: f32, slices: usize) -> Result<Recording, BadRecording> {
        if frames < 4 || !(1..=2).contains(&channels) || !(1..=SAMPLE_LEVELS).contains(&levels) || !(rate > 0.0 && rate < 1e6) {
            return Err(BadRecording);
        }
        let pcm = level_floats(frames, channels, levels);
        if buf.bytes() < (pcm + slices) * 4 {
            return Err(BadRecording);
        }
        let mut offset = [0; SAMPLE_LEVELS];
        let mut at = 0;
        for (l, o) in offset.iter_mut().enumerate().take(levels) {
            *o = at;
            at += level_len(frames, l) * channels;
        }
        Ok(Recording { buf, frames, channels, levels, rate, offset, slices: slices.min(MAX_SLICES), slice_at: pcm })
    }

    /// Channel `ch` (the last one again for a mono recording's right) of copy `l`.
    #[inline]
    pub fn channel(&self, l: usize, ch: usize) -> &[f32] {
        let n = level_len(self.frames, l);
        let c = ch.min(self.channels - 1);
        let o = self.offset[l] + c * n;
        &self.buf.as_f32()[o..o + n]
    }

    /// Slice starts, in frames, in order.
    pub fn slices(&self) -> &[f32] {
        &self.buf.as_f32()[self.slice_at..self.slice_at + self.slices]
    }

    /// The copy to read at `step` source frames per output sample, and the step within it.
    #[inline]
    pub fn level_for(&self, step: f64) -> (usize, f64) {
        let s = step.abs();
        if s <= 1.0 {
            return (0, 1.0);
        }
        let l = (s.log2().floor() as usize).min(self.levels - 1);
        (l, 1.0 / (1u64 << l) as f64)
    }

    /// Read both channels at `pos` (full-rate frames) from copy `l`.
    #[inline]
    pub fn read(&self, l: usize, scale: f64, pos: f64) -> (f32, f32) {
        let q = pos * scale;
        let a = hermite(self.channel(l, 0), q);
        let b = if self.channels > 1 { hermite(self.channel(l, 1), q) } else { a };
        (a, b)
    }

    /// The rising zero crossing nearest `at` (within `reach` frames) in the left channel.
    pub fn zero_near(&self, at: f64, reach: usize) -> f64 {
        let x = self.channel(0, 0);
        let c = at.round() as isize;
        let mut best: Option<f64> = None;
        let lo = (c - reach as isize).max(1);
        let hi = (c + reach as isize).min(x.len() as isize - 1);
        let mut i = lo;
        while i <= hi {
            let (p, q) = (x[i as usize - 1], x[i as usize]);
            if p < 0.0 && q >= 0.0 {
                let z = (i - 1) as f64 + (-p / (q - p)) as f64;
                if best.is_none_or(|b| (z - at).abs() < (b - at).abs()) {
                    best = Some(z);
                }
            }
            i += 1;
        }
        best.unwrap_or(at)
    }
}

/// Audio a read head can play: a recording (with its halved copies) or one multisample zone.
pub trait Audio {
    /// Both channels at `pos` (frames), read the way `step` frames per sample calls for.
    fn read_at(&self, step: f64, pos: f64) -> (f32, f32);
}

impl Audio for Recording {
    #[inline]
    fn read_at(&self, step: f64, pos: f64) -> (f32, f32) {
        let (l, scale) = self.level_for(step);
        self.read(l, scale, pos)
    }
}

/// 4-point Hermite interpolation of `x` at `q` (zero outside the recording).
#[inline(always)]
pub fn hermite(x: &[f32], q: f64) -> f32 {
    let i = q.floor();
    let t = (q - i) as f32;
    let i = i as isize;
    let n = x.len() as isize;
    let at = |k: isize| if k >= 0 && k < n { x[k as usize] } else { 0.0 };
    let (xm, x0, x1, x2) = (at(i - 1), at(i), at(i + 1), at(i + 2));
    let c1 = 0.5 * (x1 - xm);
    let c2 = xm - 2.5 * x0 + 2.0 * x1 - 0.5 * x2;
    let c3 = 0.5 * (x2 - xm) + 1.5 * (x0 - x1);
    ((c3 * t + c2) * t + c1) * t + x0
}

/// Where a note plays in the recording, resolved at its start (and when the settings move).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Region {
    pub start: f64,
    pub end: f64,
    pub loop_start: f64,
    pub loop_end: f64,
    pub mode: u8,
    /// Crossfade length in frames (Forward and Tailed).
    pub xfade: f64,
}

impl Region {
    /// From the settings (shares of the recording), snapped to zero crossings if asked.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(rec: &Recording, start: f32, end: f32, ls: f32, le: f32, mode: u8, xfade: f32, snap: bool) -> Region {
        let n = rec.frames as f64;
        let s = (start.clamp(0.0, 1.0) as f64 * n).min(n - 2.0);
        let e = (end.clamp(0.0, 1.0) as f64 * n).max(s + 2.0).min(n);
        let mut a = (ls.clamp(0.0, 1.0) as f64 * n).clamp(s, e - 2.0);
        let mut b = (le.clamp(0.0, 1.0) as f64 * n).clamp(a + 2.0, e);
        if snap && mode != LOOP_OFF {
            let reach = ((rec.rate as f64) * 0.005) as usize; // within 5 ms
            let a2 = rec.zero_near(a, reach);
            let b2 = rec.zero_near(b, reach);
            if b2 - a2 >= 2.0 && a2 >= s && b2 <= e {
                a = a2;
                b = b2;
            }
        }
        // the crossfade reads audio before the loop start, so it can't be longer than what's there
        let x = (xfade.clamp(0.0, 0.5) as f64 * (b - a)).min(a - s).max(0.0);
        Region { start: s, end: e, loop_start: a, loop_end: b, mode, xfade: x }
    }
}

/// One read head (one unison lane of a sample voice, or a multisample zone).
#[derive(Clone, Copy, Debug, Default)]
pub struct Head {
    pub pos: f64,
    /// +1 forwards, -1 backwards (ping-pong and reverse).
    pub dir: f64,
    pub done: bool,
}

impl Head {
    pub fn start(&mut self, r: &Region) {
        self.pos = if r.mode == LOOP_REVERSE { r.loop_end - 1e-6 } else { r.start };
        self.dir = if r.mode == LOOP_REVERSE { -1.0 } else { 1.0 };
        self.done = false;
    }

    /// One output sample: read, then advance by `step` frames. `held`: the key is still down (Tailed).
    #[inline]
    pub fn tick(&mut self, rec: &impl Audio, r: &Region, step: f64, held: bool) -> (f32, f32) {
        if self.done {
            return (0.0, 0.0);
        }
        let (mut a, mut b) = rec.read_at(step, self.pos);
        let looping = match r.mode {
            LOOP_FORWARD => true,
            LOOP_TAILED => held,
            _ => false,
        };
        // forward loops: in the last `xfade` frames, blend toward the audio just before the loop start
        if looping && r.xfade > 0.0 && self.pos > r.loop_end - r.xfade && self.pos < r.loop_end {
            let g = ((self.pos - (r.loop_end - r.xfade)) / r.xfade) as f32;
            let (c, d) = rec.read_at(step, self.pos - (r.loop_end - r.loop_start));
            a += (c - a) * g;
            b += (d - b) * g;
        }
        self.pos += step * self.dir;
        let len = r.loop_end - r.loop_start;
        match r.mode {
            LOOP_FORWARD | LOOP_TAILED if looping => {
                if self.pos >= r.loop_end {
                    self.pos -= len * ((self.pos - r.loop_end) / len).floor().max(0.0) + len;
                }
            }
            LOOP_PINGPONG => {
                if self.pos >= r.loop_end {
                    self.pos = 2.0 * r.loop_end - self.pos;
                    self.dir = -1.0;
                } else if self.pos < r.loop_start && self.dir < 0.0 {
                    self.pos = 2.0 * r.loop_start - self.pos;
                    self.dir = 1.0;
                }
                self.pos = self.pos.clamp(r.loop_start, r.loop_end);
            }
            LOOP_REVERSE if self.pos < r.loop_start => {
                self.pos += len * (((r.loop_start - self.pos) / len).floor() + 1.0);
            }
            _ => {}
        }
        if self.pos >= r.end || self.pos < 0.0 {
            self.done = true;
        }
        (a, b)
    }
}

#[cfg(test)]
pub mod tests_support {
    use super::*;

    /// A recording from mono samples (one copy, no slices).
    pub fn rec(x: &[f32], rate: f32) -> Recording {
        let mut buf = AssetBuf::alloc(x.len() * 4).unwrap();
        buf.as_f32_mut().copy_from_slice(x);
        Recording::new(buf, x.len(), 1, 1, rate, 0).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::tests_support::rec;
    use super::*;

    #[test]
    fn hermite_is_exact_on_samples_and_smooth_between() {
        let x: Vec<f32> = (0..64).map(|i| (i as f32 * 0.2).sin()).collect();
        for i in 1..60 {
            assert_eq!(hermite(&x, i as f64), x[i]);
            let mid = hermite(&x, i as f64 + 0.5);
            let want = ((i as f32 + 0.5) * 0.2).sin();
            assert!((mid - want).abs() < 2e-4, "{mid} vs {want}");
        }
    }

    #[test]
    fn loops_and_crossfades_without_clicks() {
        // a sine whose loop points sit off its zero crossings: without a crossfade the jump clicks
        let n = 48_000;
        let x: Vec<f32> = (0..n).map(|i| (2.0 * std::f32::consts::PI * 220.0 * i as f32 / 48_000.0).sin()).collect();
        let r = rec(&x, 48_000.0);
        let jump = |xfade: f32| {
            let reg = Region::resolve(&r, 0.0, 1.0, 0.2037, 0.6011, LOOP_FORWARD, xfade, false);
            let mut h = Head::default();
            h.start(&reg);
            let mut prev = h.tick(&r, &reg, 1.0, true).0;
            let mut worst = 0.0f32;
            for _ in 0..3 * n {
                let v = h.tick(&r, &reg, 1.0, true).0;
                worst = worst.max((v - prev).abs());
                prev = v;
            }
            assert!(!h.done);
            worst
        };
        let natural = 2.0 * std::f32::consts::PI * 220.0 / 48_000.0; // a sine's biggest step
        assert!(jump(0.0) > 10.0 * natural, "an unfaded loop should click here");
        assert!(jump(0.05) < 1.5 * natural, "crossfaded: {}", jump(0.05));
    }

    #[test]
    fn modes_play_where_they_should() {
        let x: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        let r = rec(&x, 1000.0);
        // once: plays to the end and stops
        let reg = Region::resolve(&r, 0.1, 0.5, 0.0, 1.0, LOOP_OFF, 0.0, false);
        let mut h = Head::default();
        h.start(&reg);
        assert_eq!(h.tick(&r, &reg, 1.0, true).0, 100.0);
        for _ in 0..400 {
            h.tick(&r, &reg, 1.0, true);
        }
        assert!(h.done);
        // ping-pong turns at both ends
        let reg = Region::resolve(&r, 0.0, 1.0, 0.2, 0.3, LOOP_PINGPONG, 0.0, false);
        let mut h = Head::default();
        h.start(&reg);
        let v: Vec<f32> = (0..700).map(|_| h.tick(&r, &reg, 1.0, true).0).collect();
        assert!(v[250..].iter().all(|&s| (199.0..=301.0).contains(&s)), "stays in the loop");
        assert!(v.windows(2).skip(300).any(|w| w[1] < w[0]) && v.windows(2).skip(300).any(|w| w[1] > w[0]));
        // reverse loops backwards
        let reg = Region::resolve(&r, 0.0, 1.0, 0.5, 0.6, LOOP_REVERSE, 0.0, false);
        let mut h = Head::default();
        h.start(&reg);
        let v: Vec<f32> = (0..300).map(|_| h.tick(&r, &reg, 1.0, true).0).collect();
        assert!(v.windows(2).filter(|w| w[1] < w[0]).count() > 280);
        assert!(v.iter().all(|&s| (499.0..=600.0).contains(&s)));
        // tailed: loops while held, then runs to the end
        let reg = Region::resolve(&r, 0.0, 1.0, 0.2, 0.3, LOOP_TAILED, 0.0, false);
        let mut h = Head::default();
        h.start(&reg);
        for _ in 0..500 {
            h.tick(&r, &reg, 1.0, true);
        }
        assert!(h.pos < 300.0);
        let mut top = 0.0f32;
        while !h.done {
            top = top.max(h.tick(&r, &reg, 1.0, false).0);
        }
        assert!(top > 990.0, "played the tail: reached {top}");
    }

    #[test]
    fn snap_moves_loop_points_to_zero_crossings() {
        let x: Vec<f32> = (0..4800).map(|i| (2.0 * std::f32::consts::PI * 100.0 * i as f32 / 4800.0).sin()).collect();
        let r = rec(&x, 48_000.0);
        let reg = Region::resolve(&r, 0.0, 1.0, 0.1013, 0.5021, LOOP_FORWARD, 0.0, true);
        // rising zero crossings every 48 samples
        assert!((reg.loop_start / 48.0 - (reg.loop_start / 48.0).round()).abs() < 1e-3, "{}", reg.loop_start);
        assert!((reg.loop_end / 48.0 - (reg.loop_end / 48.0).round()).abs() < 1e-3, "{}", reg.loop_end);
    }

    #[test]
    fn picks_a_copy_for_the_step() {
        let mut buf = AssetBuf::alloc(level_floats(1000, 2, 4) * 4).unwrap();
        buf.as_f32_mut().fill(0.25);
        let r = Recording::new(buf, 1000, 2, 4, 48_000.0, 0).unwrap();
        assert_eq!(r.level_for(1.0), (0, 1.0));
        assert_eq!(r.level_for(2.5), (1, 0.5));
        assert_eq!(r.level_for(-5.0), (2, 0.25));
        assert_eq!(r.level_for(100.0), (3, 0.125));
        assert_eq!(r.channel(3, 1).len(), 125);
        assert!(Recording::new(AssetBuf::alloc(16).unwrap(), 1000, 2, 4, 48_000.0, 0).is_err());
    }
}
