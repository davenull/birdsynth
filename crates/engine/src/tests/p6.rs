//! P6 gates in the engine: recordings play at their root pitch, grains keep
//! to their budget across voices, and spectral notes sound from their
//! note-on frame.

use super::*;
use crate::osc::sample::Recording;
use crate::osc::spectral::tests_support::analyse;
use crate::sources::{TYPE_GRANULAR, TYPE_SAMPLE, TYPE_SPECTRAL};
use crate::spec::protocol::tap;
use crate::tables::AssetBuf;

fn recording(x: &[f32], rate: f32) -> Recording {
    let mut b = AssetBuf::alloc(x.len() * 4).unwrap();
    b.as_f32_mut().copy_from_slice(x);
    Recording::new(b, x.len(), 1, 1, rate, 0).unwrap()
}

fn sine(hz: f64, rate: f64, secs: f64) -> Vec<f32> {
    (0..(rate * secs) as usize).map(|i| (std::f64::consts::TAU * hz * i as f64 / rate).sin() as f32 * 0.5).collect()
}

#[test]
fn a_sample_plays_at_its_root_pitch_within_half_a_cent() {
    for &(rate, root, note) in &[(44_100.0f64, 69u8, 69u8), (48_000.0, 60, 72), (96_000.0, 57, 45), (22_050.0, 64, 64)] {
        let f_root = 440.0 * 2f64.powf((root as f64 - 69.0) / 12.0);
        let mut e = engine(SR);
        e.assets_mut().recs[0] = Some(recording(&sine(f_root, rate, 2.0), rate as f32));
        set(&mut e, p::OSC_TYPE[0], TYPE_SAMPLE as f32);
        set(&mut e, p::OSC_ROOT[0], root as f32);
        set(&mut e, p::OSC_LOOP_MODE[0], 1.0); // forward, so it keeps going
        render(&mut e, 24_000);
        on(&mut e, note, 1);
        render(&mut e, 2048);
        let x = render(&mut e, 32_768);
        let want = f_root * 2f64.powf((note as f64 - root as f64) / 12.0);
        let cents = 1200.0 * (pitch(&x, SR) / want).log2();
        assert!(cents.abs() < 0.5, "{rate} Hz recording, root {root}, note {note}: {cents:+.3} cents");
    }
}

#[test]
fn grains_keep_to_the_budget_across_voices() {
    let mut e = engine(SR);
    e.assets_mut().recs[0] = Some(recording(&sine(220.0, 48_000.0, 2.0), 48_000.0));
    set(&mut e, p::OSC_TYPE[0], TYPE_GRANULAR as f32);
    set(&mut e, p::OSC_GR_DENSITY[0], 200.0);
    set(&mut e, p::OSC_GR_LENGTH[0], 1000.0);
    set(&mut e, p::VOICE_POLYPHONY, 16.0);
    render(&mut e, 24_000);
    for n in 0..16 {
        on(&mut e, 48 + n as u8, n + 1);
    }
    let mut peak = 0.0f32;
    for _ in 0..(48_000 * 3 / 128) {
        render(&mut e, 128);
        let t = e.telemetry();
        peak = peak.max(t[tel::GRAINS]);
        assert!(t[tel::GRAINS] <= 512.0);
    }
    assert_eq!(peak, 512.0);
    assert!(e.telemetry()[tel::GRAINS_STOLEN] > 0.0);
    // releasing every note frees their grains once the voices end
    for n in 0..16 {
        off(&mut e, 48 + n as u8, n + 1);
    }
    render(&mut e, 48_000);
    assert_eq!(e.telemetry()[tel::GRAINS], 0.0);
}

#[test]
fn spectral_notes_sound_from_their_note_on_frame() {
    // a cosine, so the recording's first sample is its peak
    let x: Vec<f32> = (0..48_000).map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / 48_000.0).cos() * 0.5).collect();
    let mut e = engine(SR);
    e.assets_mut().analyses[0] = Some(analyse(&x, 48_000.0));
    set(&mut e, p::OSC_TYPE[0], TYPE_SPECTRAL as f32);
    set(&mut e, p::OSC_ROOT[0], 69.0);
    set(&mut e, p::ENV_ATTACK[0], 0.0);
    e.command(Command::SetTaps { mask: 1 << tap::FOCUS_OSC_A }, 0.0);
    render(&mut e, 24_000);
    // note-on 37 frames into the next block; the oscillator (before the amp
    // envelope ramps in) plays the recording's first sample right there
    let at = e.frame() + 37;
    e.command(Command::NoteOn { note: 69, channel: 0, velocity: 1.0, note_id: 1 }, at as f64);
    let mut y = Vec::new();
    for _ in 0..8 {
        e.render(N, -1.0).unwrap();
        y.extend_from_slice(&e.tap(tap::FOCUS_OSC_A)[..N]);
    }
    assert!(y[..37].iter().all(|&v| v == 0.0), "silent before the note");
    assert!((y[37] - 0.5).abs() < 0.02, "the recording's first sample at the note's frame: {}", y[37]);
    assert!(render(&mut e, 256).iter().any(|v| v.abs() > 0.05), "and heard");
    assert_eq!(e.telemetry()[tel::OSC_ASSETS + 2], (x.len().div_ceil(512) + 3) as f32);
}

#[test]
fn grains_stay_their_voice_s_own_after_a_reset() {
    // the worklet resets the engine when it starts: voices must keep their slots,
    // or every voice would play (and age) every grain
    let mut e = engine(SR);
    e.command(Command::Reset, 0.0);
    e.assets_mut().recs[0] = Some(recording(&sine(220.0, 48_000.0, 2.0), 48_000.0));
    set(&mut e, p::OSC_TYPE[0], TYPE_GRANULAR as f32);
    render(&mut e, 24_000);
    for n in 0..4 {
        on(&mut e, 48 + 7 * n as u8, n + 1);
    }
    let mut low = f32::MAX;
    render(&mut e, 4800);
    for _ in 0..100 {
        render(&mut e, 128);
        low = low.min(e.telemetry()[tel::GRAINS]);
    }
    // 100 ms grains every 40 ms: two or three per voice at every moment
    assert!(low >= 8.0, "as few as {low} grains for four voices");
    let slots: std::collections::BTreeSet<u16> = e.voices().iter().map(|v| v.slot).collect();
    assert_eq!(slots.len(), e.voices().len());
}
