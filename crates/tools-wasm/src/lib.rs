//! The tools C ABI, compiled to wasm32-unknown-unknown as tools.wasm for the
//! tools Web Worker (mip building, factory tables, resampling, previews).
//! Like engine.wasm it imports nothing. The host allocates buffers with
//! `tl_alloc`, fills them, calls a function, reads results, and frees them.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::cell::UnsafeCell;

use wt_dsp::mip::{FRAME_LEN, FRAME_STRIDE};
use wt_dsp::filter;
use wt_dsp::warp::{REMAP_POINTS, Remap};
use wt_tools::{factory, ir, mips::MipBuilder, noise, preview, resample};

struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}

static BUILDER: Global<Option<MipBuilder>> = Global(UnsafeCell::new(None));

fn builder() -> &'static mut MipBuilder {
    unsafe { (*BUILDER.0.get()).get_or_insert_with(MipBuilder::default) }
}

unsafe fn slice<'a>(ptr: *const f32, len: usize) -> &'a [f32] {
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

unsafe fn slice_mut<'a>(ptr: *mut f32, len: usize) -> &'a mut [f32] {
    unsafe { std::slice::from_raw_parts_mut(ptr, len) }
}

/// Allocate `bytes` (16-byte aligned, zeroed). Null on failure.
#[unsafe(no_mangle)]
pub extern "C" fn tl_alloc(bytes: u32) -> *mut u8 {
    match Layout::from_size_align(bytes as usize, 16) {
        Ok(l) if l.size() > 0 => unsafe { alloc_zeroed(l) },
        _ => std::ptr::null_mut(),
    }
}

/// # Safety
/// `ptr` must come from `tl_alloc(bytes)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_free(ptr: *mut u8, bytes: u32) {
    if let Ok(l) = Layout::from_size_align(bytes as usize, 16)
        && !ptr.is_null()
    {
        unsafe { dealloc(ptr, l) }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn tl_frame_len() -> u32 {
    FRAME_LEN as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn tl_frame_stride() -> u32 {
    FRAME_STRIDE as u32
}

/// Band-limit `frames` raw frames (frames × 2048) into dst (frames × FRAME_STRIDE).
///
/// # Safety
/// Both buffers must be that large.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_mips(src: *const f32, frames: u32, dst: *mut f32) -> u32 {
    let n = frames as usize;
    let (s, d) = unsafe { (slice(src, n * FRAME_LEN), slice_mut(dst, n * FRAME_STRIDE)) };
    builder().table(s, d);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn tl_factory_count() -> u32 {
    factory::TABLES.len() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn tl_factory_frames(i: u32) -> u32 {
    factory::TABLES.get(i as usize).map_or(0, |t| t.frames as u32)
}

/// Write table `i`'s name (ASCII) into `out` (capacity `cap`); returns its length.
///
/// # Safety
/// `out` must hold `cap` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_factory_name(i: u32, out: *mut u8, cap: u32) -> u32 {
    let Some(t) = factory::TABLES.get(i as usize) else { return 0 };
    let b = t.name.as_bytes();
    let n = b.len().min(cap as usize);
    unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), out, n) };
    n as u32
}

/// Generate factory table `i` into dst (frames × 2048). Returns the frame count.
///
/// # Safety
/// `dst` must hold `tl_factory_frames(i) × 2048` floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_factory_build(i: u32, dst: *mut f32) -> u32 {
    let i = i as usize;
    let Some(t) = factory::TABLES.get(i) else { return 0 };
    let d = unsafe { slice_mut(dst, t.frames * FRAME_LEN) };
    factory::build(i, d) as u32
}

/// Resample one cycle of `len` samples to a 2048-sample frame.
///
/// # Safety
/// `src` holds `len` floats; `dst` holds 2048.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_resample(src: *const f32, len: u32, dst: *mut f32) -> u32 {
    if len < 2 {
        return 1;
    }
    let (s, d) = unsafe { (slice(src, len as usize), slice_mut(dst, FRAME_LEN)) };
    resample::cycle_to_frame(s, d);
    0
}

