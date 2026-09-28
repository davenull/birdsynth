//! Engine-level tests: timing, voicing and the P1 sound gates.

use realfft::RealFftPlanner;

use crate::engine::{Engine, RenderError};
use crate::params::Curve;
use crate::spec::params::{self as p, INFO};
use crate::spec::protocol::{self as proto, Command, MAX_BLOCK, SUB_BLOCK as N, VOICE_SLOTS, source, tel};
use crate::tables::AssetBuf;
use wt_dsp::mip::FRAME_STRIDE;

const SR: f32 = 48_000.0;

/// Normalized value for a plain one (inverting the parameter's curve).
fn norm(id: u16, plain: f32) -> f32 {
    let i = INFO[id as usize];
    let span = i.max - i.min;
    match i.curve {
        Curve::Lin | Curve::Int | Curve::Db => (plain - i.min) / span,
        Curve::Exp => (plain / i.min).ln() / (i.max / i.min).ln(),
        Curve::Pow(k) => ((plain - i.min) / span).max(0.0).powf(1.0 / k),
        Curve::Bool => plain,
        Curve::Enum(n) => plain / (n as f32 - 1.0),
    }
}

fn set(e: &mut Engine, id: u16, plain: f32) {
    e.command(Command::SetParam { id, value: norm(id, plain) }, 0.0);
}

fn on(e: &mut Engine, note: u8, id: u32) {
    e.command(Command::NoteOn { note, channel: 0, velocity: 1.0, note_id: id }, 0.0);
}

fn off(e: &mut Engine, note: u8, id: u32) {
    e.command(Command::NoteOff { note, channel: 0, velocity: 0.0, note_id: id }, 0.0);
}

/// Render `frames` and return the left channel.
fn render(e: &mut Engine, frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames);
    let mut left = frames;
    while left > 0 {
        let n = left.min(128);
        e.render(n.div_ceil(N) * N, -1.0).unwrap();
        out.extend_from_slice(&e.out(0)[..n]);
        left -= n;
    }
    out
}

/// A plain patch: fixed start phase, so tests repeat exactly.
fn engine(sr: f32) -> Box<Engine> {
    let mut e = Engine::new(sr);
    set(&mut e, p::OSC_RAND_PHASE[0], 0.0);
    set(&mut e, p::OSC_PHASE[0], 0.0);
    e.render(16, 0.0).unwrap();
    e
}

/// Frequency from a least-squares fit through the rising zero crossings.
fn pitch(x: &[f32], sr: f32) -> f64 {
    let mut z = Vec::new();
    for i in 1..x.len() {
        if x[i - 1] < 0.0 && x[i] >= 0.0 {
            z.push((i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64);
        }
    }
    let n = z.len() as f64;
    let mi = (n - 1.0) / 2.0;
    let mz = z.iter().sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for (i, t) in z.iter().enumerate() {
        num += (i as f64 - mi) * (t - mz);
        den += (i as f64 - mi).powi(2);
    }
    sr as f64 / (num / den)
}

/// Magnitude spectrum with a 4-term Blackman-Harris window (sidelobes near -92 dB).
fn spectrum(x: &[f32]) -> Vec<f32> {
    let n = x.len();
    let mut input: Vec<f32> = x
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let t = std::f64::consts::TAU * i as f64 / n as f64;
            let w = 0.35875 - 0.48829 * t.cos() + 0.14128 * (2.0 * t).cos() - 0.01168 * (3.0 * t).cos();
            (v as f64 * w) as f32
        })
        .collect();
    let fft = RealFftPlanner::<f32>::new().plan_fft_forward(n);
    let mut out = fft.make_output_vec();
    fft.process(&mut input, &mut out).unwrap();
    out.iter().map(|c| c.norm()).collect()
}

// ------------------------------------------------------------ timing

#[test]
fn a4_is_440_hz() {
    for &sr in &[44_100.0f32, 48_000.0] {
        let mut e = engine(sr);
        on(&mut e, 69, 1);
        let x = render(&mut e, (sr * 2.0) as usize);
        let hz = pitch(&x[(sr * 0.1) as usize..], sr);
        assert!((hz - 440.0).abs() < 0.01, "{sr}: {hz}");
    }
}

