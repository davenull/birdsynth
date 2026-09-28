//! P3 FX gates: delay timing, reverb decay, EQ and compressor curves,
//! crossovers, frequency shifting, convolution, distortion aliasing,
//! denormals, and click-free bypass and reordering.

use realfft::RealFftPlanner;

use super::*;
use crate::modmatrix::Matrix;
use crate::params::{Curve, ParamStore};
use crate::spec::params::{self as p, INFO};
use crate::spec::protocol::MAX_MOD_SLOTS;
use wt_dsp::filter;

const SR: f32 = 48_000.0;

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

/// The effects on their own: parameters, a chain setup, and a runner.
struct Rig {
    fx: Box<Fx>,
    params: ParamStore,
    matrix: Matrix,
    off: [f32; MAX_MOD_SLOTS],
    beat: f64,
}

impl Rig {
    fn new() -> Rig {
        Rig { fx: Fx::new(SR), params: ParamStore::default(), matrix: Matrix::default(), off: [0.0; MAX_MOD_SLOTS], beat: 0.0 }
    }

    fn set(&mut self, id: u16, plain: f32) -> &mut Rig {
        self.params.set(id as usize, norm(id, plain));
        self
    }

    fn chain(&mut self, chain: usize, refs: &[u16]) -> &mut Rig {
        self.fx.set_chain(chain, refs);
        self
    }

    /// Run `frames` of input through the racks (into Main); returns the output.
    fn run(&mut self, frames: usize, mut input: impl FnMut(usize) -> [f32; 2]) -> Vec<[f32; 2]> {
        let mut out = Vec::with_capacity(frames);
        let mut t = 0;
        while t < frames {
            let mut main = [[0.0f32; N]; 2];
            for n in 0..N {
                let x = input(t + n);
                main[0][n] = x[0];
                main[1][n] = x[1];
            }
            let (mut b1, mut b2) = ([[0.0f32; N]; 2], [[0.0f32; N]; 2]);
            let cx = Ctx::new(SR, 120.0, self.beat, 60.0, &self.params, &self.matrix, &self.off);
            self.fx.process(&cx, &mut main, &mut b1, &mut b2);
            self.beat += N as f64 / SR as f64 * 2.0;
            for n in 0..N {
                if t + n < frames {
                    out.push([main[0][n], main[1][n]]);
                }
            }
            t += N;
        }
        out
    }
}

fn sine(f: f64, amp: f32) -> impl FnMut(usize) -> [f32; 2] {
    move |i| {
        let v = amp * (std::f64::consts::TAU * f * i as f64 / SR as f64).sin() as f32;
        [v, v]
    }
}

/// Amplitude of `f` in a signal (one channel), by correlation.
fn amp_at(x: &[f32], f: f64) -> f64 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for (i, &v) in x.iter().enumerate() {
        let ph = std::f64::consts::TAU * f * i as f64 / SR as f64;
        s += v as f64 * ph.sin();
        c += v as f64 * ph.cos();
    }
    2.0 * (s * s + c * c).sqrt() / x.len() as f64
}

fn left(v: &[[f32; 2]]) -> Vec<f32> {
    v.iter().map(|s| s[0]).collect()
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-12).log10()
}

// -------------------------------------------------------------------- delay

#[test]
fn delay_time_is_exact_to_a_sample() {
    for &(ms, mode) in &[(250.0f32, 0.0f32), (3.3, 0.0), (125.0, 1.0)] {
        let mut r = Rig::new();
        r.chain(MAIN, &[entry(DELAY, 0)]);
        r.set(p::FX_DELAY_BPM[0], 0.0).set(p::FX_DELAY_TIME_L[0], ms).set(p::FX_DELAY_LINK[0], 1.0);
        r.set(p::FX_DELAY_FEEDBACK[0], 0.0).set(p::FX_DELAY_MIX[0], 1.0).set(p::FX_DELAY_MODE[0], mode);
        let want = r.params.plain(p::FX_DELAY_TIME_L[0]) * SR / 1000.0;
        let out = r.run((want * 2.5) as usize + 64, |i| if i == 0 { [1.0, 1.0] } else { [0.0, 0.0] });
        let ch = if mode == 1.0 { 1 } else { 0 }; // ping-pong: the first echo on the left, the second on the right
        let peak = (0..out.len()).max_by(|&a, &b| out[a][ch].abs().total_cmp(&out[b][ch].abs())).unwrap();
        let expect = if mode == 1.0 { 2.0 * want } else { want };
        assert!((peak as f32 - expect).abs() <= 1.0, "{ms} ms mode {mode}: echo at {peak}, want {expect:.1}");
    }
}

