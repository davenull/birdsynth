//! Unison: where each of up to 16 copies of an oscillator sits in pitch,
//! stereo and level.

use wt_dsp::math;

pub const MAX_LANES: usize = 16;

pub const MODE_LINEAR: u8 = 0;
pub const MODE_SUPER: u8 = 1;
pub const MODE_EXP: u8 = 2;
pub const MODE_INV: u8 = 3;
pub const MODE_RANDOM: u8 = 4;

/// The classic supersaw's relative detune pattern (seven voices), outer
/// voices at ±1; other voice counts are interpolated from it.
const SUPER: [f32; 7] = [-1.0, -0.571_55, -0.177_45, 0.0, 0.180_98, 0.565_02, 0.976_64];

#[derive(Clone, Copy, Debug)]
pub struct UniParams {
    pub voices: usize,
    pub detune: f32,
    pub blend: f32,
    pub width: f32,
    /// Semitones between the outermost voices at full detune.
    pub range: f32,
    pub mode: u8,
    pub stack: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub n: usize,
    /// Pitch ratio of each lane.
    pub ratio: [f32; MAX_LANES],
    /// Left and right gain of each lane (pan, blend and normalization).
    pub gl: [f32; MAX_LANES],
    pub gr: [f32; MAX_LANES],
    /// Each lane's spread position in -1..1, for WT and warp spread.
    pub spread: [f32; MAX_LANES],
}

impl Default for Layout {
    fn default() -> Self {
        let mut l = Layout { n: 1, ratio: [1.0; MAX_LANES], gl: [0.0; MAX_LANES], gr: [0.0; MAX_LANES], spread: [0.0; MAX_LANES] };
        l.gl[0] = 1.0;
        l.gr[0] = 1.0;
        l
    }
}

/// Even positions in -1..1 for n voices.
fn linear(i: usize, n: usize) -> f32 {
    if n == 1 { 0.0 } else { 2.0 * i as f32 / (n - 1) as f32 - 1.0 }
}

fn super_pos(i: usize, n: usize) -> f32 {
    if n == 1 {
        return 0.0;
    }
    let x = i as f32 / (n - 1) as f32 * 6.0;
    let k = (x as usize).min(5);
    let t = x - k as f32;
    SUPER[k] + (SUPER[k + 1] - SUPER[k]) * t
}

/// Semitone offset a stack mode adds to lane i (in detune order).
fn stack_offset(stack: u8, i: usize, n: usize) -> f32 {
    let centre = i * 2 + 1 == n || (n.is_multiple_of(2) && (i + 1 == n / 2 || i == n / 2));
    match stack {
        1 => if i % 2 == 1 { 12.0 } else { 0.0 },
        2 => if i % 2 == 1 { 24.0 } else { 0.0 },
        3 => [0.0, 12.0, 24.0][i % 3],
        4 => if i % 2 == 1 { 7.0 } else { 0.0 },
        5 => [0.0, 7.0, 12.0][i % 3],
        6 if centre => -12.0,
        7 if centre => -24.0,
        _ => 0.0,
    }
}

/// Lay out the lanes. `random` gives per-note positions for Random mode.
pub fn layout(p: &UniParams, random: &[f32; MAX_LANES]) -> Layout {
    let n = p.voices.clamp(1, MAX_LANES);
    let mut l = Layout { n, ..Layout::default() };
    if n == 1 {
        l.ratio[0] = math::semis_to_ratio(stack_offset(p.stack, 0, 1));
        return l;
    }
    // Pan order is linear left to right; detune positions are interleaved
    // (0, n-1, 1, n-2, ...) so each side of the stereo field gets both high
    // and low voices.
    let half_range = 0.5 * p.range * match p.mode {
        MODE_SUPER => p.detune.powf(1.6),
        _ => p.detune,
    };
    let mut gains = [0.0f32; MAX_LANES];
    let mut power = 0.0f32;
    for i in 0..n {
        let j = if i % 2 == 0 { i / 2 } else { n - 1 - i / 2 };
        let x = match p.mode {
            MODE_SUPER => super_pos(j, n),
            MODE_EXP => {
                let x = linear(j, n);
                x * x.abs()
            }
            MODE_INV => {
                let x = linear(j, n);
                x.signum() * x.abs().sqrt()
            }
            MODE_RANDOM => random[i] * 2.0 - 1.0,
            _ => linear(j, n),
        };
        let centre = j * 2 + 1 == n || (n.is_multiple_of(2) && (j + 1 == n / 2 || j == n / 2));
        let g = if centre { 1.0 - 0.5 * p.blend } else { p.blend };
        gains[i] = g;
        power += g * g;
        l.ratio[i] = math::semis_to_ratio(x * half_range + stack_offset(p.stack, j, n));
        l.spread[i] = x;
        let pan = linear(i, n) * p.width;
        let (pl, pr) = math::balance(pan);
        l.gl[i] = pl;
        l.gr[i] = pr;
    }
    let norm = if power > 0.0 { 1.0 / power.sqrt() } else { 0.0 };
    for i in 0..n {
        l.gl[i] *= gains[i] * norm;
        l.gr[i] *= gains[i] * norm;
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(voices: usize) -> UniParams {
        UniParams { voices, detune: 1.0, blend: 0.75, width: 1.0, range: 2.0, mode: MODE_LINEAR, stack: 0 }
    }

    #[test]
    fn single_voice_is_plain() {
        let l = layout(&params(1), &[0.0; MAX_LANES]);
        assert_eq!((l.n, l.ratio[0], l.gl[0], l.gr[0]), (1, 1.0, 1.0, 1.0));
    }

    #[test]
    fn detune_is_symmetric_and_spans_the_range() {
        for n in 2..=16 {
            let l = layout(&params(n), &[0.0; MAX_LANES]);
            let mut semis: Vec<f32> = (0..n).map(|i| 12.0 * l.ratio[i].log2()).collect();
            semis.sort_by(|a, b| a.partial_cmp(b).unwrap());
            assert!((semis[0] + 1.0).abs() < 1e-4 && (semis[n - 1] - 1.0).abs() < 1e-4, "n {n}: {semis:?}");
            let sum: f32 = semis.iter().sum();
            assert!(sum.abs() < 1e-3, "n {n} detune not centred");
            // constant power whatever the voice count
            let power: f32 = (0..n).map(|i| l.gl[i] * l.gl[i] + l.gr[i] * l.gr[i]).sum();
            assert!(power > 0.5 && power < 2.0, "n {n} power {power}");
        }
    }

    #[test]
    fn stacks_add_octaves() {
        let mut p = params(4);
        p.detune = 0.0;
        p.stack = 1; // "12"
        let l = layout(&p, &[0.0; MAX_LANES]);
        let octs = (0..4).filter(|&i| (l.ratio[i] - 2.0).abs() < 1e-5).count();
        assert_eq!(octs, 2);
    }
}