#[test]
fn pitch_is_within_half_a_cent_from_c0_to_c8() {
    for &sr in &[44_100.0f32, 48_000.0, 96_000.0] {
        for octave in 0..=8 {
            let note = 12 * (octave + 1);
            let mut e = engine(sr);
            on(&mut e, note as u8, 1);
            let secs = if octave < 2 { 3.0 } else { 1.0 };
            let x = render(&mut e, (sr * secs) as usize);
            let hz = pitch(&x[(sr * 0.05) as usize..], sr);
            let want = 440.0 * 2f64.powf((note as f64 - 69.0) / 12.0);
            let cents = 1200.0 * (hz / want).log2();
            assert!(cents.abs() < 0.5, "C{octave} at {sr}: {hz:.4} Hz, {cents:.3} cents");
        }
    }
}

#[test]
fn note_on_starts_on_its_exact_frame() {
    let mut e = engine(SR);
    e.command(Command::NoteOn { note: 60, channel: 0, velocity: 1.0, note_id: 1 }, 116.0);
    e.render(16 * 16, 16.0).unwrap();
    let x = &e.out(0)[..256];
    // the saw starts at 0 rising, so the first sounding sample is small but nonzero
    assert!(x[..100].iter().all(|&v| v == 0.0));
    assert!(x[101..110].iter().all(|&v| v != 0.0));
}

#[test]
fn params_land_on_the_next_grid_boundary() {
    let mut e = Engine::new(SR);
    e.command(Command::SetParam { id: p::VOICE_POLYPHONY, value: norm(p::VOICE_POLYPHONY, 3.0) }, 5.0);
    e.render(16, 0.0).unwrap();
    assert_eq!(e.params().plain(p::VOICE_POLYPHONY), 8.0);
    e.render(16, -1.0).unwrap();
    assert_eq!(e.params().plain(p::VOICE_POLYPHONY), 3.0);
}

#[test]
fn rejects_bad_block_sizes() {
    let mut e = Engine::new(SR);
    assert_eq!(e.render(15, 0.0), Err(RenderError::BadFrames));
    assert_eq!(e.render(MAX_BLOCK + 16, 0.0), Err(RenderError::BadFrames));
    assert!(e.render(MAX_BLOCK, 0.0).is_ok());
}

#[test]
fn decodes_a_batch_and_counts_garbage() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&proto::op::SET_PARAM.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&(proto::bytes::SET_PARAM as u32).to_le_bytes());
    buf.extend_from_slice(&0f64.to_le_bytes());
    buf.extend_from_slice(&p::VOICE_POLYPHONY.to_le_bytes());
    buf.extend_from_slice(&[0, 0]);
    buf.extend_from_slice(&norm(p::VOICE_POLYPHONY, 4.0).to_le_bytes());
    buf.extend_from_slice(&999u16.to_le_bytes());
    buf.extend_from_slice(&[0, 0]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&0f64.to_le_bytes());
    let mut e = Engine::new(SR);
    assert_eq!(e.apply(&buf), 1);
    e.render(16, 0.0).unwrap();
    assert_eq!(e.params().plain(p::VOICE_POLYPHONY), 4.0);
    assert_eq!(e.telemetry()[tel::UNKNOWN_CMDS], 1.0);
}

// ----------------------------------------------------------- voicing