// ------------------------------------------------------------------- reverb

/// T30 (×2) from the Schroeder energy decay of an impulse response.
fn rt60(ir: &[[f32; 2]]) -> f32 {
    let e: Vec<f64> = ir.iter().map(|s| (s[0] as f64).powi(2) + (s[1] as f64).powi(2)).collect();
    let mut edc = vec![0.0f64; e.len()];
    let mut acc = 0.0;
    for i in (0..e.len()).rev() {
        acc += e[i];
        edc[i] = acc;
    }
    let total = edc[0];
    let t = |d: f64| edc.iter().position(|&v| 10.0 * (v / total).log10() <= d).unwrap_or(e.len()) as f32 / SR;
    2.0 * (t(-35.0) - t(-5.0))
}

#[test]
fn reverb_decays_in_the_set_time() {
    for algo in 0..5 {
        for &decay in &[1.0f32, 3.0] {
            let mut r = Rig::new();
            r.chain(MAIN, &[entry(REVERB, 0)]);
            r.set(p::FX_REVERB_ALGO[0], algo as f32).set(p::FX_REVERB_DECAY[0], decay).set(p::FX_REVERB_PREDELAY[0], 0.0);
            r.set(p::FX_REVERB_DAMP[0], 20_000.0).set(p::FX_REVERB_LOWCUT[0], 20.0).set(p::FX_REVERB_MOVEMENT[0], 0.0).set(p::FX_REVERB_MIX[0], 1.0);
            let want = r.params.plain(p::FX_REVERB_DECAY[0]);
            let ir = r.run((SR * (want * 1.6 + 0.5)) as usize, |i| if i == 0 { [1.0, 1.0] } else { [0.0, 0.0] });
            let got = rt60(&ir);
            assert!((got / want - 1.0).abs() < 0.1, "algorithm {algo}: RT60 {got:.2} s, set {want:.2} s");
        }
    }
}

// ----------------------------------------------------------------------- EQ

#[test]
fn eq_matches_the_analytic_response() {
    // (low type, low f, low q, low gain), mid, (high type, f, q, gain)
    for &((lt, lf, lq, lg), (mf, mq, mg), (ht, hf, hq, hg)) in &[
        ((0.0, 200.0, 0.5, 6.0), (1500.0, 0.7, -4.0), (0.0, 8000.0, 0.5, 3.0)),
        ((1.0, 90.0, 0.8, -9.0), (400.0, 0.3, 5.0), (1.0, 5000.0, 0.4, -6.0)),
        ((2.0, 120.0, 0.2, 0.0), (2500.0, 0.9, 12.0), (2.0, 9000.0, 0.3, 0.0)),
    ] {
        let mut r = Rig::new();
        r.chain(MAIN, &[entry(EQ, 0)]);
        r.set(p::FX_EQ_LOW_TYPE[0], lt).set(p::FX_EQ_LOW_FREQ[0], lf).set(p::FX_EQ_LOW_Q[0], lq).set(p::FX_EQ_LOW_GAIN[0], lg);
        r.set(p::FX_EQ_MID_FREQ[0], mf).set(p::FX_EQ_MID_Q[0], mq).set(p::FX_EQ_MID_GAIN[0], mg);
        r.set(p::FX_EQ_HIGH_TYPE[0], ht).set(p::FX_EQ_HIGH_FREQ[0], hf).set(p::FX_EQ_HIGH_Q[0], hq).set(p::FX_EQ_HIGH_GAIN[0], hg);
        let cx = Ctx::new(SR, 120.0, 0.0, 60.0, &r.params, &r.matrix, &r.off);
        let bands = eq::Eq::settings(&cx, 0);
        for &f in &[40.0f64, 110.0, 250.0, 700.0, 1500.0, 3500.0, 8000.0, 14000.0] {
            let want: f64 = bands.iter().map(|b| filter::response(b.kind, b.cutoff, b.res, b.var, SR, f as f32) as f64).product();
            let mut rr = Rig::new();
            rr.params = ParamStore::default();
            for id in 0..INFO.len() {
                rr.params.set(id, r.params.norm(id as u16));
            }
            rr.chain(MAIN, &[entry(EQ, 0)]);
            let out = rr.run(SR as usize, sine(f, 0.1));
            let got = amp_at(&left(&out)[SR as usize / 2..], f) / 0.1;
            assert!((db(got) - db(want)).abs() < 0.3, "EQ at {f} Hz: {:.2} dB, analytic {:.2} dB", db(got), db(want));
        }
    }
}

