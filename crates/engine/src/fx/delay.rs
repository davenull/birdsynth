//! Delay: echoes with a band-pass in the feedback loop. Normal (left and
//! right separately), Ping-Pong (echoes bounce between the sides) and
//! Tap->Delay (one early tap at the left time, then echoes repeating at the
//! right time). Delay times glide toward their targets, so moving them
//! bends the pitch like tape instead of clicking.

use super::Ctx;
use super::util::{Block, DelayLine, Lp1, N, NOTE_BEATS, onepole_coef};
use crate::spec::params as p;

/// Longest echo, in seconds.
pub const MAX_SECONDS: f32 = 4.0;
/// Time constant of the delay-time glide.
const GLIDE_MS: f32 = 40.0;

pub struct Delay {
    line: [DelayLine; 2],
    lp: [Lp1; 2],
    hp: [Lp1; 2],
    time: [f32; 2],
    primed: bool,
}

pub const NORMAL: u8 = 0;
pub const PING_PONG: u8 = 1;
pub const TAP: u8 = 2;

impl Delay {
    pub fn new(sr: f32) -> Delay {
        let len = (sr * MAX_SECONDS) as usize + 8;
        Delay { line: [DelayLine::new(len), DelayLine::new(len)], lp: [Lp1::default(); 2], hp: [Lp1::default(); 2], time: [0.0; 2], primed: false }
    }

    pub fn reset(&mut self) {
        self.line.iter_mut().for_each(DelayLine::clear);
        self.lp = [Lp1::default(); 2];
        self.hp = [Lp1::default(); 2];
        self.primed = false;
    }

    /// Target delay times in samples.
    pub fn times(cx: &Ctx, i: usize) -> [f32; 2] {
        let ms = if cx.p(p::FX_DELAY_BPM[i]) >= 0.5 {
            let beat_ms = 60_000.0 / cx.bpm.max(1.0) as f64;
            let b = |id: u16| (NOTE_BEATS[(cx.p(id) as usize).min(NOTE_BEATS.len() - 1)] * beat_ms) as f32;
            [b(p::FX_DELAY_SYNC_L[i]), b(p::FX_DELAY_SYNC_R[i])]
        } else {
            [cx.p(p::FX_DELAY_TIME_L[i]), cx.p(p::FX_DELAY_TIME_R[i])]
        };
        let link = cx.p(p::FX_DELAY_LINK[i]) >= 0.5;
        let ms = if link { [ms[0], ms[0]] } else { ms };
        let max = cx.sr * MAX_SECONDS - 4.0;
        ms.map(|t| (t * cx.sr / 1000.0).clamp(2.0, max))
    }

    pub fn process(&mut self, cx: &Ctx, i: usize, buf: &mut Block) {
        let mode = cx.p(p::FX_DELAY_MODE[i]) as u8;
        let target = Delay::times(cx, i);
        if !self.primed {
            self.time = target;
            self.primed = true;
        }
        let fb = cx.p(p::FX_DELAY_FEEDBACK[i]).clamp(0.0, 1.0);
        let center = cx.p(p::FX_DELAY_FREQ[i]);
        let r = (0.25 + 4.75 * cx.p(p::FX_DELAY_WIDTH[i])).exp2();
        let a_lp = onepole_coef((center * r).min(cx.sr * 0.45), cx.sr);
        let a_hp = onepole_coef(center / r, cx.sr);
        let hq = cx.p(p::FX_DELAY_HQ[i]) >= 0.5;
        let mix = cx.p(p::FX_DELAY_MIX[i]);
        let glide = 1.0 - (-1.0 / (GLIDE_MS * 0.001 * cx.sr)).exp();
        for n in 0..N {
            for ch in 0..2 {
                self.time[ch] += (target[ch] - self.time[ch]) * glide;
            }
            // the line is read before this sample is written, so reading d - 1 back delays by exactly d
            let read = |line: &DelayLine, d: f32| if hq { line.read_cubic(d - 1.0) } else { line.read(d - 1.0) };
            let (x0, x1) = (buf[0][n], buf[1][n]);
            let (w0, w1, y0, y1) = match mode {
                PING_PONG => {
                    let y0 = read(&self.line[0], self.time[0]);
                    let y1 = read(&self.line[1], self.time[0]);
                    let f1 = self.band(1, y1, a_lp, a_hp);
                    let f0 = self.band(0, y0, a_lp, a_hp);
                    (0.5 * (x0 + x1) + fb * f1, f0, y0, y1)
                }
                TAP => {
                    // the tap line holds the input; the repeat line echoes the tap
                    let tap = read(&self.line[0], self.time[0]);
                    let rep = read(&self.line[1], self.time[1]);
                    let f = self.band(1, rep, a_lp, a_hp);
                    let y = tap + rep;
                    (0.5 * (x0 + x1), tap + fb * f, y, y)
                }
                _ => {
                    let y0 = read(&self.line[0], self.time[0]);
                    let y1 = read(&self.line[1], self.time[1]);
                    let f0 = self.band(0, y0, a_lp, a_hp);
                    let f1 = self.band(1, y1, a_lp, a_hp);
                    (x0 + fb * f0, x1 + fb * f1, y0, y1)
                }
            };
            self.line[0].write(w0);
            self.line[1].write(w1);
            buf[0][n] = x0 + (y0 - x0) * mix;
            buf[1][n] = x1 + (y1 - x1) * mix;
        }
        for f in self.lp.iter_mut().chain(self.hp.iter_mut()) {
            f.flush();
        }
    }

    /// The feedback band-pass: a one-pole high-pass then low-pass around the centre.
    #[inline(always)]
    fn band(&mut self, ch: usize, x: f32, a_lp: f32, a_hp: f32) -> f32 {
        let h = x - self.hp[ch].tick(x, a_hp);
        self.lp[ch].tick(h, a_lp)
    }

    pub fn has_subnormal(&self) -> bool {
        self.line.iter().any(DelayLine::has_subnormal)
    }
}
