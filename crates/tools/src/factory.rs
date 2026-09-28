//! birdsynth's factory wavetables, generated on demand. Each is procedural:
//! additive spectra (through an inverse FFT) or naive shapes that the mip
//! builder band-limits. Every frame has its DC removed and is normalized to
//! a 0.99 peak, so sweeping the position keeps a steady level.

use std::f64::consts::TAU;

use realfft::RealFftPlanner;
use realfft::num_complex::Complex;
use wt_dsp::mip::FRAME_LEN;
use wt_dsp::rng::Rng;

pub struct Table {
    pub name: &'static str,
    pub frames: usize,
    generate: fn(&mut Ctx, f64, usize, &mut [f32]),
}

pub const TABLES: &[Table] = &[
    Table { name: "Basic Shapes", frames: 64, generate: basic_shapes },
    Table { name: "Saw", frames: 1, generate: saw },
    Table { name: "Square", frames: 1, generate: square },
    Table { name: "Triangle", frames: 1, generate: triangle },
    Table { name: "Sine", frames: 1, generate: sine },
    Table { name: "PWM", frames: 64, generate: pwm },
    Table { name: "Harmonic Build", frames: 64, generate: harmonic_build },
    Table { name: "Odd Build", frames: 64, generate: odd_build },
    Table { name: "Vowels", frames: 64, generate: vowels },
    Table { name: "Sync Sweep", frames: 64, generate: sync_sweep },
    Table { name: "FM Index", frames: 64, generate: fm_index },
    Table { name: "FM Ratio", frames: 64, generate: fm_ratio },
    Table { name: "Wavefold", frames: 64, generate: wavefold },
    Table { name: "Tanh Drive", frames: 64, generate: tanh_drive },
    Table { name: "Phase Dist", frames: 64, generate: phase_dist },
    Table { name: "Bitcrush", frames: 64, generate: bitcrush },
    Table { name: "Organ", frames: 32, generate: organ },
    Table { name: "Formant Sweep", frames: 64, generate: formant_sweep },
    Table { name: "Spectral Noise", frames: 32, generate: spectral_noise },
    Table { name: "Skewed Tri", frames: 64, generate: skewed_tri },
    Table { name: "Growl", frames: 64, generate: growl },
];

/// Scratch for additive generation: an inverse FFT of the frame size.
pub struct Ctx {
    inv: std::sync::Arc<dyn realfft::ComplexToReal<f32>>,
    spec: Vec<Complex<f32>>,
}

impl Default for Ctx {
    fn default() -> Self {
        let inv = RealFftPlanner::<f32>::new().plan_fft_inverse(FRAME_LEN);
        let spec = inv.make_input_vec();
        Ctx { inv, spec }
    }
}

impl Ctx {
    /// Fill `out` from sine partials: `amp(h)` for h = 1..=1024, all in sine phase.
    fn additive(&mut self, out: &mut [f32], amp: impl Fn(usize) -> f64) {
        self.additive_phased(out, |h| (amp(h), 0.0));
    }

    /// Partials with explicit phase offsets (radians, relative to a sine).
    fn additive_phased(&mut self, out: &mut [f32], partial: impl Fn(usize) -> (f64, f64)) {
        for c in self.spec.iter_mut() {
            *c = Complex::new(0.0, 0.0);
        }
        for h in 1..FRAME_LEN / 2 {
            let (a, ph) = partial(h);
            if a == 0.0 {
                continue;
            }
            // a·sin(hx + ph) is bin h = (a/2)·(sin ph - i cos ph)·N in an unnormalized inverse
            let half = 0.5 * a;
            self.spec[h] = Complex::new((half * ph.sin()) as f32, (-half * ph.cos()) as f32);
        }
        self.inv.process(&mut self.spec, out).expect("inverse fft");
    }
}

/// Generate table `index` into `out` (frames × FRAME_LEN). Returns the frame count.
pub fn build(index: usize, out: &mut [f32]) -> usize {
    let t = &TABLES[index];
    assert!(out.len() >= t.frames * FRAME_LEN);
    let mut cx = Ctx::default();
    for f in 0..t.frames {
        let pos = if t.frames > 1 { f as f64 / (t.frames - 1) as f64 } else { 0.0 };
        let frame = &mut out[f * FRAME_LEN..(f + 1) * FRAME_LEN];
        (t.generate)(&mut cx, pos, f, frame);
        finish(frame);
    }
    t.frames
}

/// Remove DC and normalize to a 0.99 peak.
fn finish(frame: &mut [f32]) {
    let mean = frame.iter().map(|&v| v as f64).sum::<f64>() / frame.len() as f64;
    let mut peak = 0.0f64;
    for v in frame.iter_mut() {
        *v = (*v as f64 - mean) as f32;
        peak = peak.max((*v as f64).abs());
    }
    if peak > 1e-9 {
        let g = 0.99 / peak;
        for v in frame.iter_mut() {
            *v = (*v as f64 * g) as f32;
        }
    }
}