// --------------------------------------------------------------- compressor

#[test]
fn compressor_follows_its_static_curve() {
    let (thr, ratio, knee) = (-20.0f32, 4.0f32, 6.0f32);
    for &level in &[-40.0f32, -30.0, -24.0, -21.0, -19.0, -17.0, -12.0, -6.0, 0.0] {
        let mut r = Rig::new();
        r.chain(MAIN, &[entry(COMPRESSOR, 0)]);
        r.set(p::FX_COMPRESSOR_THRESHOLD[0], thr).set(p::FX_COMPRESSOR_RATIO[0], ratio).set(p::FX_COMPRESSOR_KNEE[0], knee);
        r.set(p::FX_COMPRESSOR_ATTACK[0], 1.0).set(p::FX_COMPRESSOR_RELEASE[0], 800.0).set(p::FX_COMPRESSOR_GAIN[0], 0.0).set(p::FX_COMPRESSOR_MIX[0], 1.0);
        let amp = 10f32.powf(level / 20.0);
        let out = r.run(SR as usize, sine(997.0, amp));
        let tail = &left(&out)[(SR * 0.7) as usize..];
        let peak = tail.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let got = 20.0 * (peak / amp).log10();
        let want = dynamics::static_curve(level, r.params.plain(p::FX_COMPRESSOR_THRESHOLD[0]), r.params.plain(p::FX_COMPRESSOR_RATIO[0]), r.params.plain(p::FX_COMPRESSOR_KNEE[0]));
        assert!((got - want).abs() < 0.5, "input {level} dB: gain {got:.2} dB, curve says {want:.2} dB");
    }
}

// --------------------------------------------------------------- crossovers

#[test]
fn crossovers_add_back_up_flat() {
    let freqs = [30.0f64, 80.0, 200.0, 300.0, 500.0, 1000.0, 2000.0, 3000.0, 5000.0, 10000.0, 16000.0];
    // splitter two- and three-band, the multiband compressor at no compression, and mono bass
    type Setup = Box<dyn Fn(&mut Rig)>;
    let setups: Vec<(&str, Setup)> = vec![
        ("splitter low/high", Box::new(|r: &mut Rig| {
            r.chain(MAIN, &[entry(SPLITTER, 0)]);
            r.set(p::FX_SPLITTER_MODE[0], 0.0).set(p::FX_SPLITTER_FREQ1[0], 500.0);
        })),
        ("splitter low/mid/high", Box::new(|r: &mut Rig| {
            r.chain(MAIN, &[entry(SPLITTER, 1)]);
            r.set(p::FX_SPLITTER_MODE[1], 1.0).set(p::FX_SPLITTER_FREQ1[1], 300.0).set(p::FX_SPLITTER_FREQ2[1], 3000.0);
        })),
        ("multiband compressor at rest", Box::new(|r: &mut Rig| {
            r.chain(MAIN, &[entry(COMPRESSOR, 0)]);
            r.set(p::FX_COMPRESSOR_MODE[0], 1.0).set(p::FX_COMPRESSOR_DEPTH[0], 0.0);
        })),
        ("mono bass", Box::new(|r: &mut Rig| {
            r.chain(MAIN, &[entry(UTILITY, 0)]);
            r.set(p::FX_UTILITY_MONO_BASS[0], 1.0).set(p::FX_UTILITY_BASS_FREQ[0], 150.0);
        })),
    ];
    for (name, setup) in &setups {
        for &f in &freqs {
            let mut r = Rig::new();
            setup(&mut r);
            let out = r.run(SR as usize, sine(f, 0.25));
            let g = amp_at(&left(&out)[SR as usize / 2..], f) / 0.25;
            assert!(db(g).abs() < 0.1, "{name} at {f} Hz: {:.3} dB", db(g));
        }
    }
}

// ------------------------------------------------------------ freq shifter

fn frequency(x: &[f32]) -> f64 {
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
    SR as f64 / (num / den)
}

