//! P2 gates: envelope and LFO timing, the modulation policy, routing,
//! noise, fused cross-modulation and oversampling.

use super::*;
use crate::osc::kernel::{self, XIn};
use crate::osc::unison::{MAX_LANES, UniParams};
use crate::osc::{self, OscSettings, OscVoice};
use crate::samples;
use crate::spec::protocol::tap;
use crate::tables::Tables;
use crate::voice::Voice;
use wt_dsp::math;
use wt_dsp::rng::Rng;

const MS: f32 = SR / 1000.0;

/// Render one sub-block at a time, calling `f` after each.
fn run(e: &mut Engine, frames: usize, mut f: impl FnMut(&Engine)) {
    for _ in 0..frames / N {
        e.render(N, -1.0).unwrap();
        f(e);
    }
}

fn route(e: &mut Engine, slot: u8, source: u8, dest: u16, amount: f32) {
    e.command(Command::SetModSlot { slot, source, aux: 0, flags: 0, dest, amount, curve: 0.0, output: 1.0 }, 0.0);
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

fn voice_of(e: &Engine, note_id: u32) -> &Voice {
    e.voices().iter().find(|v| v.active && v.note_id == note_id).expect("voice is playing")
}

// ----------------------------------------------------------- envelopes

#[test]
fn envelope_segments_land_within_a_millisecond() {
    // (tempo, env rate, expected time scale): BPM mode scales by 120/tempo, the rate divides
    for (bpm, rate, scale) in [(120.0, 1.0, 1.0), (60.0, 1.0, 2.0), (120.0, 2.0, 0.5)] {
        let mut e = engine(SR);
        let env = 1; // Env 2 (Env 1 stays open so the voice lives through the release)
        let (a, h, d, s, r) = (100.0, 50.0, 200.0, 0.5, 300.0);
        set(&mut e, p::ENV_ATTACK[env], a);
        set(&mut e, p::ENV_HOLD[env], h);
        set(&mut e, p::ENV_DECAY[env], d);
        set(&mut e, p::ENV_SUSTAIN[env], s);
        set(&mut e, p::ENV_RELEASE[env], r);
        set(&mut e, p::ENV_RELEASE[0], 5000.0);
        if bpm != 120.0 {
            set(&mut e, p::ENV_BPM[env], 1.0);
            set(&mut e, p::GLOBAL_BPM, bpm);
        }
        set(&mut e, p::GLOBAL_ENV_RATE, rate);
        e.render(N, -1.0).unwrap();
        on(&mut e, 60, 1);
        let t0 = e.frame();
        let level = |e: &Engine| e.telemetry()[tel::FOCUS_ENV + env];
        let (mut attack, mut hold, mut decay) = (None, None, None);
        run(&mut e, (1000.0 * scale * MS) as usize, |e| {
            let t = (e.frame() - t0) as f32;
            let lv = level(e);
            if attack.is_none() && lv >= 1.0 - 1e-6 {
                attack = Some(t);
            } else if attack.is_some() && hold.is_none() && lv < 1.0 - 1e-6 {
                hold = Some(t - N as f32); // the decay began inside this sub-block
            } else if hold.is_some() && decay.is_none() && (lv - s).abs() < 1e-6 {
                decay = Some(t);
            }
        });
        off(&mut e, 60, 1);
        let t_off = e.frame();
        let mut release = None;
        run(&mut e, (500.0 * scale * MS) as usize, |e| {
            if release.is_none() && level(e) == 0.0 {
                release = Some((e.frame() - t_off) as f32);
            }
        });
        let ms = |x: Option<f32>| x.expect("segment ended") / MS;
        for (name, got, want) in [
            ("attack", ms(attack), a * scale),
            ("hold", ms(hold), (a + h) * scale),
            ("decay", ms(decay), (a + h + d) * scale),
            ("release", ms(release), r * scale),
        ] {
            assert!((got - want).abs() <= 1.0, "bpm {bpm} rate {rate}: {name} ended at {got:.2} ms, want {want:.2} ms");
        }
    }
}

#[test]
fn legato_invert_flips_which_envelopes_retrigger() {
    for from_zero in [false, true] {
        let mut e = engine(SR);
        set(&mut e, p::VOICE_MONO, 1.0);
        set(&mut e, p::VOICE_LEGATO, 1.0);
        for env in 0..2 {
            set(&mut e, p::ENV_ATTACK[env], 50.0);
            set(&mut e, p::ENV_ATTACK_CURVE[env], 0.0);
            set(&mut e, p::ENV_DECAY[env], 50.0);
            set(&mut e, p::ENV_SUSTAIN[env], 0.5);
        }
        set(&mut e, p::ENV_LEGATO_INVERT[1], 1.0);
        if from_zero {
            set(&mut e, p::ENV_RETRIG[1], 1.0);
        }
        on(&mut e, 60, 1);
        render(&mut e, 9600);
        on(&mut e, 62, 2); // overlapping: legato
        render(&mut e, 480); // 10 ms into env 2's new attack
        let t = e.telemetry();
        assert!((t[tel::FOCUS_ENV] - 0.5).abs() < 1e-6, "env 1 carries on through legato notes");
        let want = if from_zero { 0.2 } else { 0.6 };
        assert!((t[tel::FOCUS_ENV + 1] - want).abs() < 0.01, "env 2 restarts from {}: {}", if from_zero { "zero" } else { "its level" }, t[tel::FOCUS_ENV + 1]);
        assert_eq!(t[tel::FOCUS_PITCH], 62.0);
    }
}

// ---------------------------------------------------------------- LFOs

#[test]
fn lfo_period_is_within_a_tenth_of_a_percent() {
    // Free (shared) and Retrig (per voice) LFOs at a spread of rates
    for (mode, hz) in [(0.0, 2.5), (1.0, 0.37), (1.0, 7.0), (0.0, 31.0)] {
        let mut e = engine(SR);
        set(&mut e, p::LFO_MODE[0], mode);
        set(&mut e, p::LFO_RATE[0], hz);
        e.render(N, -1.0).unwrap();
        on(&mut e, 60, 1);
        let rate = e.params().plain(p::LFO_RATE[0]) as f64;
        let mut x = Vec::new();
        run(&mut e, (60.0 * SR) as usize, |e| x.push(e.telemetry()[tel::LFO_VALUE]));
        let got = pitch(&x, SR / N as f32);
        assert!((got / rate - 1.0).abs() < 1e-3, "mode {mode}: {got:.6} Hz, want {rate:.6} Hz");
    }
}

#[test]
fn synced_lfos_stay_locked_to_the_beat() {
    let mut e = engine(SR);
    set(&mut e, p::GLOBAL_BPM, 128.0);
    for i in 0..2 {
        set(&mut e, p::LFO_BPM[i], 1.0);
        set(&mut e, p::LFO_SYNC_RATE[i], 5.0); // 1/4: one cycle per beat
    }
    set(&mut e, p::LFO_MODE[0], 0.0); // Free: follows the song position
    set(&mut e, p::LFO_MODE[1], 1.0); // Retrig: counts from the note
    e.render(N, -1.0).unwrap();
    on(&mut e, 60, 1);
    let t_on = e.frame();
    let per_frame = e.params().plain(p::GLOBAL_BPM) as f64 / 60.0 / SR as f64;
    let dist = |a: f64, b: f64| {
        let d = (a - b).rem_euclid(1.0);
        d.min(1.0 - d)
    };
    let mut worst: (f64, f64) = (0.0, 0.0);
    for _ in 0..60 {
        run(&mut e, SR as usize, |_| {});
        // telemetry holds the phase at the start of the last sub-block
        let free = e.telemetry()[tel::LFO_PHASE] as f64;
        let retrig = e.telemetry()[tel::LFO_PHASE + 1] as f64;
        worst.0 = worst.0.max(dist(free, (e.frame() - N as u64) as f64 * per_frame));
        worst.1 = worst.1.max(dist(retrig, (e.frame() - t_on) as f64 * per_frame));
    }
    assert!(worst.0 < 1e-4, "free-running synced LFO drifted {:.2e} cycles", worst.0);
    assert!(worst.1 < 1e-3, "retriggered synced LFO drifted {:.2e} cycles", worst.1);
}

#[test]
fn drawn_lfo_shapes_arrive_by_command() {
    let mut e = engine(SR);
    // a near-square: high for the first half-cycle, low for the second
    let pts: [(f32, f32, f32); 4] = [(0.0, 1.0, 0.0), (0.49, 1.0, 0.0), (0.5, -1.0, 0.0), (0.99, -1.0, 0.0)];
    let mut payload = vec![0u8, 0, pts.len() as u8, 0, 0, 0, 0, 0]; // lfo 0, kind 0 (curve), count, padding
    for (x, y, c) in pts {
        for v in [x, y, c] {
            payload.extend_from_slice(&v.to_le_bytes());
        }
    }
    let mut cmd = Vec::new();
    cmd.extend_from_slice(&proto::op::SET_LFO_SHAPE.to_le_bytes());
    cmd.extend_from_slice(&[0, 0]);
    cmd.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    cmd.extend_from_slice(&0f64.to_le_bytes());
    cmd.extend_from_slice(&payload);
    assert_eq!(e.apply(&cmd), 1);
    set(&mut e, p::LFO_MODE[0], 1.0);
    set(&mut e, p::LFO_RATE[0], 1.0);
    e.render(N, -1.0).unwrap();
    on(&mut e, 60, 1);
    render(&mut e, 12_000); // a quarter cycle
    assert!((e.telemetry()[tel::LFO_VALUE] - 1.0).abs() < 1e-3);
    render(&mut e, 24_000); // three quarters
    assert!((e.telemetry()[tel::LFO_VALUE] + 1.0).abs() < 1e-3);
    assert_eq!(e.telemetry()[tel::UNKNOWN_CMDS], 0.0);
}

#[test]
fn chaos_and_sample_and_hold_stay_in_range() {
    for kind in [2.0, 3.0, 4.0] {
        let mut e = engine(SR);
        set(&mut e, p::LFO_TYPE[0], kind);
        set(&mut e, p::LFO_RATE[0], 5.0);
        e.render(N, -1.0).unwrap();
        on(&mut e, 60, 1);
        let mut values = Vec::new();
        run(&mut e, 10 * SR as usize, |e| values.push((e.telemetry()[tel::LFO_VALUE], e.telemetry()[tel::LFO_Y])));
        assert!(values.iter().all(|(x, y)| x.abs() <= 1.0 && y.abs() <= 1.0), "type {kind} left -1..1");
        let moved = values.windows(2).filter(|w| w[0].0 != w[1].0).count();
        if kind == 4.0 {
            // S&H: one new value per cycle, 5 Hz for 10 s
            assert!((49..=51).contains(&moved), "S&H stepped {moved} times");
        } else {
            assert!(moved > values.len() / 2, "chaos type {kind} should move continuously");
            let spread = values.iter().fold((1f32, -1f32), |(lo, hi), (x, _)| (lo.min(*x), hi.max(*x)));
            assert!(spread.1 - spread.0 > 0.5, "chaos type {kind} covers too little range: {spread:?}");
        }
    }
}

// ------------------------------------------------------ modulation policy

#[test]
fn global_destinations_follow_the_newest_voice_and_hold() {
    let mut e = engine(SR);
    let dest = p::MASTER_VOLUME;
    route(&mut e, 0, source::VELOCITY, dest, -0.5);
    set(&mut e, p::ENV_RELEASE[0], 5.0);
    let base = e.params().norm(dest);
    let modv = |e: &Engine| e.telemetry()[tel::MOD_VALUE];
    e.command(Command::NoteOn { note: 60, channel: 0, velocity: 1.0, note_id: 1 }, 0.0);
    render(&mut e, 480);
    assert!((modv(&e) - (base - 0.5)).abs() < 1e-6);
    e.command(Command::NoteOn { note: 64, channel: 0, velocity: 0.2, note_id: 2 }, 0.0);
    render(&mut e, 480);
    assert!((modv(&e) - (base - 0.1)).abs() < 1e-6, "follows the newest voice");
    off(&mut e, 60, 1);
    render(&mut e, 4800);
    assert!((modv(&e) - (base - 0.1)).abs() < 1e-6, "an older voice ending changes nothing");
    off(&mut e, 64, 2);
    render(&mut e, 4800);
    assert_eq!(e.telemetry()[tel::VOICES_ACTIVE], 0.0);
    assert!((modv(&e) - (base - 0.1)).abs() < 1e-6, "held after the last voice ends");
}

#[test]
fn shared_lfos_restart_with_every_note_and_poly_lfos_do_not() {
    let mut e = engine(SR);
    for i in 0..2 {
        set(&mut e, p::LFO_MODE[i], 1.0);
        set(&mut e, p::LFO_RATE[i], 1.0);
        // routed, so every voice evaluates them
        route(&mut e, i as u8, source::LFO_1 + i as u8, p::OSC_PAN[0], 0.01);
    }
    set(&mut e, p::LFO_POLY[0], 0.0); // LFO 1 is shared, LFO 2 runs per voice
    on(&mut e, 60, 1);
    render(&mut e, 12_000); // 0.25 s
    on(&mut e, 64, 2);
    render(&mut e, 4_800); // 0.1 s
    let (a, b) = (voice_of(&e, 1), voice_of(&e, 2));
    assert_eq!(a.tel_lfo[0].0, b.tel_lfo[0].0, "a shared LFO has one value for all voices");
    assert!((e.telemetry()[tel::LFO_PHASE] - 0.1).abs() < 0.01, "and the newest note restarted it");
    assert!((a.tel_lfo[1].2 - 0.35).abs() < 0.01 && (b.tel_lfo[1].2 - 0.1).abs() < 0.01, "per-voice phases {} and {}", a.tel_lfo[1].2, b.tel_lfo[1].2);
}

#[test]
fn macros_and_controllers_reach_every_voice() {
    let mut e = engine(SR);
    route(&mut e, 0, source::MACRO_1, p::OSC_WT_POS[0], 1.0);
    route(&mut e, 1, source::MOD_WHEEL, p::OSC_LEVEL[0], -0.5);
    set(&mut e, p::MACRO_VALUE[0], 0.7);
    e.command(Command::Controller { channel: 0, cc: 1, value: 0.4 }, 0.0);
    render(&mut e, 24_000); // macros glide like any smoothed parameter
    on(&mut e, 60, 1);
    on(&mut e, 67, 2);
    render(&mut e, 480);
    let (a, b) = (voice_of(&e, 1), voice_of(&e, 2));
    assert_eq!(a.tel_mod.0[..2], b.tel_mod.0[..2]);
    assert!((a.tel_mod.0[0] - (e.params().norm(p::OSC_WT_POS[0]) + 0.7)).abs() < 1e-6);
    assert!((a.tel_mod.0[1] - (e.params().norm(p::OSC_LEVEL[0]) - 0.2)).abs() < 1e-6);
}

// -------------------------------------------------------------- routing

#[test]
fn sources_route_to_filters_buses_or_nowhere() {
    let level = |route: f32, send: f32| {
        let mut e = engine(SR);
        set(&mut e, p::OSC_ENABLE[0], 0.0);
        set(&mut e, p::SUB_ENABLE, 1.0);
        set(&mut e, p::SUB_ROUTE, route);
        set(&mut e, p::SUB_SEND1, send);
        set(&mut e, p::FILTER_ENABLE[0], 1.0);
        set(&mut e, p::FILTER_CUTOFF[0], 20.0); // far below the sub (65 Hz)
        on(&mut e, 48, 1);
        render(&mut e, 4800);
        rms(&render(&mut e, 9600))
    };
    let (filters, main, direct, none) = (level(0.0, 0.0), level(1.0, 0.0), level(2.0, 0.0), level(3.0, 0.0));
    assert!(main > 0.05, "the sub plays: {main}");
    assert!(filters < 0.2 * main, "through the closed filter: {filters} vs {main}");
    assert!((direct / main - 1.0).abs() < 1e-4, "direct skips the filters too: {direct} vs {main}");
    assert_eq!(none, 0.0);
    let sent = level(3.0, 1.0);
    assert!((sent / main - 1.0).abs() < 1e-4, "a full send carries the source to bus 1: {sent} vs {main}");
}

#[test]
fn filter_2_runs_in_series_or_in_parallel() {
    // everything into filter 1 (a low-pass), filter 2 a high-pass above it
    let level = |parallel: bool| {
        let mut e = engine(SR);
        for f in 0..2 {
            set(&mut e, p::FILTER_ENABLE[f], 1.0);
        }
        set(&mut e, p::FILTER_TYPE[0], 0.0);
        set(&mut e, p::FILTER_CUTOFF[0], 300.0);
        set(&mut e, p::FILTER_TYPE[1], 2.0);
        set(&mut e, p::FILTER_CUTOFF[1], 5000.0);
        set(&mut e, p::OSC_BALANCE[0], 0.0);
        set(&mut e, p::MIX_FILTER_ROUTING, if parallel { 1.0 } else { 0.0 });
        on(&mut e, 45, 1);
        render(&mut e, 4800);
        rms(&render(&mut e, 9600))
    };
    let (serial, parallel) = (level(false), level(true));
    assert!(parallel > 0.05, "parallel passes filter 1's output: {parallel}");
    assert!(serial < 0.05 * parallel, "in series the high-pass removes what the low-pass kept: {serial} vs {parallel}");
}

#[test]
fn noise_plays_its_sample_once_or_looped() {
    for oneshot in [false, true] {
        let mut e = engine(SR);
        let frames = 4800;
        let mut a = AssetBuf::alloc(frames * 4).unwrap();
        for (i, v) in a.as_f32_mut().iter_mut().enumerate() {
            *v = ((i as f32 * 0.37).sin() * 0.5).round() - 0.25; // any audible signal
        }
        e.samples_mut().load(samples::NOISE, a, frames, SR).unwrap();
        set(&mut e, p::OSC_ENABLE[0], 0.0);
        set(&mut e, p::NOISE_ENABLE, 1.0);
        set(&mut e, p::NOISE_ONESHOT, if oneshot { 1.0 } else { 0.0 });
        set(&mut e, p::NOISE_PHASE, 0.0);
        set(&mut e, p::NOISE_RAND, 0.0);
        on(&mut e, 60, 1);
        let x = render(&mut e, 2 * frames);
        assert!(rms(&x[..frames]) > 0.02, "noise plays");
        if oneshot {
            assert_eq!(rms(&x[frames + N..]), 0.0, "a one-shot ends with its sample");
        } else {
            assert!(rms(&x[frames..]) > 0.02, "a loop keeps going");
        }
    }
}

// --------------------------------------------------- fused cross-mod

/// The fused per-sample loop against a reference built independently from
/// the same kernels: A <- B FM (B renders first), then A <-> B, a cycle
/// where A hears B's previous sample and B hears A's current one. A has 4
/// unison lanes and B 3, so carrier lane k follows modulator lane k mod 3.
#[test]
fn fused_fm_matches_the_reference() {
    for cycle in [false, true] {
        let mut e = engine(SR);
        set(&mut e, p::OSC_ENABLE[1], 1.0);
        set(&mut e, p::OSC_PHASE[1], 0.0);
        set(&mut e, p::OSC_RAND_PHASE[1], 0.0);
        set(&mut e, p::OSC_UNISON[0], 4.0);
        set(&mut e, p::OSC_UNISON[1], 3.0);
        set(&mut e, p::OSC_SEMI[1], 7.0);
        set(&mut e, p::OSC_WARP1_MODE[0], 18.0); // FM <- B
        set(&mut e, p::OSC_WARP1_AMOUNT[0], 0.3);
        if cycle {
            set(&mut e, p::OSC_WARP1_MODE[1], 17.0); // FM <- A
            set(&mut e, p::OSC_WARP1_AMOUNT[1], 0.2);
        }
        set(&mut e, p::GLOBAL_QUALITY, 0.0); // compare at the output rate
        e.command(Command::SetTaps { mask: (1 << tap::FOCUS_OSC_A) | (1 << tap::FOCUS_OSC_B) }, 0.0);
        render(&mut e, 24_000); // let smoothed parameters settle exactly
        let note = 57u8;
        on(&mut e, note, 1);
        let blocks = 3000;
        let mut got = [Vec::new(), Vec::new()];
        for _ in 0..blocks {
            e.render(N, -1.0).unwrap();
            got[0].extend_from_slice(&e.tap(tap::FOCUS_OSC_A)[..N]);
            got[1].extend_from_slice(&e.tap(tap::FOCUS_OSC_B)[..N]);
        }

        // --- the reference
        let pr = e.params();
        let fm = [Some(pr.plain(p::OSC_WARP1_AMOUNT[0])), cycle.then(|| pr.plain(p::OSC_WARP1_AMOUNT[1]))];
        let settings = |o: usize| {
            let base = pr.plain(p::OSC_OCTAVE[o]) * 12.0 + pr.plain(p::OSC_FINE[o]) * 0.01 + pr.plain(p::OSC_COARSE[o]);
            let hz = math::note_to_hz((note as f32 + base + pr.plain(p::OSC_SEMI[o])) as f64) as f32;
            OscSettings {
                hz,
                pos: pr.plain(p::OSC_WT_POS[o]),
                smooth: pr.plain(p::OSC_WT_SMOOTH[o]) >= 0.5,
                uni: UniParams {
                    voices: pr.plain(p::OSC_UNISON[o]) as usize,
                    detune: pr.plain(p::OSC_DETUNE[o]),
                    blend: pr.plain(p::OSC_BLEND[o]),
                    width: pr.plain(p::OSC_WIDTH[o]),
                    range: pr.plain(p::OSC_UNI_RANGE[o]),
                    mode: pr.plain(p::OSC_UNI_MODE[o]) as u8,
                    stack: pr.plain(p::OSC_UNI_STACK[o]) as u8,
                },
                wt_spread: pr.plain(p::OSC_WT_SPREAD[o]),
                warp_spread: pr.plain(p::OSC_WARP_SPREAD[o]),
                w1: (pr.plain(p::OSC_WARP1_MODE[o]) as u8, pr.plain(p::OSC_WARP1_AMOUNT[o])),
                w2: (pr.plain(p::OSC_WARP2_MODE[o]) as u8, pr.plain(p::OSC_WARP2_AMOUNT[o])),
                xstretch: fm[o].map_or(1.0, |a| osc::xstretch(&osc::XMod { kind: osc::XKind::Fm, src: 1 - o as u8, amount: a })),
            }
        };
        let s = [settings(0), settings(1)];
        let tables = Tables::default();
        let (table, frames) = tables.get(0);
        let remap = wt_dsp::warp::Remap::default();
        let mut ov = [OscVoice::default(); 2];
        let mut rng = Rng::new(7);
        for v in ov.iter_mut() {
            v.start(0.0, 0.0, None, &mut rng);
        }
        // B first when only A listens; a cycle falls back to index order
        let order: [usize; 2] = if cycle { [0, 1] } else { [1, 0] };
        let lanes = [s[0].uni.voices, s[1].uni.voices];
        let mut prev = [[0.0f32; MAX_LANES]; 2];
        let mut want = [Vec::new(), Vec::new()];
        for _ in 0..blocks {
            let k = [ov[0].prepare(&s[0], table, frames, &remap, SR, N), ov[1].prepare(&s[1], table, frames, &remap, SR, N)];
            let mut inc = [k[0].inc0, k[1].inc0];
            for i in 0..N {
                let mut cur = prev;
                let mut done = [false; 2];
                let mut out = [0.0f32; 2];
                for &o in &order {
                    let src = 1 - o;
                    let mut x = XIn::default();
                    if let Some(a) = fm[o] {
                        for l in 0..k[o].lanes {
                            let v = if done[src] { cur[src][l % lanes[src]] } else { prev[src][l % lanes[src]] };
                            x.fm[l] += osc::FM_DEPTH * a * v;
                        }
                    }
                    let (l, r) = kernel::step(&k[o], &mut ov[o].phase, i, inc[o], &x, &mut cur[o]);
                    out[o] = 0.5 * (l + r);
                    inc[o] *= k[o].inc_step;
                    done[o] = true;
                }
                prev = cur;
                want[0].push(out[0]);
                want[1].push(out[1]);
            }
        }
        for o in 0..2 {
            let first = got[o].iter().zip(&want[o]).position(|(g, w)| g.to_bits() != w.to_bits());
            assert!(first.is_none(), "cycle {cycle}, osc {o}: differs from sample {:?}", first);
            assert!(rms(&got[o]) > 0.1, "osc {o} plays");
        }
    }
}

#[test]
fn fm_and_ring_mod_change_the_sound() {
    // with a modulator present, each cross-mod kind moves the carrier's spectrum
    let tone = |mode: f32| {
        let mut e = engine(SR);
        set(&mut e, p::OSC_ENABLE[1], 1.0);
        set(&mut e, p::OSC_LEVEL[1], 0.0); // B only modulates
        set(&mut e, p::OSC_SEMI[1], 12.0);
        set(&mut e, p::OSC_WARP1_MODE[0], mode);
        set(&mut e, p::OSC_WARP1_AMOUNT[0], if mode == 0.0 { 0.0 } else { 0.5 });
        on(&mut e, 57, 1);
        render(&mut e, 4800);
        render(&mut e, 8192)
    };
    let dry = tone(0.0);
    for (mode, name) in [(18.0, "FM"), (25.0, "PD"), (31.0, "PD self"), (33.0, "AM"), (40.0, "RM")] {
        let wet = tone(mode);
        let diff: Vec<f32> = dry.iter().zip(&wet).map(|(a, b)| a - b).collect();
        assert!(rms(&diff) > 0.05 * rms(&dry), "{name} made no difference");
        assert!(wet.iter().all(|v| v.is_finite() && v.abs() < 4.0), "{name} blew up");
    }
}

// --------------------------------------------------------- oversampling

/// Worst non-harmonic bin of a held note, in dB below its strongest harmonic.
fn worst_alias(e: &mut Engine, f0: f32) -> f32 {
    let n = 1 << 16;
    let bin_hz = SR / n as f32;
    let x = render(e, n);
    let s = spectrum(&x);
    let fund = s.iter().fold(0.0f32, |m, &v| m.max(v));
    let mut worst = -200.0f32;
    for (k, &mag) in s.iter().enumerate() {
        let hz = k as f32 * bin_hz;
        if !(20.0..=18_000.0).contains(&hz) {
            continue;
        }
        let h = (hz / f0).round();
        if h >= 1.0 && (hz - h * f0).abs() <= 8.0 * bin_hz {
            continue;
        }
        worst = worst.max(20.0 * (mag / fund).log10());
    }
    worst
}

#[test]
fn oversampling_cuts_sync_aliasing() {
    let alias = |quality: f32| {
        let mut e = engine(SR);
        set(&mut e, p::GLOBAL_QUALITY, quality);
        set(&mut e, p::OSC_WARP1_MODE[0], 1.0); // Sync
        set(&mut e, p::OSC_WARP1_AMOUNT[0], 0.63); // a ratio of 10.45: the reset is a jump
        on(&mut e, 84, 1);
        render(&mut e, 4800);
        assert_eq!(e.telemetry()[tel::OVERSAMPLE], [1.0, 2.0, 4.0][quality as usize]);
        worst_alias(&mut e, math::note_to_hz(84.0) as f32)
    };
    let (x1, x2, x4) = (alias(0.0), alias(1.0), alias(2.0));
    eprintln!("sync aliasing, worst non-harmonic bin: 1x {x1:.1} dB, 2x {x2:.1} dB, 4x {x4:.1} dB");
    assert!(x2 < x1 - 6.0 && x4 < x2, "sync aliasing at 1x/2x/4x: {x1:.1} / {x2:.1} / {x4:.1} dBc");

    // no warp that needs it: no oversampling, whatever the setting
    let mut e = engine(SR);
    set(&mut e, p::GLOBAL_QUALITY, 2.0);
    on(&mut e, 60, 1);
    render(&mut e, 480);
    assert_eq!(e.telemetry()[tel::OVERSAMPLE], 1.0);
}

// ----------------------------------------------------------- FX routing (P3)

/// Sends reach the bus racks, whose output joins the Main rack or goes
/// straight to the master; chains arrive through the SetChain command.
#[test]
fn bus_racks_route_to_main_or_master() {
    let chain_cmd = |chain: u8, refs: &[u16]| {
        let mut payload = vec![chain, refs.len() as u8, 0, 0, 0, 0, 0, 0];
        for r in refs {
            payload.extend_from_slice(&r.to_le_bytes());
        }
        while payload.len() % 8 != 0 {
            payload.push(0);
        }
        let mut cmd = Vec::new();
        cmd.extend_from_slice(&proto::op::SET_CHAIN.to_le_bytes());
        cmd.extend_from_slice(&[0, 0]);
        cmd.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        cmd.extend_from_slice(&0f64.to_le_bytes());
        cmd.extend_from_slice(&payload);
        cmd
    };
    let level = |to_master: bool| {
        let mut e = engine(SR);
        // osc A only on bus 1; the main rack mutes everything (utility at -36 dB)
        set(&mut e, p::OSC_ROUTE[0], 3.0);
        set(&mut e, p::OSC_SEND1[0], 1.0);
        set(&mut e, p::FX_UTILITY_GAIN[0], -36.0);
        set(&mut e, p::RACK_BUS1_TO, if to_master { 1.0 } else { 0.0 });
        assert_eq!(e.apply(&chain_cmd(0, &[crate::fx::entry(crate::fx::UTILITY, 0)])), 1);
        on(&mut e, 57, 1);
        render(&mut e, 4800);
        rms(&render(&mut e, 9600))
    };
    let (via_main, direct) = (level(false), level(true));
    assert!(direct > 0.05, "bus 1 reaches the master: {direct}");
    assert!(via_main < 0.03 * direct, "routed into Main, the muting utility applies: {via_main} vs {direct}");
}