fn sounding(e: &Engine) -> Vec<f32> {
    let t = e.telemetry();
    let mut v: Vec<f32> = (0..VOICE_SLOTS).map(|i| t[tel::VOICE_NOTE + i]).filter(|&n| n >= 0.0).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

#[test]
fn steal_priorities() {
    // notes 60, 72, 64 at velocities 1.0, 0.3, 0.6; poly 2; a fourth note steals one
    let cases = [(0.0, 60.0), (1.0, 64.0), (2.0, 60.0), (3.0, 72.0), (4.0, 72.0)];
    for (policy, victim) in cases {
        let mut e = engine(SR);
        set(&mut e, p::VOICE_POLYPHONY, 3.0);
        set(&mut e, p::VOICE_STEAL, policy);
        render(&mut e, 64);
        for (i, (note, vel)) in [(60u8, 1.0f32), (72, 0.3), (64, 0.6)].iter().enumerate() {
            e.command(Command::NoteOn { note: *note, channel: 0, velocity: *vel, note_id: i as u32 + 1 }, 0.0);
            render(&mut e, 64);
        }
        on(&mut e, 67, 9);
        render(&mut e, 1024);
        let notes = sounding(&e);
        assert_eq!(notes.len(), 3, "policy {policy}: {notes:?}");
        assert!(!notes.contains(&victim), "policy {policy} should steal {victim}: {notes:?}");
    }
}

#[test]
fn note_off_by_id_releases_only_that_note() {
    let mut e = engine(SR);
    on(&mut e, 60, 7);
    on(&mut e, 60, 8);
    render(&mut e, 128);
    off(&mut e, 60, 7);
    render(&mut e, 48_000 / 4);
    assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 1.0);
}

#[test]
fn sustain_pedal_holds_notes_until_released() {
    let mut e = engine(SR);
    on(&mut e, 60, 1);
    e.command(Command::Controller { channel: 0, cc: 64, value: 1.0 }, 0.0);
    render(&mut e, 256);
    off(&mut e, 60, 1);
    render(&mut e, 24_000);
    assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 1.0, "pedal holds the note");
    e.command(Command::Controller { channel: 0, cc: 64, value: 0.0 }, 0.0);
    render(&mut e, 24_000);
    assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 0.0);
}

#[test]
fn mono_legato_glides_without_retriggering() {
    let mut e = engine(SR);
    set(&mut e, p::VOICE_MONO, 1.0);
    set(&mut e, p::VOICE_LEGATO, 1.0);
    set(&mut e, p::VOICE_GLIDE, 100.0);
    set(&mut e, p::ENV_ATTACK[0], 50.0);
    on(&mut e, 48, 1);
    render(&mut e, 48_000 / 2); // attack done, sustaining
    on(&mut e, 60, 2);
    render(&mut e, 16);
    let t = e.telemetry();
    assert_eq!(t[tel::VOICES_ACTIVE], 1.0);
    assert!(t[tel::FOCUS_ENV] > 0.99, "legato keeps the envelope at sustain");
    assert!(t[tel::FOCUS_PITCH] > 48.0 && t[tel::FOCUS_PITCH] < 49.0, "glide has just started: {}", t[tel::FOCUS_PITCH]);
    render(&mut e, 48_000 * 60 / 1000);
    let mid = e.telemetry()[tel::FOCUS_PITCH];
    assert!(mid > 53.0 && mid < 57.0, "halfway through a 100 ms glide: {mid}");
    render(&mut e, 48_000 / 10);
    assert_eq!(e.telemetry()[tel::FOCUS_PITCH], 60.0);
    // releasing the top note returns to the held one
    off(&mut e, 60, 2);
    render(&mut e, 48_000 / 5);
    assert_eq!(e.telemetry()[tel::FOCUS_PITCH], 48.0);
}

#[test]
fn pitch_bend_follows_the_range() {
    let mut e = engine(SR);
    set(&mut e, p::VOICE_BEND_UP, 7.0);
    on(&mut e, 57, 1);
    e.command(Command::PitchBend { channel: 0, value: 1.0 }, 0.0);
    render(&mut e, 48_000 / 4);
    let x = render(&mut e, 48_000);
    let hz = pitch(&x, SR);
    let want = 220.0 * 2f64.powf(7.0 / 12.0);
    assert!((hz - want).abs() < 0.05, "{hz} vs {want}");
}

// -------------------------------------------------- tables and matrix

fn table_asset(frames: usize, value_of: impl Fn(usize, usize) -> f32) -> AssetBuf {
    let mut a = AssetBuf::alloc(frames * FRAME_STRIDE * 4).unwrap();
    let data = a.as_f32_mut();
    for f in 0..frames {
        for i in 0..FRAME_STRIDE {
            data[f * FRAME_STRIDE + i] = value_of(f, i);
        }
    }
    a
}