#[test]
fn frequency_shifter_moves_by_the_set_hz() {
    for &(f, shift) in &[(1000.0f64, 100.0f32), (1000.0, -250.0), (440.0, 33.3)] {
        let mut r = Rig::new();
        r.chain(MAIN, &[entry(BODE, 0)]);
        r.set(p::FX_BODE_SHIFT[0], shift).set(p::FX_BODE_MIX[0], 1.0).set(p::FX_BODE_FEEDBACK[0], 0.0);
        let want = f + r.params.plain(p::FX_BODE_SHIFT[0]) as f64;
        let out = r.run(2 * SR as usize, sine(f, 0.5));
        let got = frequency(&left(&out)[SR as usize / 2..]);
        assert!((got - want).abs() < 0.1, "{f} Hz shifted by {shift}: {got:.3} Hz, want {want:.3}");
    }
}

// --------------------------------------------------------------- convolve

#[test]
fn convolution_matches_direct_convolution() {
    let mut rng = wt_dsp::rng::Rng::new(9);
    for &taps in &[40usize, 700, 6000, 20_000] {
        let ir: [Vec<f32>; 2] = std::array::from_fn(|_| (0..taps).map(|t| (rng.next_f32() * 2.0 - 1.0) * (-(t as f32) / (taps as f32 * 0.3)).exp()).collect());
        let mut asset = crate::tables::AssetBuf::alloc(2 * convolve::channel_len(taps) * 4).unwrap();
        convolve::prepare([&ir[0], &ir[1]], asset.as_f32_mut());
        let mut r = Rig::new();
        assert!(r.fx.load_ir(0, Some(asset), taps));
        r.chain(MAIN, &[entry(CONVOLVE, 0)]);
        r.set(p::FX_CONVOLVE_MIX[0], 1.0).set(p::FX_CONVOLVE_PREDELAY[0], 0.0).set(p::FX_CONVOLVE_LOWCUT[0], 20.0).set(p::FX_CONVOLVE_HIGHCUT[0], 20_000.0).set(p::FX_CONVOLVE_WIDTH[0], 1.0);
        let n = taps + 9000;
        let input: Vec<[f32; 2]> = (0..n).map(|_| [rng.next_f32() * 2.0 - 1.0, rng.next_f32() * 2.0 - 1.0]).collect();
        let out = r.run(n, |i| input.get(i).copied().unwrap_or([0.0, 0.0]));
        let (mut err, mut sig) = (0.0f64, 0.0f64);
        for ch in 0..2 {
            for t in 0..n {
                let mut y = 0.0f64;
                for k in 0..taps.min(t + 1) {
                    y += ir[ch][k] as f64 * input[t - k][ch] as f64;
                }
                err += (y - out[t][ch] as f64).powi(2);
                sig += y * y;
            }
        }
        let rel = 10.0 * (err / sig).log10();
        assert!(rel < -100.0, "{taps} taps: error {rel:.1} dB");
    }
}

// ------------------------------------------------------------- distortion

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

/// Worst non-harmonic bin in 20 Hz..20 kHz, in dB below the strongest component.
fn worst_alias(x: &[f32], f0: f64) -> f64 {
    let s = spectrum(x);
    let bin = SR as f64 / x.len() as f64;
    let top = s.iter().fold(0.0f32, |m, &v| m.max(v)) as f64;
    let mut worst = -300.0f64;
    for (k, &m) in s.iter().enumerate() {
        let hz = k as f64 * bin;
        if !(20.0..=20_000.0).contains(&hz) {
            continue;
        }
        let h = (hz / f0).round();
        if (hz - h * f0).abs() <= 8.0 * bin {
            continue;
        }
        worst = worst.max(db(m as f64 / top));
    }
    worst
}