/// Preview one cycle of a raw frame through two warps into `points` samples.
/// `remap` is the oscillator's remap table (257 floats), or null for the identity.
///
/// # Safety
/// `frame` holds 2048 floats; `dst` holds `points`; a non-null `remap` holds 257.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_preview(frame: *const f32, w1_mode: u32, w1_amt: f32, w2_mode: u32, w2_amt: f32, remap: *const f32, points: u32, dst: *mut f32) -> u32 {
    let (f, d) = unsafe { (slice(frame, FRAME_LEN), slice_mut(dst, points as usize)) };
    let r = if remap.is_null() { Remap::default() } else { Remap::from_values(unsafe { slice(remap, REMAP_POINTS) }) };
    preview::cycle(f, (w1_mode as u8, w1_amt), (w2_mode as u8, w2_amt), &r, d);
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn tl_noise_count() -> u32 {
    noise::NAMES.len() as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn tl_noise_len() -> u32 {
    noise::LEN as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn tl_noise_rate() -> f32 {
    noise::RATE
}

/// Generate noise `i` into dst (tl_noise_len() floats). Returns 0 on success.
///
/// # Safety
/// `dst` must hold tl_noise_len() floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_noise_build(i: u32, dst: *mut f32) -> u32 {
    if i as usize >= noise::NAMES.len() {
        return 1;
    }
    let d = unsafe { slice_mut(dst, noise::LEN) };
    noise::build(i as usize, d);
    0
}

/// A filter's magnitude response in dB at `points` log-spaced frequencies from f0 to f1.
///
/// # Safety
/// `dst` holds `points` floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_filter_response(kind: u32, cutoff: f32, res: f32, var: f32, sr: f32, f0: f32, f1: f32, points: u32, dst: *mut f32) -> u32 {
    let d = unsafe { slice_mut(dst, points as usize) };
    let n = (points.max(2) - 1) as f32;
    for (i, v) in d.iter_mut().enumerate() {
        let f = f0 * (f1 / f0).powf(i as f32 / n);
        *v = 20.0 * filter::response(kind as u8, cutoff, res, var, sr, f).max(1e-6).log10();
    }
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn tl_ir_count() -> u32 {
    ir::NAMES.len() as u32
}

/// Taps of factory response `i` at sample rate `sr`.
#[unsafe(no_mangle)]
pub extern "C" fn tl_ir_taps(i: u32, sr: f32) -> u32 {
    ir::taps(i as usize, sr) as u32
}

/// Build factory response `i` at `sr` into `l` and `r` (tl_ir_taps floats each).
///
/// # Safety
/// `l` and `r` hold tl_ir_taps(i, sr) floats.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_ir_build(i: u32, sr: f32, l: *mut f32, r: *mut f32) -> u32 {
    let n = ir::taps(i as usize, sr);
    let (l, r) = unsafe { (slice_mut(l, n), slice_mut(r, n)) };
    ir::build(i as usize, sr, l, r);
    0
}

/// Floats a prepared response of `taps` takes (both channels).
#[unsafe(no_mangle)]
pub extern "C" fn tl_ir_prepared_len(taps: u32) -> u32 {
    (2 * wt_dsp::conv::channel_len((taps as usize).min(wt_dsp::conv::MAX_TAPS))) as u32
}

/// Prepare a response for the engine's convolver (layout in wt_dsp::conv).
///
/// # Safety
/// `l` and `r` hold `taps` floats; `dst` holds tl_ir_prepared_len(taps).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_ir_prepare(l: *const f32, r: *const f32, taps: u32, dst: *mut f32) -> u32 {
    let taps = (taps as usize).min(wt_dsp::conv::MAX_TAPS);
    let (l, r) = unsafe { (slice(l, taps), slice(r, taps)) };
    let d = unsafe { slice_mut(dst, 2 * wt_dsp::conv::channel_len(taps)) };
    wt_dsp::conv::prepare([l, r], d);
    0
}
