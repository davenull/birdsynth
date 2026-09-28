//! The oscillator inner loop: every unison lane of one oscillator of one
//! voice, for one sub-block.
//!
//! `render_scalar` is the reference. `render_simd` runs four lanes per
//! vector and is written to perform the same floating-point operations in
//! the same order, including adding lanes into the output one at a time,
//! so the two agree bit for bit (tested below). Wasm has no gather, so table
//! loads stay scalar; everything around them is vectorized.

use wide::bytemuck;
use wide::{f32x4, i32x4, u32x4};
use wt_dsp::mip::{FRAME_STRIDE, LEVEL_BITS, LEVEL_OFFSET, Pick, level_len, read_pick};
use wt_dsp::warp;

use super::unison::MAX_LANES;
use crate::spec::protocol::SUB_BLOCK as N;

const INV_2_24: f32 = 1.0 / 16_777_216.0;
/// Lane increments are capped just under Nyquist (cycles per sample).
const MAX_INC: f32 = 0.4999;

pub struct Kernel<'a> {
    /// frames × FRAME_STRIDE floats.
    pub table: &'a [f32],
    pub frames: usize,
    pub pick: Pick,
    pub lanes: usize,
    /// Base increment (cycles per sample) at sample 0 of the sub-block, and
    /// the per-sample factor of an exponential pitch ramp.
    pub inc0: f32,
    pub inc_step: f32,
    pub ratio: [f32; MAX_LANES],
    pub gl: [f32; MAX_LANES],
    pub gr: [f32; MAX_LANES],
    /// Frame position (0..frames-1) per lane at sample 0, and per sample.
    pub fpos0: [f32; MAX_LANES],
    pub fpos_step: [f32; MAX_LANES],
    /// Blend neighbouring frames (else snap to the nearest).
    pub smooth: bool,
    pub w1_mode: u8,
    pub w1_amt: [f32; MAX_LANES],
    pub w2_mode: u8,
    pub w2_amt: [f32; MAX_LANES],
    /// True when either warp changes the phase (then phases go through f32).
    pub warped: bool,
}

impl Kernel<'_> {
    #[inline(always)]
    fn frame(&self, i: usize) -> &[f32] {
        &self.table[i * FRAME_STRIDE..(i + 1) * FRAME_STRIDE]
    }
}

// ------------------------------------------------------------------ scalar

#[inline(always)]
fn level_read_f(frame: &[f32], level: usize, q: f32) -> f32 {
    let x = q * level_len(level) as f32;
    let i = x as i32;
    let t = x - i as f32;
    let o = LEVEL_OFFSET[level] + i as usize;
    let a = frame[o];
    let b = frame[o + 1];
    a + (b - a) * t
}

#[inline(always)]
fn pick_read_f(frame: &[f32], p: Pick, q: f32) -> f32 {
    let a = level_read_f(frame, p.lo, q);
    if p.w > 0.0 { a + (level_read_f(frame, p.hi, q) - a) * p.w } else { a }
}

