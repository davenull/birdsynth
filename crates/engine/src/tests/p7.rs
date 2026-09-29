//! P7 gates in the engine: arp onsets on exact frames, no stranded notes
//! whatever the sequencer is put through, the keyboard's scale, Voice
//! Control and the oscillators' key ranges.

use super::*;
use wt_dsp::rng::Rng;

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

fn arp_engine() -> Box<Engine> {
    let mut e = engine(SR);
    set(&mut e, p::OSC_PHASE[0], 90.0); // the saw starts away from zero, so a note's first sample shows
    set(&mut e, p::ENV_ATTACK[0], 0.0);
    set(&mut e, p::ENV_RELEASE[0], 0.5);
    set(&mut e, p::ARP_ENABLE, 1.0);
    set(&mut e, p::ARP_GATE, 0.5);
    e
}

#[test]
fn arp_sixteenths_at_120_land_every_6000_frames() {
    let mut e = arp_engine();
    render(&mut e, 24_000);
    // a key pressed 37 frames into a sub-block: the pattern starts on that frame (Launch)
    let at = e.frame() + 16 * 3 + 37 % 16;
    e.command(Command::NoteOn { note: 60, channel: 0, velocity: 1.0, note_id: 1 }, at as f64);
    e.command(Command::NoteOn { note: 67, channel: 0, velocity: 1.0, note_id: 2 }, at as f64);
    let start = e.frame();
    let x = render(&mut e, 6000 * 16 + 1000);
    let ons = onsets(&x);
    assert_eq!(ons.len(), 17, "{ons:?}");
    let first = ons[0];
    assert_eq!(first as u64 + start, at, "the first step is the key's own frame");
    for (i, &f) in ons.iter().enumerate() {
        assert_eq!(f - first, i * 6000, "step {i}");
    }
}

#[test]
fn no_note_is_stranded_by_ten_thousand_operations() {
    let mut e = arp_engine();
    set(&mut e, p::VOICE_POLYPHONY, 16.0);
    set(&mut e, p::ARP_RATE, 5.0); // 1/32, busy
    let mut rng = Rng::new(99);
    let mut held: Vec<u8> = Vec::new();
    let mut id = 1u32;
    for op in 0..10_000 {
        let r = rng.next_u32() % 100;
        match r {
            0..=29 => {
                let n = 36 + (rng.next_u32() % 48) as u8;
                e.command(Command::NoteOn { note: n, channel: 0, velocity: 0.3 + rng.next_f32() * 0.7, note_id: id }, 0.0);
                id += 1;
                held.push(n);
            }
            30..=54 => {
                if !held.is_empty() {
                    let n = held.remove((rng.next_u32() as usize) % held.len());
                    e.command(Command::NoteOff { note: n, channel: 0, velocity: 0.0, note_id: 0 }, 0.0);
                }
            }
            55..=59 => set(&mut e, p::ARP_ENABLE, (rng.next_u32() % 2) as f32),
            60..=63 => set(&mut e, p::ARP_SHAPE, (rng.next_u32() % 11) as f32),
            64..=66 => set(&mut e, p::ARP_HOLD, (rng.next_u32() % 2) as f32),
            67..=69 => set(&mut e, p::ARP_GATE, 0.05 + rng.next_f32() * 1.95),
            70..=73 => e.command(Command::Transport { play: (rng.next_u32() % 2) as u8 }, 0.0),
            74..=77 => set(&mut e, p::CLIP_ENABLE, (rng.next_u32() % 2) as f32),
            78..=81 => set(&mut e, p::CLIP_SLOT, 1.0 + (rng.next_u32() % 3) as f32),
            82..=87 => {
                // rewrite a clip while it may be playing
                let mut tail = Vec::new();
                let n = rng.next_u32() % 12;
                for _ in 0..n {
                    for v in [rng.next_f32() * 4.0, 0.05 + rng.next_f32() * 3.0, 36.0 + (rng.next_u32() % 48) as f32, 1.0, 1.0, 0.0] {
                        tail.extend_from_slice(&v.to_le_bytes());
                    }
                }
                let slot = (rng.next_u32() % 3) as u8;
                e.command_with_tail(Command::SetClip { slot, count: n as u16, length: 1.0 + (rng.next_u32() % 4) as f32 }, &tail);
            }
            88 => set(&mut e, p::ARP_OCTAVES, 1.0 + (rng.next_u32() % 4) as f32),
            89 => {
                // a new pattern: steps on or off, long gates, strums, bends
                let mut tail = Vec::new();
                for i in 0..7 * 16 {
                    let v = match i / 16 {
                        0 => !rng.next_u32().is_multiple_of(4) as u32 as f32,
                        4 => (rng.next_u32() % 7) as f32 - 3.0,
                        6 => (rng.next_u32() % 8) as f32,
                        _ => rng.next_f32(),
                    };
                    tail.extend_from_slice(&v.to_le_bytes());
                }
                e.command_with_tail(Command::SetArpPattern { bank: 0, count: 7 * 16 }, &tail);
            }
            90 => {
                e.command(Command::AllNotesOff, 0.0);
                held.clear();
            }
            _ => set(&mut e, p::KEYS_TRANSPOSE, (rng.next_u32() % 25) as f32 - 12.0),
        }
        if op % 5 == 0 {
            render(&mut e, 64 + (rng.next_u32() % 4) as usize * 64);
        }
    }
    // let go of everything, stop the transport, switch the sequencers off
    for n in held.drain(..) {
        e.command(Command::NoteOff { note: n, channel: 0, velocity: 0.0, note_id: 0 }, 0.0);
    }
    set(&mut e, p::ARP_HOLD, 0.0);
    e.command(Command::Transport { play: 0 }, 0.0);
    render(&mut e, 4800);
    set(&mut e, p::ARP_ENABLE, 0.0);
    set(&mut e, p::CLIP_ENABLE, 0.0);
    render(&mut e, 48_000 * 2);
    let stranded = e.voices().iter().filter(|v| v.active && !v.released).count();
    assert_eq!(stranded, 0, "voices still held");
    assert_eq!(e.telemetry()[tel::SEQ_NOTES], 0.0);
    assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 0.0, "everything has faded out");
}