#[test]
fn distortion_aliasing_stays_70_db_down() {
    let f0 = 1234.5;
    let smooth = [distortion::TUBE, distortion::SOFT_CLIP, distortion::TAPE, distortion::OVERDRIVE, distortion::SINE_SHAPER];
    for mode in 0..15u8 {
        if mode == distortion::DOWNSAMPLE {
            continue; // aliasing is the point
        }
        let mut r = Rig::new();
        r.chain(MAIN, &[entry(DISTORTION, 0)]);
        r.set(p::FX_DISTORTION_MODE[0], mode as f32).set(p::FX_DISTORTION_DRIVE[0], 0.5).set(p::FX_DISTORTION_MIX[0], 1.0);
        let out = r.run(4800 + (1 << 16), sine(f0, 0.5));
        let x = &left(&out)[4800..];
        let w = worst_alias(x, f0);
        // the same curve with no oversampling, for comparison
        let naive: Vec<f32> = (0..x.len())
            .map(|i| {
                let v = 0.5 * (std::f64::consts::TAU * f0 * i as f64 / SR as f64).sin() as f32;
                distortion::curve(mode, v * util::db(18.0))
            })
            .collect();
        let wn = worst_alias(&naive, f0);
        if smooth.contains(&mode) {
            assert!(w <= -70.0, "mode {mode}: worst alias {w:.1} dBc (without oversampling {wn:.1})");
        } else {
            // hard-edged curves alias at any finite rate; oversampling must still buy a clear margin
            assert!(w < wn - 10.0 || w <= -70.0, "mode {mode}: oversampling only reached {w:.1} dBc (without: {wn:.1})");
        }
    }
}

// --------------------------------------------------------------- denormals

#[test]
fn silence_after_an_impulse_leaves_no_subnormals() {
    let mut r = Rig::new();
    r.chain(MAIN, &[entry(DISTORTION, 0), entry(FLANGER, 0), entry(PHASER, 0), entry(CHORUS, 0), entry(DELAY, 0), entry(REVERB, 0), entry(FILTER, 0), entry(BODE, 0), entry(COMPRESSOR, 0)]);
    r.set(p::FX_DISTORTION_FILTER[0], 2.0).set(p::FX_FLANGER_FEEDBACK[0], 0.9).set(p::FX_PHASER_FEEDBACK[0], 0.9).set(p::FX_CHORUS_FEEDBACK[0], 0.9);
    r.set(p::FX_DELAY_FEEDBACK[0], 0.8).set(p::FX_FILTER_TYPE[0], 43.0).set(p::FX_FILTER_RES[0], 0.95).set(p::FX_BODE_FEEDBACK[0], 0.9);
    r.set(p::FX_REVERB_DECAY[0], 1.0);
    r.run((60.0 * SR) as usize, |i| if i == 0 { [1.0, 1.0] } else { [0.0, 0.0] });
    assert!(!r.fx.has_subnormal(), "an effect kept subnormal state after 60 s of silence");
}

// ------------------------------------------------------- bypass and reorder

#[test]
fn bypass_and_reorder_never_click() {
    // a low sine through delay → utility; switch the utility, then reorder, mid-signal
    let mut r = Rig::new();
    r.chain(MAIN, &[entry(DELAY, 0), entry(UTILITY, 0)]);
    r.set(p::FX_DELAY_BPM[0], 0.0).set(p::FX_DELAY_TIME_L[0], 20.0).set(p::FX_DELAY_FEEDBACK[0], 0.5).set(p::FX_DELAY_MIX[0], 0.5);
    r.set(p::FX_UTILITY_GAIN[0], 12.0);
    let mut out = r.run(9600, sine(50.0, 0.3));
    r.set(p::FX_UTILITY_ENABLE[0], 0.0);
    out.extend(r.run(9600, sine(50.0, 0.3)));
    r.set(p::FX_UTILITY_ENABLE[0], 1.0);
    out.extend(r.run(9600, sine(50.0, 0.3)));
    r.chain(MAIN, &[entry(UTILITY, 0), entry(DELAY, 0)]);
    out.extend(r.run(9600, sine(50.0, 0.3)));
    // the signal itself moves at most about 2π·50/48000·(peak) per sample; allow headroom for the fades
    let (at, worst) = out.windows(2).map(|w| (w[1][0] - w[0][0]).abs()).enumerate().fold((0, 0.0f32), |m, (i, d)| if d > m.1 { (i, d) } else { m });
    assert!(worst < 0.03, "a sample-to-sample jump of {worst} at sample {at}");
}

// ----------------------------------------------------------------- smoke

#[test]
fn every_module_stays_finite_on_loud_noise() {
    let mut rng = wt_dsp::rng::Rng::new(3);
    for ty in 0..TYPES as u8 {
        let mut r = Rig::new();
        r.chain(MAIN, &[entry(ty, 2)]);
        let out = r.run(2 * SR as usize, |_| [rng.next_f32() * 2.0 - 1.0, rng.next_f32() * 2.0 - 1.0]);
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s[0].abs()).max(s[1].abs()));
        assert!(peak.is_finite() && peak < 100.0, "type {ty}: peak {peak}");
    }
}

