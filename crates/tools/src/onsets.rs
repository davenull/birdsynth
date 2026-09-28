//! Where the transients in a recording are (for slicing, and for keeping
//! attacks sharp in spectral resynthesis). A transient is a sudden rise in
//! level: the level of short overlapping frames, in dB, is compared with
//! its value a few milliseconds before; rises past a threshold (and above
//! their neighbourhood) are onsets, each then placed where the energy climbs
//! most steeply, to about a millisecond. Working in dB makes quiet hits count
//! like loud ones, and tones like noise.

const N: usize = 512;
const HOP: usize = 64;
/// Hops the rise is measured over.
const SPAN: usize = 3;
/// Rise that makes an onset.
const RISE_DB: f32 = 9.0;

/// Onset positions in frames, in order.
pub fn onsets(x: &[f32], rate: f32) -> Vec<f32> {
    if x.len() < N + SPAN * HOP {
        return Vec::new();
    }
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs())).max(1e-9);
    // levels below about -70 dB of the loudest sample are silence
    let floor = (peak * 3e-4).powi(2);
    let win: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / N as f32).cos()).collect();
    let wsum: f32 = win.iter().map(|w| w * w).sum();
    let frames = (x.len() - N) / HOP + 1;
    let level: Vec<f32> = (0..frames)
        .map(|m| {
            let e: f32 = (0..N).map(|i| (x[m * HOP + i] * win[i]).powi(2)).sum::<f32>() / wsum;
            10.0 * (e + floor).log10()
        })
        .collect();
    let rise: Vec<f32> = (0..frames).map(|m| if m >= SPAN { level[m] - level[m - SPAN] } else { 0.0 }).collect();
    let gap = (0.03 * rate / HOP as f32) as usize; // 30 ms between onsets at least
    let mut picks = Vec::new();
    let mut last: Option<usize> = None;
    for m in SPAN..frames {
        let local = (m.saturating_sub(4)..=(m + 4).min(frames - 1)).all(|j| rise[j] <= rise[m]);
        if rise[m] >= RISE_DB && local && last.is_none_or(|l| m - l >= gap) {
            picks.push(m);
            last = Some(m);
        }
    }
    // the steepest climb in energy (1 ms steps) in the frames the rise was measured over
    let e = ((rate * 0.001) as usize).max(8);
    let energy = |a: usize| -> f32 {
        let a = a.min(x.len());
        let b = (a + e).min(x.len());
        let s: f32 = x[a..b].iter().map(|v| v * v).sum();
        (s / e as f32 + floor).ln()
    };
    picks
        .into_iter()
        .map(|m| {
            let from = ((m - SPAN) * HOP).max(e);
            let to = (m * HOP + N).min(x.len().saturating_sub(2 * e));
            let mut best = from;
            let mut climb = f32::MIN;
            let mut a = from;
            while a < to {
                let r = energy(a) - energy(a - e);
                if r > climb {
                    climb = r;
                    best = a;
                }
                a += e / 8;
            }
            best as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wt_dsp::rng::Rng;

    /// Hits of different kinds and levels at known times, over a quiet noise floor.
    fn sequence(seed: u32, rate: f32) -> (Vec<f32>, Vec<f32>) {
        let mut rng = Rng::new(seed);
        let n = (rate * 20.0) as usize;
        let mut x: Vec<f32> = (0..n).map(|_| (rng.next_f32() * 2.0 - 1.0) * 0.001).collect();
        let mut at = Vec::new();
        let mut t = (0.2 * rate) as usize;
        let mut kind = 0;
        while t < n - (rate as usize) {
            at.push(t as f32);
            let level = 10f32.powf(-(rng.next_f32() * 24.0) / 20.0);
            let hz = 80.0 + rng.next_f32() * 1500.0;
            let gap = (0.08 + rng.next_f32() * 0.35) * rate;
            // each hit has mostly died away (by about 20 dB) before the next: a drum loop, not a wash
            let decay = (gap / rate / 2.3) * (0.3 + 0.7 * rng.next_f32());
            for i in 0..(rate * 0.5) as usize {
                let s = i as f32 / rate;
                let env = (-s / decay).exp() * (1.0 - (-s * 3000.0).exp());
                let v = match kind % 3 {
                    0 => rng.next_f32() * 2.0 - 1.0,                               // a noise burst (snare-ish)
                    1 => (std::f32::consts::TAU * hz * s).sin(),                   // a tone (pluck, tom)
                    _ => (std::f32::consts::TAU * hz * s).sin() * 0.5 + (rng.next_f32() - 0.5), // both (hat over tone)
                };
                x[t + i] += v * env * level;
            }
            kind += 1;
            t += gap as usize;
        }
        (x, at)
    }

    #[test]
    fn finds_ninety_five_percent_of_transients_within_five_ms() {
        let rate = 48_000.0;
        let (mut hit, mut total, mut false_pos) = (0, 0, 0);
        for seed in 1..=6 {
            let (x, truth) = sequence(seed, rate);
            let found = onsets(&x, rate);
            total += truth.len();
            for &t in &truth {
                if found.iter().any(|&f| (f - t).abs() <= 0.005 * rate) {
                    hit += 1;
                }
            }
            false_pos += found.iter().filter(|&&f| !truth.iter().any(|&t| (f - t).abs() <= 0.02 * rate)).count();
        }
        let rate_found = hit as f32 / total as f32;
        eprintln!("onsets: {hit} of {total} within 5 ms, {false_pos} false");
        assert!(rate_found >= 0.95, "{hit} of {total} within 5 ms ({:.1}%)", 100.0 * rate_found);
        assert!(false_pos as f32 <= 0.05 * total as f32, "{false_pos} false onsets for {total}");
    }
}