/// Phase of sample i, 0..1.
#[inline]
fn ph(i: usize) -> f64 {
    i as f64 / FRAME_LEN as f64
}

fn naive(out: &mut [f32], f: impl Fn(f64) -> f64) {
    for (i, v) in out.iter_mut().enumerate() {
        *v = f(ph(i)) as f32;
    }
}

fn saw_at(p: f64) -> f64 {
    // rising, crossing zero at phase 0 like the built-in saw
    let q = p + 0.5;
    2.0 * (q - q.floor()) - 1.0
}

fn tri_at(p: f64) -> f64 {
    let q = (p + 0.25) % 1.0;
    if q < 0.5 { 4.0 * q - 1.0 } else { 3.0 - 4.0 * q }
}

fn square_at(p: f64) -> f64 {
    if p < 0.5 { 1.0 } else { -1.0 }
}

fn basic_shapes(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let shapes: [fn(f64) -> f64; 4] = [|p| (TAU * p).sin(), tri_at, saw_at, square_at];
    let x = t * 3.0;
    let k = (x.floor() as usize).min(2);
    let w = x - k as f64;
    naive(out, |p| shapes[k](p) * (1.0 - w) + shapes[k + 1](p) * w);
}

fn saw(cx: &mut Ctx, _: f64, _: usize, out: &mut [f32]) {
    cx.additive(out, |h| if h % 2 == 1 { 1.0 / h as f64 } else { -1.0 / h as f64 });
}

fn square(cx: &mut Ctx, _: f64, _: usize, out: &mut [f32]) {
    cx.additive(out, |h| if h % 2 == 1 { 1.0 / h as f64 } else { 0.0 });
}

fn triangle(_: &mut Ctx, _: f64, _: usize, out: &mut [f32]) {
    naive(out, tri_at);
}

fn sine(_: &mut Ctx, _: f64, _: usize, out: &mut [f32]) {
    naive(out, |p| (TAU * p).sin());
}

fn pwm(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let d = 0.5 - 0.48 * t;
    naive(out, |p| if p < d { 1.0 } else { -1.0 });
}

fn harmonic_build(cx: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let n = 1.0 + 127.0 * t * t;
    cx.additive(out, |h| {
        let w = (n - (h as f64 - 1.0)).clamp(0.0, 1.0); // the newest partial fades in
        let s = if h % 2 == 1 { 1.0 } else { -1.0 };
        w * s / h as f64
    });
}

fn odd_build(cx: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let n = 1.0 + 63.0 * t * t; // count of odd partials
    cx.additive(out, |h| {
        if h % 2 == 0 {
            return 0.0;
        }
        let k = (h / 2) as f64; // 0 for h=1
        (n - k).clamp(0.0, 1.0) / h as f64
    });
}

/// Formant amplitudes over the harmonics of a nominal 110 Hz note.
fn formants(h: usize, f: [f64; 3]) -> f64 {
    let hz = 110.0 * h as f64;
    let bw = [90.0, 110.0, 160.0];
    let gain = [1.0, 0.5, 0.25];
    let mut a = 0.0;
    for i in 0..3 {
        let d = (hz - f[i]) / bw[i];
        a += gain[i] * (-d * d).exp();
    }
    a + 0.02 / h as f64
}

const VOWELS: [[f64; 3]; 5] = [
    [800.0, 1150.0, 2900.0], // A
    [400.0, 1600.0, 2700.0], // E
    [350.0, 2300.0, 3000.0], // I
    [450.0, 800.0, 2830.0],  // O
    [325.0, 700.0, 2530.0],  // U
];

fn vowel_at(t: f64) -> [f64; 3] {
    let x = t * 4.0;
    let k = (x.floor() as usize).min(3);
    let w = x - k as f64;
    let (a, b) = (VOWELS[k], VOWELS[k + 1]);
    [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w, a[2] + (b[2] - a[2]) * w]
}

fn vowels(cx: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let f = vowel_at(t);
    cx.additive(out, |h| if h <= 200 { formants(h, f) } else { 0.0 });
}

fn sync_sweep(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let r = 1.0 + 7.0 * t;
    naive(out, |p| saw_at(p * r - 0.5 * (r - 1.0).min(0.0)));
}

fn fm_index(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let i = 8.0 * t;
    naive(out, |p| (TAU * p + i * (TAU * 2.0 * p).sin()).sin());
}

fn fm_ratio(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    // integer ratios keep each cycle periodic; neighbours are crossfaded
    let x = 1.0 + 7.0 * t;
    let lo = x.floor().min(7.0);
    let w = x - lo;
    naive(out, |p| {
        let a = (TAU * p + 2.5 * (TAU * lo * p).sin()).sin();
        let b = (TAU * p + 2.5 * (TAU * (lo + 1.0) * p).sin()).sin();
        a * (1.0 - w) + b * w
    });
}

