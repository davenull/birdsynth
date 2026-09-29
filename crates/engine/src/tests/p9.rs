//! Linked instances: the transport follows a shared timeline (SetTimeline),
//! on its exact frame, joins mid-way in step, and tempo-synced LFOs keep
//! time with the transport.

use super::*;

fn timeline(e: &mut Engine, frame: u64, beat: f64, rate: f64, playing: bool) {
    e.command(Command::SetTimeline { beat, rate, playing: playing as u8 }, frame as f64);
}

/// Frames where the output starts again after silence.
fn onsets(x: &[f32]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut quiet = 64;
    for (i, &v) in x.iter().enumerate() {
        if v.abs() > 1e-6 {
            if quiet >= 64 {
                out.push(i);
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    out
}

/// A 2-bar clip (notes on beats 0, 1.5, 4 and 6.5), each note's first sample audible.
fn clip_engine() -> Box<Engine> {
    let mut e = engine(SR);
    set(&mut e, p::OSC_PHASE[0], 90.0);
    set(&mut e, p::ENV_ATTACK[0], 0.0);
    set(&mut e, p::ENV_RELEASE[0], 0.5);
    set(&mut e, p::CLIP_ENABLE, 1.0);
    let mut tail = Vec::new();
    for (start, key) in [(0.0f32, 60.0f32), (1.5, 64.0), (4.0, 67.0), (6.5, 72.0)] {
        for v in [start, 0.5, key, 1.0, 1.0, 0.0] {
            tail.extend_from_slice(&v.to_le_bytes());
        }
    }
    e.command_with_tail(Command::SetClip { slot: 0, count: 4, length: 8.0 }, &tail);
    render(&mut e, 24_000);
    e
}

#[test]
fn a_timeline_starts_on_its_frame() {
    let mut e = clip_engine();
    // off the 16-frame grid on purpose
    let at = e.frame() + 1_037;
    timeline(&mut e, at, 0.0, 1.0, true);
    let start = e.frame();
    let ons = onsets(&render(&mut e, 24_000 * 8 + 4_000));
    let beats: Vec<f64> = ons.iter().map(|&f| (f as u64 + start - at) as f64 / 24_000.0).collect();
    assert_eq!(beats, vec![0.0, 1.5, 4.0, 6.5, 8.0], "{ons:?}");
}

#[test]
fn joining_mid_way_plays_in_step_with_the_song() {
    let mut e = clip_engine();
    // this instance joins at beat 3.25 of a song that started earlier elsewhere
    let at = e.frame() + 500;
    timeline(&mut e, at, 3.25, 1.0, true);
    let start = e.frame();
    let ons = onsets(&render(&mut e, 24_000 * 8));
    let beats: Vec<f64> = ons.iter().map(|&f| 3.25 + (f as u64 + start - at) as f64 / 24_000.0).collect();
    // the clip carries on as if it had played from the top: beats 4 and 6.5, then the next pass
    assert_eq!(beats, vec![4.0, 6.5, 8.0, 9.5], "{ons:?}");
    // stopping ends what the clip started
    let now = e.frame();
    timeline(&mut e, now, 11.25, 1.0, false);
    render(&mut e, 4_800);
    assert_eq!(e.telemetry()[tel::SEQ_NOTES], 0.0);
}

#[test]
fn a_late_anchor_still_lands_where_it_was_meant_to() {
    let mut e = clip_engine();
    // the beat-0 anchor was meant for 2,000 frames ago (its message came late):
    // the clock is where it would have been, and later notes keep their places
    let meant = e.frame() - 2_000;
    timeline(&mut e, meant, 0.0, 1.0, true);
    let start = e.frame();
    let ons = onsets(&render(&mut e, 24_000 * 8));
    let beats: Vec<f64> = ons.iter().map(|&f| (f as u64 + start - meant) as f64 / 24_000.0).collect();
    assert_eq!(beats, vec![1.5, 4.0, 6.5, 8.0], "{ons:?}");
}

#[test]
fn the_rate_trim_runs_the_clock_faster_or_slower() {
    for rate in [1.0, 1.002, 0.997] {
        let mut e = clip_engine();
        let at = e.frame() + 64;
        timeline(&mut e, at, 0.0, rate, true);
        let start = e.frame();
        let ons = onsets(&render(&mut e, 24_000 * 5));
        let want = [0.0, 1.5, 4.0].map(|b| (at - start) as f64 + b * 24_000.0 / rate);
        for (got, want) in ons.iter().zip(want) {
            assert!((*got as f64 - want).abs() <= 1.0, "rate {rate}: {got} vs {want}");
        }
    }
}

#[test]
fn synced_lfos_keep_their_phase_through_a_tempo_change() {
    let mut e = engine(SR);
    set(&mut e, p::LFO_BPM[0], 1.0);
    set(&mut e, p::LFO_SYNC_RATE[0], 5.0); // one cycle per beat
    set(&mut e, p::LFO_MODE[0], 0.0); // free: the song position
    render(&mut e, 24_000);
    let mut last = e.telemetry()[tel::LFO_PHASE] as f64;
    let mut worst = 0.0f64;
    for k in 0..400 {
        if k == 200 {
            set(&mut e, p::GLOBAL_BPM, 90.0);
        }
        e.render(N, -1.0).unwrap();
        let ph = e.telemetry()[tel::LFO_PHASE] as f64;
        worst = worst.max((ph - last).rem_euclid(1.0));
        last = ph;
    }
    // each sub-block moves it by 16 frames' worth (at 120 BPM, 16/24000 of a cycle); a tempo change used to jump it
    assert!(worst < 1.5 * 16.0 / 24_000.0, "the phase jumped {worst:.4} cycles");
}
