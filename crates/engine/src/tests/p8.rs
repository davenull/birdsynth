//! P8 gates in the engine: MPE (each note's own bend, slide and pressure).

use super::*;

fn expr(e: &mut Engine, note_id: u32, kind: u8, value: f32) {
    e.command(Command::NoteExpression { kind, value, note_id }, 0.0);
}

fn on_ch(e: &mut Engine, note: u8, channel: u8, id: u32) {
    e.command(Command::NoteOn { note, channel, velocity: 1.0, note_id: id }, 0.0);
}

#[test]
fn mpe_bends_each_note_by_its_own_x() {
    for (x, want_semis) in [(0.5f32, 24.0f64), (-0.25, -12.0), (1.0, 48.0)] {
        let mut e = engine(SR);
        set(&mut e, p::VOICE_MPE, 1.0);
        on_ch(&mut e, 60, 1, 1);
        expr(&mut e, 1, 0, x);
        render(&mut e, 24_000);
        let hz = pitch(&render(&mut e, 48_000), SR);
        let want = 440.0 * 2f64.powf((60.0 + want_semis - 69.0) / 12.0);
        let cents = 1200.0 * (hz / want).log2();
        assert!(cents.abs() < 0.5, "X {x}: {hz:.3} Hz vs {want:.3}");
    }
    // with MPE off, X only feeds its source
    let mut e = engine(SR);
    on_ch(&mut e, 60, 1, 1);
    expr(&mut e, 1, 0, 0.5);
    render(&mut e, 24_000);
    let hz = pitch(&render(&mut e, 48_000), SR);
    assert!((hz - 261.6256).abs() < 0.05, "{hz}");
    assert!((e.voices().iter().find(|v| v.note_id == 1).unwrap().mpe_now()[0] - 0.5).abs() < 1e-3);
}

#[test]
fn mpe_expression_reaches_only_its_own_note() {
    let mut e = engine(SR);
    set(&mut e, p::VOICE_MPE, 1.0);
    set(&mut e, p::VOICE_MPE_RANGE, 12.0);
    on_ch(&mut e, 60, 1, 1);
    on_ch(&mut e, 64, 2, 2);
    on_ch(&mut e, 67, 3, 3);
    expr(&mut e, 1, 0, -0.5);
    expr(&mut e, 1, 2, 0.3);
    expr(&mut e, 2, 1, 0.8);
    expr(&mut e, 99, 1, 1.0); // no such note: ignored
    render(&mut e, 24_000);
    let get = |id: u32| e.voices().iter().find(|v| v.active && v.note_id == id).unwrap().mpe_now();
    let close = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
    assert!(close(get(1), [-0.5, 0.0, 0.3]), "{:?}", get(1));
    assert!(close(get(2), [0.0, 0.8, 0.0]), "{:?}", get(2));
    assert!(close(get(3), [0.0, 0.0, 0.0]), "{:?}", get(3));
}

#[test]
fn mpe_sources_drive_the_matrix_per_note() {
    // MPE Y opens the filter of its own note only (a routing per voice, read from the focused voice's telemetry)
    let mut e = engine(SR);
    set(&mut e, p::VOICE_MPE, 1.0);
    set(&mut e, p::FILTER_ENABLE[0], 1.0);
    let flags = 0;
    e.command(Command::SetModSlot { slot: 0, source: source::MPE_Y, aux: 0, flags, dest: p::FILTER_CUTOFF[0], amount: 0.5, curve: 0.0, output: 1.0 }, 0.0);
    on_ch(&mut e, 60, 1, 1);
    render(&mut e, 4_800);
    // the matrix's first destination is the cutoff
    let base = e.telemetry()[tel::MOD_VALUE];
    expr(&mut e, 1, 1, 1.0);
    render(&mut e, 24_000);
    let moved = e.telemetry()[tel::MOD_VALUE];
    assert!((moved - base - 0.5).abs() < 0.01, "{base} → {moved}");
}

#[test]
fn the_cpu_guard_drops_oversampling_then_unison() {
    // an FM patch at 4x with 16-voice unison
    let heavy = |guard: u8, unison: f32| {
        let mut e = engine(SR);
        set(&mut e, p::GLOBAL_QUALITY, 2.0);
        set(&mut e, p::OSC_UNISON[0], unison);
        set(&mut e, p::OSC_DETUNE[0], 0.3);
        set(&mut e, p::OSC_WARP1_MODE[0], 1.0); // sync: wants oversampling
        set(&mut e, p::OSC_WARP1_AMOUNT[0], 0.5);
        e.command(Command::SetGuard { level: guard }, 0.0);
        on(&mut e, 48, 1);
        let x = render(&mut e, 9600);
        (x, e.telemetry()[tel::OVERSAMPLE], e.telemetry()[tel::GUARD])
    };
    let (_, os, g) = heavy(0, 16.0);
    assert_eq!((os, g), (4.0, 0.0));
    let (_, os, g) = heavy(1, 16.0);
    assert_eq!((os, g), (1.0, 1.0), "level 1: no oversampling");
    // level 3 caps unison at 4: the same as asking for 4
    let (capped, os, _) = heavy(3, 16.0);
    let (four, _, _) = heavy(3, 4.0);
    assert_eq!(os, 1.0);
    assert_eq!(capped, four);
    let (sixteen, _, _) = heavy(1, 16.0);
    assert_ne!(capped, sixteen);
    let (_, _, g) = heavy(9, 16.0);
    assert_eq!(g, 4.0, "clamped to the last level");
}