fn wavefold(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let g = 1.0 + 7.0 * t;
    naive(out, |p| (std::f64::consts::FRAC_PI_2 * g * (TAU * p).sin()).sin());
}

fn tanh_drive(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let g = 1.0 + 29.0 * t * t;
    naive(out, |p| (g * (TAU * p).sin()).tanh());
}

fn phase_dist(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let r = 1.0 + 15.0 * t;
    naive(out, |p| (1.0 - p) * (TAU * p * r).cos() - 0.5 * (1.0 - p));
}

fn bitcrush(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let levels = 2f64.powf(8.0 - 6.0 * t);
    naive(out, |p| (saw_at(p) * levels * 0.5).round() / (levels * 0.5));
}

fn organ(cx: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    // drawbars 16' 8' 5 1/3' 4' 2 2/3' 2' 1 3/5' 1 1/3' 1', with the table's fundamental as 16'
    const H: [usize; 9] = [1, 2, 3, 4, 6, 8, 10, 12, 16];
    const REG: [[f64; 9]; 4] = [
        [8.0, 8.0, 8.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
        [8.0, 0.0, 8.0, 8.0, 0.0, 8.0, 0.0, 0.0, 8.0],
        [8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0, 8.0],
        [8.0, 0.0, 0.0, 0.0, 0.0, 0.0, 8.0, 8.0, 8.0],
    ];
    let x = t * 3.0;
    let k = (x.floor() as usize).min(2);
    let w = x - k as f64;
    cx.additive(out, |h| match H.iter().position(|&v| v == h) {
        Some(i) => (REG[k][i] * (1.0 - w) + REG[k + 1][i] * w) / 8.0,
        None => 0.0,
    });
}

fn formant_sweep(cx: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let centre = 1.0 + 63.0 * t;
    cx.additive(out, |h| {
        let d = (h as f64 - centre) / 2.5;
        let peak = (-d * d).exp();
        let base = if h == 1 { 0.6 } else { 0.0 };
        base + peak / (1.0 + 0.05 * h as f64)
    });
}

fn spectral_noise(cx: &mut Ctx, _: f64, frame: usize, out: &mut [f32]) {
    let mut rng = Rng::new(0xB1D5 + frame as u32 * 7919);
    let amps: Vec<(f64, f64)> = (0..=256)
        .map(|h| {
            if h == 0 {
                (0.0, 0.0)
            } else {
                let a = rng.next_f32() as f64 / (h as f64).sqrt();
                let p = rng.next_f32() as f64 * TAU;
                (a, p)
            }
        })
        .collect();
    cx.additive_phased(out, |h| if h <= 256 { amps[h] } else { (0.0, 0.0) });
}

fn skewed_tri(_: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    let a = 0.5 - 0.49 * t;
    naive(out, |p| if p < a { -1.0 + 2.0 * p / a } else { 1.0 - 2.0 * (p - a) / (1.0 - a) });
}

fn growl(cx: &mut Ctx, t: f64, _: usize, out: &mut [f32]) {
    // an A -> O formant sweep whose partials are pushed around by FM sidebands
    let f = {
        let (a, b) = (VOWELS[0], VOWELS[3]);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    };
    let depth = 0.8 * t;
    cx.additive(out, |h| {
        if h > 200 {
            return 0.0;
        }
        let side = if h > 2 { formants(h - 2, f) * depth } else { 0.0 };
        formants(h, f) * (1.0 - 0.5 * depth) + side
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_builds_finite_normalized_frames() {
        for (i, t) in TABLES.iter().enumerate() {
            let mut out = vec![0.0f32; t.frames * FRAME_LEN];
            assert_eq!(build(i, &mut out), t.frames);
            for f in 0..t.frames {
                let fr = &out[f * FRAME_LEN..(f + 1) * FRAME_LEN];
                let peak = fr.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                let mean = fr.iter().sum::<f32>() / FRAME_LEN as f32;
                assert!(fr.iter().all(|v| v.is_finite()), "{} frame {f}", t.name);
                assert!((peak - 0.99).abs() < 1e-3, "{} frame {f} peak {peak}", t.name);
                assert!(mean.abs() < 1e-3, "{} frame {f} dc {mean}", t.name);
            }
        }
        assert_eq!(TABLES.len(), 21);
    }

    #[test]
    fn additive_saw_matches_the_naive_one_in_shape() {
        let mut a = vec![0.0f32; FRAME_LEN];
        saw(&mut Ctx::default(), 0.0, 0, &mut a);
        finish(&mut a);
        // rising through zero at phase 0 and at the middle of each ramp
        assert!(a[0].abs() < 0.01 && a[10] > 0.0 && a[FRAME_LEN - 10] < 0.0);
    }
}
