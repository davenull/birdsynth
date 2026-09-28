//! One voice: a note's oscillator and amp envelope. (P0: oscillator A only.)

use wt_dsp::{math, mip, phase};

use crate::env::{Env, EnvTimes, Stage};
use crate::spec::protocol::SUB_BLOCK;

const N: usize = SUB_BLOCK;

/// Per-sub-block values shared by every voice.
pub struct VoiceCtx<'a> {
    pub sr: f32,
    /// The oscillator's single-frame block (see `mip`).
    pub table: &'a [f32],
    pub env: EnvTimes,
    pub osc_on: bool,
    /// Pitch offset of the oscillator in semitones (octave, semi, fine, coarse).
    pub pitch: f32,
    /// Oscillator level at the start and end of the sub-block (smoothed).
    pub level: (f32, f32),
    pub pan: (f32, f32),
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Voice {
    pub active: bool,
    pub note: u8,
    pub channel: u8,
    pub note_id: u32,
    #[allow(dead_code)] // read by the modulation matrix (P1)
    pub velocity: f32,
    /// Start order; higher is newer.
    pub age: u64,
    pub released: bool,
    pub env: Env,
    phase: u32,
    /// Amp at the end of the previous sub-block; this sub-block ramps from it.
    amp: f32,
    /// Where the note starts inside its first sub-block.
    start: usize,
    /// Fade-out gain while being stolen, and how much it drops per sample.
    kill_gain: f32,
    kill_step: f32,
}

impl Voice {
    pub fn start(&mut self, note: u8, channel: u8, velocity: f32, note_id: u32, offset: usize, age: u64) {
        *self = Voice {
            active: true,
            note,
            channel,
            note_id,
            velocity,
            age,
            kill_gain: 1.0,
            start: offset.min(N - 1),
            ..Voice::default()
        };
        self.env.trigger();
    }

    pub fn release(&mut self) {
        self.released = true;
        self.env.release();
    }

    /// Fade out over `samples` and then free the slot (voice stealing, all-sound-off).
    pub fn kill(&mut self, samples: f32) {
        if self.kill_step == 0.0 {
            self.kill_step = 1.0 / samples.max(1.0);
        }
    }

    #[inline]
    pub fn killing(&self) -> bool {
        self.kill_step > 0.0
    }

    /// Render one sub-block into `mix`. `tap_osc` and `tap_out` receive the
    /// oscillator signal before and after the amp.
    pub fn render(
        &mut self,
        cx: &VoiceCtx,
        mix: &mut [[f32; N]; 2],
        tap_osc: &mut [f32; N],
        tap_out: &mut [f32; N],
    ) {
        let start = self.start;
        self.start = 0;
        let n = N - start;
        tap_osc[..start].fill(0.0);
        tap_out[..start].fill(0.0);

        let env_end = self.env.advance(n as f32, &cx.env);
        let kill_end = if self.killing() {
            let k = self.kill_gain - self.kill_step * n as f32;
            if k < 1e-4 { 0.0 } else { k }
        } else {
            1.0
        };
        let amp0 = self.amp * if self.killing() { self.kill_gain } else { 1.0 };
        let amp1 = env_end * kill_end;

        let hz = math::note_to_hz(self.note as f64 + cx.pitch as f64);
        let inc = phase::inc(hz, cx.sr as f64);
        let pick = mip::pick(phase::cycles(inc), cx.sr);
        let (gl, gr) = cx.pan;
        let (lv0, lv1) = cx.level;
        let damp = (amp1 - amp0) / n as f32;
        let dlv = (lv1 - lv0) / N as f32;

        let mut ph = self.phase;
        let mut amp = amp0;
        for i in start..N {
            amp += damp;
            let o = if cx.osc_on { mip::read_pick(cx.table, pick, ph) * (lv0 + dlv * (i + 1) as f32) } else { 0.0 };
            ph = ph.wrapping_add(inc);
            let y = o * amp;
            mix[0][i] += y * gl;
            mix[1][i] += y * gr;
            tap_osc[i] = o;
            tap_out[i] = y;
        }
        self.phase = ph;

        // Keep the ramp's end point without the kill fade folded in.
        self.amp = env_end;
        if self.killing() {
            self.kill_gain = kill_end;
        }
        if kill_end <= 0.0 || (self.env.stage == Stage::Idle && amp1 == 0.0) {
            self.active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wt_dsp::saw;

    fn ctx(table: &[f32]) -> VoiceCtx<'_> {
        VoiceCtx {
            sr: 48_000.0,
            table,
            env: EnvTimes { attack: 0.0, hold: 0.0, decay: 0.0, sustain: 1.0, release: 480.0 },
            osc_on: true,
            pitch: 0.0,
            level: (1.0, 1.0),
            pan: (1.0, 1.0),
        }
    }

    #[test]
    fn starts_silent_before_its_offset() {
        let table = saw::saw_frame();
        let cx = ctx(&table);
        let mut v = Voice::default();
        v.start(69, 0, 1.0, 1, 5, 1);
        let mut mix = [[0.0; N]; 2];
        let (mut a, mut b) = ([0.0; N], [0.0; N]);
        v.render(&cx, &mut mix, &mut a, &mut b);
        assert!(mix[0][..5].iter().all(|&x| x == 0.0));
        assert!(mix[0][5..].iter().any(|&x| x != 0.0));
    }

    #[test]
    fn kill_frees_the_slot_quickly() {
        let table = saw::saw_frame();
        let cx = ctx(&table);
        let mut v = Voice::default();
        v.start(60, 0, 1.0, 1, 0, 1);
        let mut mix = [[0.0; N]; 2];
        let (mut a, mut b) = ([0.0; N], [0.0; N]);
        v.render(&cx, &mut mix, &mut a, &mut b);
        v.kill(144.0);
        let mut blocks = 0;
        while v.active {
            v.render(&cx, &mut mix, &mut a, &mut b);
            blocks += 1;
            assert!(blocks < 20);
        }
        assert_eq!(blocks, 9);
    }
}