#[test]
fn the_keyboard_transposes_and_snaps_to_the_scale() {
    let mut e = engine(SR);
    set(&mut e, p::KEYS_SCALE, 2.0); // natural minor
    set(&mut e, p::KEYS_ROOT, 9.0); // A minor: A B C D E F G
    set(&mut e, p::KEYS_TRANSPOSE, 12.0);
    render(&mut e, 24_000);
    on(&mut e, 49, 1); // C#3 + 12 = C#4 (61): not in A minor; snaps down to C4 (60)
    render(&mut e, 256);
    let t = e.telemetry();
    let notes: Vec<f32> = (0..tel::VOICE_NOTE_LEN).map(|i| t[tel::VOICE_NOTE + i]).filter(|&n| n >= 0.0).collect();
    assert_eq!(notes, vec![60.0]);
    off(&mut e, 49, 1);
    render(&mut e, 48_000);
    assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 0.0, "its note-off found it");
}

#[test]
fn voice_control_steps_through_its_values() {
    let mut e = engine(SR);
    set(&mut e, p::FILTER_ENABLE[0], 1.0);
    set(&mut e, p::FILTER_CUTOFF[0], 1000.0);
    set(&mut e, p::VC_RATE, 4.0); // 1/16 at 120: 125 ms a step
    set(&mut e, p::VC_STEPS, 4.0);
    for (i, v) in [0.0, 0.5, -0.5, 1.0].iter().enumerate() {
        set(&mut e, [p::VC_A1, p::VC_A2, p::VC_A3, p::VC_A4][i], *v);
    }
    e.command(Command::SetModSlot { slot: 0, source: source::VOICE_MOD_1, aux: 0, flags: 1, dest: p::FILTER_CUTOFF[0], amount: 0.3, curve: 0.0, output: 1.0 }, 0.0);
    render(&mut e, 24_000);
    on(&mut e, 60, 1);
    let mut seen = Vec::new();
    for _ in 0..6 {
        render(&mut e, 3000); // the middle of each step
        seen.push(e.telemetry()[tel::FOCUS_CUTOFF]);
        render(&mut e, 3000);
    }
    let base = seen[0];
    assert!(seen[1] > base && seen[2] < base && seen[3] > seen[1], "{seen:?}");
    assert!((seen[4] - base).abs() / base < 0.01, "it loops after four steps: {seen:?}");
}

#[test]
fn key_ranges_silence_fold_and_warp() {
    let mut e = engine(SR);
    set(&mut e, p::OSC_KEY_LO[0], 60.0);
    set(&mut e, p::OSC_KEY_HI[0], 72.0);
    render(&mut e, 24_000);
    on(&mut e, 48, 1);
    render(&mut e, 4800);
    assert!(render(&mut e, 4800).iter().all(|&v| v == 0.0), "outside the range: silent");
    off(&mut e, 48, 1);
    render(&mut e, 48_000);
    set(&mut e, p::OSC_KEY_MODE[0], 1.0); // fold
    render(&mut e, 24_000);
    on(&mut e, 48, 2);
    render(&mut e, 4800);
    let x = render(&mut e, 16_384);
    let f = pitch(&x, SR);
    assert!((f / 261.626 - 1.0).abs() < 0.002, "folded an octave up to C4: {f} Hz");
}

