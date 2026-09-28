//! A recording packed for the engine's Sample and Granular types: the audio
//! at full rate, then five more copies each half as long (low-passed with a
//! halfband before halving, centred so every copy stays in time), then the
//! slice points. See crates/engine/src/osc/sample.rs for the reader.

use wt_dsp::oversample::design_taps;

pub const LEVELS: usize = 6;

/// Frames in copy `l` (the engine's `level_len`).
pub fn level_len(frames: usize, l: usize) -> usize {
    frames.div_ceil(1 << l).max(1)
}

/// Floats the packed recording takes.
pub fn floats(frames: usize, channels: usize, slices: usize) -> usize {
    (0..LEVELS).map(|l| level_len(frames, l) * channels).sum::<usize>() + slices
}

/// One halving: a centred halfband low-pass, then every other sample.
fn halve(x: &[f32], h: &[f32]) -> Vec<f32> {
    let c = (h.len() / 2) as isize;
    let n = x.len().div_ceil(2);
    (0..n)
        .map(|i| {
            let mid = 2 * i as isize;
            let mut s = 0.0f32;
            for (k, &hk) in h.iter().enumerate() {
                if hk == 0.0 {
                    continue;
                }
                let j = mid + c - k as isize;
                if j >= 0 && (j as usize) < x.len() {
                    s += hk * x[j as usize];
                }
            }
            s
        })
        .collect()
}

/// Pack `channels` (1 or 2, equal lengths) and `slices` (frame positions) into `out`.
pub fn prepare(channels: &[&[f32]], slices: &[f32], out: &mut [f32]) {
    let frames = channels[0].len();
    let h = design_taps(63, 8.0);
    let mut at = 0;
    let mut cur: Vec<Vec<f32>> = channels.iter().map(|c| c.to_vec()).collect();
    for l in 0..LEVELS {
        let n = level_len(frames, l);
        for c in &cur {
            out[at..at + n].copy_from_slice(&c[..n]);
            at += n;
        }
        if l + 1 < LEVELS {
            cur = cur.iter().map(|c| {
                let mut v = halve(c, &h);
                v.resize(level_len(frames, l + 1), 0.0);
                v
            }).collect();
        }
    }
    out[at..at + slices.len()].copy_from_slice(slices);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_stay_in_time_and_lose_only_what_they_must() {
        let n = 4800;
        // a low tone that every copy keeps, and a high one the halved copies drop
        let x: Vec<f32> = (0..n).map(|i| (i as f32 * 0.01).sin() + 0.5 * (i as f32 * 2.5).sin()).collect();
        let mut out = vec![0.0; floats(n, 1, 2)];
        prepare(&[&x], &[10.0, 20.0], &mut out);
        let l1 = &out[n..n + level_len(n, 1)];
        // copy 1 at sample i is the recording at 2i: the low tone, with the high one gone
        for i in (100..2300).step_by(97) {
            let want = (2.0 * i as f32 * 0.01).sin();
            assert!((l1[i] - want).abs() < 0.01, "{i}: {} vs {want}", l1[i]);
        }
        assert_eq!(&out[out.len() - 2..], &[10.0, 20.0]);
    }
}