#[test]
fn tables_load_and_play() {
    // (Commands carry wasm32 pointers, so natively the table goes in directly;
    // the command path is covered by the wasm tests in Node.)
    let mut e = engine(SR);
    e.tables_mut().load(0, table_asset(4, |f, _| f as f32 * 0.25), 4).unwrap();
    set(&mut e, p::OSC_WT_POS[0], 1.0);
    on(&mut e, 60, 1);
    render(&mut e, 4800);
    let t = e.telemetry();
    assert_eq!(t[tel::OSC_FRAMES], 4.0);
    assert_eq!(t[tel::TABLE_ERRORS], 0.0);
    // a DC table: frame 3 is 0.75 everywhere; after the -6 dB master that's
    // 0.75 (frame) * 0.75 (level) * 0.5
    let x = render(&mut e, 256);
    assert!((x[200] - 0.75 * 0.75 * 0.501_187).abs() < 1e-3, "{}", x[200]);
    // a null asset is rejected and counted, and the table stays
    e.command(Command::LoadTable { osc: 0, frames: 4, ptr: 0, bytes: 64 }, 0.0);
    render(&mut e, 16);
    assert_eq!(e.telemetry()[tel::TABLE_ERRORS], 1.0);
    assert_eq!(e.telemetry()[tel::OSC_FRAMES], 4.0);
}

#[test]
fn matrix_moves_parameters_per_voice() {
    let mut e = engine(SR);
    set(&mut e, p::FILTER_ENABLE[0], 1.0);
    set(&mut e, p::FILTER_CUTOFF[0], 200.0);
    set(&mut e, p::ENV_SUSTAIN[1], 1.0);
    e.command(Command::SetModSlot { slot: 0, source: source::ENV_2, aux: 0, flags: 0, dest: p::FILTER_CUTOFF[0], amount: 0.5 }, 0.0);
    on(&mut e, 60, 1);
    render(&mut e, 4800);
    let t = e.telemetry();
    assert_eq!(t[tel::MOD_SLOTS], 1.0);
    let want = INFO[p::FILTER_CUTOFF[0] as usize].to_plain(norm(p::FILTER_CUTOFF[0], 200.0) + 0.5);
    assert!((t[tel::FOCUS_CUTOFF] - want).abs() / want < 1e-3, "{} vs {want}", t[tel::FOCUS_CUTOFF]);
    // the mod wheel on WT position
    e.command(Command::SetModSlot { slot: 1, source: source::MOD_WHEEL, aux: 0, flags: 0, dest: p::OSC_WT_POS[0], amount: 1.0 }, 0.0);
    e.command(Command::Controller { channel: 0, cc: 1, value: 0.4 }, 0.0);
    render(&mut e, 64);
    assert!((e.telemetry()[tel::OSC_WT_POS] - 0.4).abs() < 1e-6);
}

// ----------------------------------------------------------- P1 gates

#[test]
fn simd_and_scalar_engines_agree_bit_for_bit() {
    let mut a = engine(SR);
    let mut b = engine(SR);
    b.set_scalar(true);
    for e in [&mut a, &mut b] {
        set(e, p::OSC_UNISON[0], 7.0);
        set(e, p::OSC_WARP1_MODE[0], 3.0);
        set(e, p::OSC_WARP1_AMOUNT[0], 0.4);
        set(e, p::OSC_WARP_SPREAD[0], 0.5);
        set(e, p::OSC_ENABLE[1], 1.0);
        set(e, p::OSC_UNISON[1], 3.0);
        on(e, 60, 1);
        on(e, 67, 2);
    }
    let (xa, xb) = (render(&mut a, 9600), render(&mut b, 9600));
    assert!(xa.iter().zip(&xb).all(|(p, q)| p.to_bits() == q.to_bits()));
}