/// Reference kernel: adds this oscillator into `out_l`/`out_r` from sample `start`.
pub fn render_scalar(k: &Kernel, phase: &mut [u32; MAX_LANES], start: usize, out_l: &mut [f32; N], out_r: &mut [f32; N]) {
    let last = (k.frames - 1) as f32;
    let single = k.frames == 1;
    let mut inc_base = k.inc0;
    for _ in 0..start {
        inc_base *= k.inc_step;
    }
    for i in start..N {
        let fi = i as f32;
        for l in 0..k.lanes {
            let inc_c = (inc_base * k.ratio[l]).min(MAX_INC);
            let inc = (inc_c * 4_294_967_296.0) as i32 as u32;
            let ph = phase[l];
            phase[l] = ph.wrapping_add(inc);

            let f = (k.fpos0[l] + k.fpos_step[l] * fi).clamp(0.0, last);
            let (fr0, t) = if k.smooth {
                let i0 = f as i32;
                (i0, f - i0 as f32)
            } else {
                ((f + 0.5) as i32, 0.0)
            };
            let i0 = (fr0 as usize).min(k.frames - 1);
            let i1 = (i0 + 1).min(k.frames - 1);

            let (s, g) = if k.warped {
                let p = (ph >> 8) as i32 as f32 * INV_2_24;
                let (p1, g1) = warp::apply(k.w1_mode, p, k.w1_amt[l]);
                let (p2, g2) = warp::apply(k.w2_mode, warp::wrap01(p1), k.w2_amt[l]);
                let q = warp::wrap01(p2);
                let a = pick_read_f(k.frame(i0), k.pick, q);
                let s = if single { a } else { a + (pick_read_f(k.frame(i1), k.pick, q) - a) * t };
                (s, g1 * g2)
            } else {
                let a = read_pick(k.frame(i0), k.pick, ph);
                let s = if single { a } else { a + (read_pick(k.frame(i1), k.pick, ph) - a) * t };
                (s, 1.0)
            };
            let y = s * g;
            out_l[i] += y * k.gl[l];
            out_r[i] += y * k.gr[l];
        }
        inc_base *= k.inc_step;
    }
}

// -------------------------------------------------------------------- simd

#[inline(always)]
fn u2i(v: u32x4) -> i32x4 {
    bytemuck::cast(v)
}

#[inline(always)]
fn i2u(v: i32x4) -> u32x4 {
    bytemuck::cast(v)
}

#[inline(always)]
fn sel(mask: f32x4, t: f32x4, f: f32x4) -> f32x4 {
    mask.select(t, f)
}

#[inline(always)]
fn wrap01_4(p: f32x4) -> f32x4 {
    let q = p - p.floor();
    sel(q.simd_ge(f32x4::ONE), f32x4::ZERO, q)
}

#[inline(always)]
fn bend_plus4(p: f32x4, k: f32x4) -> f32x4 {
    p * (f32x4::ONE + k) / (f32x4::ONE + k * p)
}

#[inline(always)]
fn bend_minus4(p: f32x4, k: f32x4) -> f32x4 {
    p / (f32x4::ONE + k * (f32x4::ONE - p))
}

#[inline(always)]
fn asym4(p: f32x4, s: f32x4) -> f32x4 {
    let half = f32x4::splat(0.5);
    sel(p.simd_lt(s), half * p / s, half + half * (p - s) / (f32x4::ONE - s))
}

/// Vector twin of `wt_dsp::warp::apply`, formula for formula.
#[inline(always)]
fn warp4(mode: u8, p: f32x4, a: f32x4) -> (f32x4, f32x4) {
    if mode == warp::OFF || mode > warp::LAST_IMPLEMENTED {
        return (p, f32x4::ONE);
    }
    let half = f32x4::splat(0.5);
    let two = f32x4::splat(2.0);
    let (q, g) = match mode {
        warp::SYNC => {
            let x = p * (f32x4::ONE + a * f32x4::splat(15.0));
            (x - x.floor(), f32x4::ONE)
        }
        warp::WINDOW_SYNC => {
            let x = p * (f32x4::ONE + a * f32x4::splat(15.0));
            let s = (a * f32x4::splat(8.0)).min(f32x4::ONE);
            let y = f32x4::splat(4.0) * p * (f32x4::ONE - p);
            let v = y * (f32x4::splat(0.775) + f32x4::splat(0.225) * y);
            let w = v * v;
            (x - x.floor(), f32x4::ONE - s + s * w)
        }
        warp::BEND_PLUS => (bend_plus4(p, a * f32x4::splat(8.0)), f32x4::ONE),
        warp::BEND_MINUS => (bend_minus4(p, a * f32x4::splat(8.0)), f32x4::ONE),
        warp::BEND_BOTH => {
            let k = a * f32x4::splat(8.0);
            let lo = half * bend_minus4(two * p, k);
            let hi = f32x4::ONE - half * bend_minus4(two - two * p, k);
            (sel(p.simd_lt(half), lo, hi), f32x4::ONE)
        }
        warp::PWM => {
            let w = f32x4::ONE - a * f32x4::splat(0.98);
            (sel(p.simd_lt(w), p / w, f32x4::ZERO), f32x4::ONE)
        }
        warp::ASYM_PLUS => (asym4(p, half - f32x4::splat(0.49) * a), f32x4::ONE),
        warp::ASYM_MINUS => (asym4(p, half + f32x4::splat(0.49) * a), f32x4::ONE),
        warp::ASYM_BOTH => {
            let s = half - f32x4::splat(0.49) * a;
            let lo = half * asym4(two * p, s);
            let hi = half + half * asym4(two * p - f32x4::ONE, f32x4::ONE - s);
            (sel(p.simd_lt(half), lo, hi), f32x4::ONE)
        }
        _ => (p, f32x4::ONE),
    };
    // lanes with no amount pass through, exactly like the scalar `active` check
    let on = a.simd_gt(f32x4::ZERO);
    (sel(on, q, p), sel(on, g, f32x4::ONE))
}

