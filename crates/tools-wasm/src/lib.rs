//! The tools C ABI, compiled to wasm32-unknown-unknown as tools.wasm for the
//! tools Web Worker (mip building, factory tables, resampling, previews).
//! Like engine.wasm it imports nothing. The host allocates buffers with
//! `tl_alloc`, fills them, calls a function, reads results, and frees them.

use std::alloc::{Layout, alloc_zeroed, dealloc};
use std::cell::UnsafeCell;

use wt_dsp::mip::{FRAME_LEN, FRAME_STRIDE};
use wt_tools::{factory, mips::MipBuilder, preview, resample};

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
///
/// # Safety
/// `frame` holds 2048 floats; `dst` holds `points`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tl_preview(frame: *const f32, w1_mode: u32, w1_amt: f32, w2_mode: u32, w2_amt: f32, points: u32, dst: *mut f32) -> u32 {
    let (f, d) = unsafe { (slice(frame, FRAME_LEN), slice_mut(dst, points as usize)) };
    preview::cycle(f, (w1_mode as u8, w1_amt), (w2_mode as u8, w2_amt), d);
    0
}