/// A clip in slot 0: `notes` as (start, length, key), `length` beats.
fn clip(e: &mut Engine, notes: &[(f32, f32, u8)], length: f32) {
    let mut tail = Vec::new();
    for &(start, len, key) in notes {
        for v in [start, len, key as f32, 1.0, 1.0, 0.0] {
            tail.extend_from_slice(&v.to_le_bytes());
        }
    }
    e.command_with_tail(Command::SetClip { slot: 0, count: notes.len() as u16, length }, &tail);
}

#[test]
fn clip_automation_moves_a_parameter_then_lets_it_go() {
    let mut e = engine(SR);
    let cutoff = p::FILTER_CUTOFF[0];
    e.command(Command::SetParam { id: cutoff, value: 0.3 }, 0.0);
    let mut tail = Vec::new();
    for v in [0.0f32, 0.0, 4.0, 1.0] {
        tail.extend_from_slice(&v.to_le_bytes());
    }
    e.command_with_tail(Command::SetClipLane { slot: 0, lane: 0, param: cutoff, count: 2 }, &tail);
    clip(&mut e, &[], 4.0);
    set(&mut e, p::CLIP_ENABLE, 1.0);
    set(&mut e, p::CLIP_QUANTIZE, 0.0);
    render(&mut e, 24_000);
    assert!((e.param(cutoff) - 0.3).abs() < 1e-6, "stopped: the host's value");
    e.command(Command::Transport { play: 1 }, 0.0);
    render(&mut e, 48_000); // two beats: halfway along the ramp
    assert!((e.param(cutoff) - 0.5).abs() < 0.01, "{}", e.param(cutoff));
    // the host moves it meanwhile: the clip keeps it...
    e.command(Command::SetParam { id: cutoff, value: 0.8 }, 0.0);
    render(&mut e, 24_000);
    assert!((e.param(cutoff) - 0.75).abs() < 0.01, "{}", e.param(cutoff));
    // ...and it goes to that value when the clip stops
    e.command(Command::Transport { play: 0 }, 0.0);
    render(&mut e, 24_000);
    assert!((e.param(cutoff) - 0.8).abs() < 1e-4, "{}", e.param(cutoff));
}

#[test]
fn trigger_keys_launch_clips_instead_of_playing() {
    let mut e = engine(SR);
    set(&mut e, p::CLIP_ENABLE, 1.0);
    set(&mut e, p::CLIP_TRIGGER_KEYS, 1.0);
    render(&mut e, 1600);
    on(&mut e, 26, 1); // D1: clip 3
    render(&mut e, 1600);
    assert!((e.param(p::CLIP_SLOT) - 2.0 / 11.0).abs() < 1e-6);
    assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 0.0, "no note");
    off(&mut e, 26, 1);
    set(&mut e, p::CLIP_TRIGGER_KEYS, 0.0);
    on(&mut e, 26, 2);
    render(&mut e, 1600);
    assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 1.0, "a note again");
}

#[test]
fn swing_moves_arp_and_clip_off_beats_alike() {
    let mut e = arp_engine();
    set(&mut e, p::GLOBAL_SWING, 0.5);
    render(&mut e, 24_000);
    e.command(Command::Transport { play: 1 }, 0.0);
    on(&mut e, 60, 1);
    let x = render(&mut e, 24_000);
    let arp = onsets(&x);
    off(&mut e, 60, 1);
    set(&mut e, p::ARP_ENABLE, 0.0);
    set(&mut e, p::ENV_RELEASE[0], 0.0);
    render(&mut e, 12_000);
    // the same rhythm from a clip of sixteenths, each 1/32 long
    clip(&mut e, &[(0.0, 0.125, 60), (0.25, 0.125, 60), (0.5, 0.125, 60), (0.75, 0.125, 60)], 1.0);
    set(&mut e, p::CLIP_ENABLE, 1.0);
    set(&mut e, p::CLIP_QUANTIZE, 0.0);
    e.command(Command::Transport { play: 1 }, 0.0);
    let x = render(&mut e, 24_000);
    let clip = onsets(&x);
    let rel = |o: &[usize]| o.iter().map(|&f| f - o[0]).collect::<Vec<_>>();
    assert_eq!(rel(&arp), vec![0, 7_500, 12_000, 19_500], "arp: {arp:?}");
    assert_eq!(rel(&clip), rel(&arp), "clip: {clip:?}");
}