/// Four lanes of a u32-phase level read.
#[inline(always)]
fn level_read_u4(frame: &[f32], level: usize, ph: u32x4) -> f32x4 {
    let bits = LEVEL_BITS[level];
    let idx = (ph >> (32 - bits)).to_array();
    let frac = f32x4::from_i32x4(u2i((ph << bits) >> 8)) * f32x4::splat(INV_2_24);
    let base = LEVEL_OFFSET[level];
    let mut a = [0.0f32; 4];
    let mut b = [0.0f32; 4];
    for j in 0..4 {
        let o = base + idx[j] as usize;
        a[j] = frame[o];
        b[j] = frame[o + 1];
    }
    let (a, b) = (f32x4::new(a), f32x4::new(b));
    a + (b - a) * frac
}

/// Four lanes of an f32-phase level read.
#[inline(always)]
fn level_read_f4(frame: &[f32], level: usize, q: f32x4) -> f32x4 {
    let x = q * f32x4::splat(level_len(level) as f32);
    let i = x.trunc_int();
    let t = x - f32x4::from_i32x4(i);
    let idx = i.to_array();
    let base = LEVEL_OFFSET[level];
    let mut a = [0.0f32; 4];
    let mut b = [0.0f32; 4];
    for j in 0..4 {
        let o = base + idx[j] as usize;
        a[j] = frame[o];
        b[j] = frame[o + 1];
    }
    let (a, b) = (f32x4::new(a), f32x4::new(b));
    a + (b - a) * t
}

/// Read four lanes from per-lane frames (each lane can sit on a different frame).
#[inline(always)]
fn gather_frames<F: Fn(&[f32], usize) -> f32>(k: &Kernel, fr: [i32; 4], read: F, lanes: usize) -> f32x4 {
    let mut v = [0.0f32; 4];
    for j in 0..lanes.min(4) {
        v[j] = read(k.frame(fr[j] as usize), j);
    }
    f32x4::new(v)
}