#[test]
fn saw_aliasing_stays_60_db_down() {
    let n = 1 << 16;
    let bin_hz = SR / n as f32;
    let mut f0 = 20.0f32;
    let mut worst: (f32, f32) = (-200.0, 0.0);
    while f0 <= 12_000.0 {
        let mut e = engine(SR);
        let note = 69.0 + 12.0 * (f0 / 440.0).log2();
        let base = note.floor();
        set(&mut e, p::OSC_COARSE[0], note - base);
        on(&mut e, base as u8, 1);
        render(&mut e, 4800);
        let x = render(&mut e, n);
        let s = spectrum(&x);
        let fund = s[(f0 / bin_hz).round() as usize - 3..=(f0 / bin_hz).round() as usize + 3].iter().fold(0.0f32, |m, &v| m.max(v));
        for (k, &mag) in s.iter().enumerate() {
            let hz = k as f32 * bin_hz;
            if !(20.0..=18_000.0).contains(&hz) {
                continue;
            }
            let h = (hz / f0).round();
            if h >= 1.0 && (hz - h * f0).abs() <= 8.0 * bin_hz {
                continue; // a harmonic (within the window's main lobe)
            }
            let db = 20.0 * (mag / fund).log10();
            if db > worst.0 {
                worst = (db, f0);
            }
        }
        f0 *= 1.19;
    }
    assert!(worst.0 <= -60.0, "worst non-harmonic bin {:.1} dBc at f0 {:.1} Hz", worst.0, worst.1);
}

#[test]
fn glide_has_no_timbre_steps() {
    // a two-octave glide over two seconds: the spectral centroid must move smoothly
    let mut e = engine(SR);
    set(&mut e, p::VOICE_MONO, 1.0);
    set(&mut e, p::VOICE_GLIDE, 2000.0);
    set(&mut e, p::VOICE_GLIDE_ALWAYS, 1.0);
    on(&mut e, 48, 1);
    render(&mut e, 4800);
    on(&mut e, 72, 2);
    let x = render(&mut e, (SR * 2.0) as usize);
    // the centroid of the audible band: partials allowed to fold back above
    // 18 kHz are inaudible by design and are left out
    let (win, hop) = (4096usize, 512usize);
    let top = (16_000.0 * win as f32 / SR) as usize;
    let mut prev: Option<f32> = None;
    let mut worst = 0.0f32;
    let mut i = 0;
    while i + win <= x.len() {
        let s = spectrum(&x[i..i + win]);
        let (mut num, mut den) = (0.0f32, 0.0f32);
        for (k, &m) in s.iter().enumerate().take(top) {
            num += k as f32 * m;
            den += m;
        }
        let c = num / den;
        if std::env::var("CENTROID_DEBUG").is_ok() {
            eprintln!("{:6} {:8.2} {:8.2}", i, c * SR / win as f32, if let Some(q) = prev { (c / q - 1.0) * 100.0 } else { 0.0 });
        }
        if let Some(q) = prev {
            worst = worst.max((c / q - 1.0).abs());
        }
        prev = Some(c);
        i += hop;
    }
    assert!(worst < 0.03, "centroid stepped by {:.1}%", worst * 100.0);
}

#[test]
fn unison_detunes_symmetrically() {
    let mut e = engine(SR);
    set(&mut e, p::OSC_UNISON[0], 2.0);
    set(&mut e, p::OSC_DETUNE[0], 1.0); // ±1 semitone at the default 2-semitone range
    set(&mut e, p::OSC_WIDTH[0], 0.0);
    set(&mut e, p::OSC_WT_POS[0], 0.0);
    on(&mut e, 69, 1);
    render(&mut e, 4800);
    let x = render(&mut e, 1 << 16);
    let s = spectrum(&x);
    let bin = SR / (1 << 16) as f32;
    // energy around the peak, so where a tone falls between bins doesn't matter
    let peak_near = |hz: f32| {
        let k = (hz / bin).round() as usize;
        s[k - 5..=k + 5].iter().map(|&v| v * v).sum::<f32>().sqrt()
    };
    let lo = peak_near(440.0 * 2f32.powf(-1.0 / 12.0));
    let hi = peak_near(440.0 * 2f32.powf(1.0 / 12.0));
    let mid = peak_near(440.0);
    assert!((lo / hi - 1.0).abs() < 0.05, "{lo} vs {hi}");
    assert!(mid < lo * 0.05, "no energy left at the undetuned pitch");
}
