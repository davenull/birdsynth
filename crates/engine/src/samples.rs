//! Sample slots: audio the host hands over (the noise library now; the
//! sample-based oscillators later), and the per-voice player for noise.

use wt_dsp::rng::Rng;

use crate::filter::MAX_N;
use crate::tables::AssetBuf;

pub const SLOTS: usize = 8;
pub const NOISE: usize = 0;

pub struct Sample {
    buf: AssetBuf,
    pub frames: usize,
    /// Rate the sample was recorded at.
    pub rate: f32,
}

impl Sample {
    pub fn data(&self) -> &[f32] {
        &self.buf.as_f32()[..self.frames]
    }
}

/// A sample that can't be installed (bad slot, length or rate).
#[derive(Debug, PartialEq, Eq)]
pub struct BadSample;

#[derive(Default)]
pub struct Samples {
    slots: [Option<Sample>; SLOTS],
}

impl Samples {
    /// Install mono f32 audio. On error the asset is freed.
    pub fn load(&mut self, slot: usize, buf: AssetBuf, frames: usize, rate: f32) -> Result<(), BadSample> {
        if slot >= SLOTS || frames < 2 || buf.bytes() < frames * 4 || rate.is_nan() || rate <= 0.0 {
            return Err(BadSample);
        }
        self.slots[slot] = Some(Sample { buf, frames, rate });
        Ok(())
    }

    pub fn get(&self, slot: usize) -> Option<&Sample> {
        self.slots.get(slot).and_then(|s| s.as_ref())
    }
}

/// Plays a sample slot per voice: looped or once, from a (random) start.
#[derive(Clone, Copy, Debug, Default)]
pub struct Player {
    pos: f64,
    done: bool,
}

impl Player {
    pub fn start(&mut self, frames: usize, start: f32, rand: f32, rng: &mut Rng) {
        let f = (start + rand * rng.next_f32()).fract();
        self.pos = f as f64 * frames as f64;
        self.done = false;
    }

    /// Render samples `start..len`, advancing `step` source frames per output sample.
    pub fn render(&mut self, s: Option<&Sample>, step: f64, once: bool, start: usize, len: usize, out: &mut [f32; MAX_N]) {
        let Some(s) = s else {
            out[start..len].fill(0.0);
            return;
        };
        let d = s.data();
        let n = s.frames as f64;
        for v in out.iter_mut().take(len).skip(start) {
            if self.done {
                *v = 0.0;
                continue;
            }
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            let a = d[i % s.frames];
            let b = d[(i + 1) % s.frames];
            *v = a + (b - a) * t;
            self.pos += step;
            if self.pos >= n {
                if once {
                    self.done = true;
                } else {
                    self.pos -= n * (self.pos / n).floor();
                }
            }
        }
    }
}