/// SIMD kernel: same results as `render_scalar`, four lanes per vector.
pub fn render_simd(k: &Kernel, phase: &mut [u32; MAX_LANES], start: usize, out_l: &mut [f32; N], out_r: &mut [f32; N]) {
    let groups = k.lanes.div_ceil(4);
    let last = f32x4::splat((k.frames - 1) as f32);
    let last_i = i32x4::splat((k.frames - 1) as i32);
    let single = k.frames == 1;
    let v = |a: &[f32; MAX_LANES], g: usize| f32x4::new([a[g * 4], a[g * 4 + 1], a[g * 4 + 2], a[g * 4 + 3]]);
    let mut ratio = [f32x4::ZERO; 4];
    let mut gl = [f32x4::ZERO; 4];
    let mut gr = [f32x4::ZERO; 4];
    let mut fpos0 = [f32x4::ZERO; 4];
    let mut fstep = [f32x4::ZERO; 4];
    let mut w1 = [f32x4::ZERO; 4];
    let mut w2 = [f32x4::ZERO; 4];
    let mut ph = [u32x4::ZERO; 4];
    for g in 0..groups {
        ratio[g] = v(&k.ratio, g);
        gl[g] = v(&k.gl, g);
        gr[g] = v(&k.gr, g);
        fpos0[g] = v(&k.fpos0, g);
        fstep[g] = v(&k.fpos_step, g);
        w1[g] = v(&k.w1_amt, g);
        w2[g] = v(&k.w2_amt, g);
        ph[g] = u32x4::new([phase[g * 4], phase[g * 4 + 1], phase[g * 4 + 2], phase[g * 4 + 3]]);
    }
    let mut inc_base = k.inc0;
    for _ in 0..start {
        inc_base *= k.inc_step;
    }
    for i in start..N {
        let fi = f32x4::splat(i as f32);
        let ib = f32x4::splat(inc_base);
        for g in 0..groups {
            let lanes = (k.lanes - g * 4).min(4);
            let inc_c = (ib * ratio[g]).min(f32x4::splat(MAX_INC));
            let inc = i2u((inc_c * f32x4::splat(4_294_967_296.0)).trunc_int());
            let p = ph[g];
            ph[g] = p + inc;

            let f = (fpos0[g] + fstep[g] * fi).max(f32x4::ZERO).min(last);
            let (fr0, t) = if k.smooth {
                let i0 = f.trunc_int();
                (i0, f - f32x4::from_i32x4(i0))
            } else {
                ((f + f32x4::splat(0.5)).trunc_int(), f32x4::ZERO)
            };
            let i0 = fr0.min(last_i);
            let i1 = (i0 + i32x4::ONE).min(last_i);
            let (i0a, i1a) = (i0.to_array(), i1.to_array());

            let (s, gain) = if k.warped {
                let pf = f32x4::from_i32x4(u2i(p >> 8)) * f32x4::splat(INV_2_24);
                let (p1, g1) = warp4(k.w1_mode, pf, w1[g]);
                let (p2, g2) = warp4(k.w2_mode, wrap01_4(p1), w2[g]);
                let q = wrap01_4(p2);
                let qa = q.to_array();
                let pick = k.pick;
                let read = |frame: &[f32], j: usize| pick_read_f(frame, pick, qa[j]);
                // f32 reads are gathered per lane but interpolated as vectors
                let a = if uniform(&i0a, lanes) {
                    pick_read_f4(k.frame(i0a[0] as usize), pick, q)
                } else {
                    gather_frames(k, i0a, read, lanes)
                };
                let s = if single {
                    a
                } else {
                    let b = if uniform(&i1a, lanes) { pick_read_f4(k.frame(i1a[0] as usize), pick, q) } else { gather_frames(k, i1a, read, lanes) };
                    a + (b - a) * t
                };
                (s, g1 * g2)
            } else {
                let pick = k.pick;
                let pa = p.to_array();
                let read = |frame: &[f32], j: usize| read_pick(frame, pick, pa[j]);
                let a = if uniform(&i0a, lanes) { pick_read_u4(k.frame(i0a[0] as usize), pick, p) } else { gather_frames(k, i0a, read, lanes) };
                let s = if single {
                    a
                } else {
                    let b = if uniform(&i1a, lanes) { pick_read_u4(k.frame(i1a[0] as usize), pick, p) } else { gather_frames(k, i1a, read, lanes) };
                    a + (b - a) * t
                };
                (s, f32x4::ONE)
            };
            let y = s * gain;
            let yl = (y * gl[g]).to_array();
            let yr = (y * gr[g]).to_array();
            for j in 0..lanes {
                out_l[i] += yl[j];
                out_r[i] += yr[j];
            }
        }
        inc_base *= k.inc_step;
    }
    for g in 0..groups {
        let a = ph[g].to_array();
        let lanes = (k.lanes - g * 4).min(4);
        phase[g * 4..g * 4 + lanes].copy_from_slice(&a[..lanes]);
    }
}

#[inline(always)]
fn uniform(idx: &[i32; 4], lanes: usize) -> bool {
    idx[1..lanes].iter().all(|&v| v == idx[0])
}

#[inline(always)]
fn pick_read_u4(frame: &[f32], p: Pick, ph: u32x4) -> f32x4 {
    let a = level_read_u4(frame, p.lo, ph);
    if p.w > 0.0 { a + (level_read_u4(frame, p.hi, ph) - a) * f32x4::splat(p.w) } else { a }
}

#[inline(always)]
fn pick_read_f4(frame: &[f32], p: Pick, q: f32x4) -> f32x4 {
    let a = level_read_f4(frame, p.lo, q);
    if p.w > 0.0 { a + (level_read_f4(frame, p.hi, q) - a) * f32x4::splat(p.w) } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wt_dsp::mip;
    use wt_dsp::rng::Rng;

    fn table(frames: usize) -> Vec<f32> {
        let mut rng = Rng::new(5);
        (0..frames * FRAME_STRIDE).map(|_| rng.next_f32() * 2.0 - 1.0).collect()
    }

    fn random_kernel<'a>(t: &'a [f32], frames: usize, rng: &mut Rng) -> Kernel<'a> {
        let lanes = 1 + (rng.next_u32() % 16) as usize;
        let mut k = Kernel {
            table: t,
            frames,
            pick: mip::pick(0.001 + rng.next_f32() * 0.05, 48_000.0),
            lanes,
            inc0: 0.001 + rng.next_f32() * 0.02,
            inc_step: if rng.next_f32() < 0.5 { 1.0 } else { 1.0 + (rng.next_f32() - 0.5) * 0.01 },
            ratio: [1.0; MAX_LANES],
            gl: [0.0; MAX_LANES],
            gr: [0.0; MAX_LANES],
            fpos0: [0.0; MAX_LANES],
            fpos_step: [0.0; MAX_LANES],
            smooth: rng.next_f32() < 0.8,
            w1_mode: (rng.next_u32() % 12) as u8,
            w1_amt: [0.0; MAX_LANES],
            w2_mode: (rng.next_u32() % 12) as u8,
            w2_amt: [0.0; MAX_LANES],
            warped: false,
        };
        for l in 0..MAX_LANES {
            k.ratio[l] = 0.9 + rng.next_f32() * 0.2;
            k.gl[l] = rng.next_f32();
            k.gr[l] = rng.next_f32();
            k.fpos0[l] = rng.next_f32() * (frames - 1) as f32;
            k.fpos_step[l] = (rng.next_f32() - 0.5) * 0.05;
            k.w1_amt[l] = if rng.next_f32() < 0.2 { 0.0 } else { rng.next_f32() };
            k.w2_amt[l] = if rng.next_f32() < 0.2 { 0.0 } else { rng.next_f32() };
        }
        k.warped = (0..lanes).any(|l| warp::active(k.w1_mode, k.w1_amt[l]) || warp::active(k.w2_mode, k.w2_amt[l]));
        k
    }

    #[test]
    fn simd_matches_scalar_bit_for_bit() {
        let mut rng = Rng::new(99);
        for &frames in &[1usize, 2, 7, 64] {
            let t = table(frames);
            for _ in 0..400 {
                let k = random_kernel(&t, frames, &mut rng);
                let mut ph_a = [0u32; MAX_LANES];
                for p in ph_a.iter_mut() {
                    *p = rng.next_u32();
                }
                let mut ph_b = ph_a;
                let start = if rng.next_f32() < 0.3 { (rng.next_u32() % N as u32) as usize } else { 0 };
                let (mut la, mut ra) = ([0.0; N], [0.0; N]);
                let (mut lb, mut rb) = ([0.0; N], [0.0; N]);
                render_scalar(&k, &mut ph_a, start, &mut la, &mut ra);
                render_simd(&k, &mut ph_b, start, &mut lb, &mut rb);
                for i in 0..N {
                    assert_eq!(la[i].to_bits(), lb[i].to_bits(), "L[{i}] frames {frames} lanes {} warped {}", k.lanes, k.warped);
                    assert_eq!(ra[i].to_bits(), rb[i].to_bits(), "R[{i}]");
                }
                assert_eq!(ph_a, ph_b);
            }
        }
    }
}
