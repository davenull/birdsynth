//! The engine's C ABI, compiled to wasm32-unknown-unknown as engine.wasm.
//!
//! The module imports nothing, so the AudioWorklet can instantiate it
//! with `{}`. The host writes command batches into the buffer at
//! `wt_cmd_ptr`, calls `wt_apply`, then `wt_render`, and reads the output,
//! telemetry and taps straight out of wasm memory.
//!
//! Panics abort as a wasm trap. The panic hook first copies the message into
//! a static buffer (`wt_panic_ptr`/`wt_panic_len`, ASCII) so the host can
//! report it. The AudioWorklet has no TextDecoder, which is why it's ASCII.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

use wt_engine::Engine;
use wt_engine::tables::AssetBuf;
use wt_engine::spec::{params, protocol as proto};

// ------------------------------------------------------------ allocator

/// Counts every allocation, so tests can prove rendering never allocates.
struct Counting;

static ALLOCS: AtomicU32 = AtomicU32::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Relaxed);
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

// --------------------------------------------------------------- globals

/// A global cell. The engine runs on one thread (the audio thread), so
/// plain interior mutability is enough.
struct Global<T>(UnsafeCell<T>);

unsafe impl<T> Sync for Global<T> {}

impl<T> Global<T> {
    const fn new(v: T) -> Self {
        Global(UnsafeCell::new(v))
    }

    #[allow(clippy::mut_from_ref)]
    fn get(&self) -> &mut T {
        unsafe { &mut *self.0.get() }
    }
}

struct PanicBuf {
    len: usize,
    bytes: [u8; 1024],
}

impl std::fmt::Write for PanicBuf {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        for b in s.bytes() {
            if self.len < self.bytes.len() {
                self.bytes[self.len] = if b.is_ascii() && b != 0 { b } else { b'?' };
                self.len += 1;
            }
        }
        Ok(())
    }
}

static ENGINE: Global<Option<Box<Engine>>> = Global::new(None);
static CMD: Global<[u8; proto::CMD_CAPACITY]> = Global::new([0; proto::CMD_CAPACITY]);
static PANIC: Global<PanicBuf> = Global::new(PanicBuf { len: 0, bytes: [0; 1024] });

fn engine() -> &'static mut Engine {
    ENGINE.get().as_deref_mut().expect("wt_init has not been called")
}

// ------------------------------------------------------------------ ABI

/// Hash of the spec this module was built from; the host checks it.
#[unsafe(no_mangle)]
pub extern "C" fn wt_abi_hash() -> u32 {
    proto::ABI_HASH
}

/// Create the engine. Returns 0 on success, 1 for an unusable sample rate.
#[unsafe(no_mangle)]
pub extern "C" fn wt_init(sample_rate: f32) -> u32 {
    std::panic::set_hook(Box::new(|info| {
        let b = PANIC.get();
        b.len = 0;
        let _ = write!(b, "{info}");
    }));
    if !(8_000.0..=384_000.0).contains(&sample_rate) {
        return 1;
    }
    *ENGINE.get() = Some(Engine::new(sample_rate));
    0
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_cmd_ptr() -> *mut u8 {
    CMD.get().as_mut_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_cmd_cap() -> u32 {
    proto::CMD_CAPACITY as u32
}

/// Decode `len` bytes of commands from the command buffer. Returns the count.
#[unsafe(no_mangle)]
pub extern "C" fn wt_apply(len: u32) -> u32 {
    let len = (len as usize).min(proto::CMD_CAPACITY);
    engine().apply(&CMD.get()[..len])
}

/// Render `frames` (a multiple of 16, at most wt_max_block) from absolute
/// frame `start` (negative: continue the engine's clock). 0 on success.
#[unsafe(no_mangle)]
pub extern "C" fn wt_render(frames: u32, start: f64) -> u32 {
    match engine().render(frames as usize, start) {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_out_ptr(ch: u32) -> *const f32 {
    engine().out(ch as usize).as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_tel_ptr() -> *const f32 {
    engine().telemetry().as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_tel_len() -> u32 {
    proto::tel::LEN as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_tap_ptr(i: u32) -> *const f32 {
    engine().tap(i as usize).as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_tap_count() -> u32 {
    proto::tap::COUNT as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_max_block() -> u32 {
    proto::MAX_BLOCK as u32
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_panic_ptr() -> *const u8 {
    PANIC.get().bytes.as_ptr()
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_panic_len() -> u32 {
    PANIC.get().len as u32
}

/// Allocations made so far (by anything in the module).
#[unsafe(no_mangle)]
pub extern "C" fn wt_alloc_count() -> u32 {
    ALLOCS.load(Relaxed)
}

#[unsafe(no_mangle)]
pub extern "C" fn wt_param_count() -> u32 {
    params::COUNT as u32
}

/// The engine's mapping from normalized to plain, for parity tests.
#[unsafe(no_mangle)]
pub extern "C" fn wt_param_plain(id: u32, norm: f32) -> f32 {
    params::INFO.get(id as usize).map_or(f32::NAN, |p| p.to_plain(norm))
}

/// Allocate a zeroed, 16-byte aligned region for an asset the host will
/// copy in (wavetables, samples, IRs). Returns null on failure. Called on
/// the command path, never while rendering. Ownership passes to the engine
/// with a command such as LoadTable.
#[unsafe(no_mangle)]
pub extern "C" fn wt_asset_alloc(bytes: u32) -> *mut u8 {
    AssetBuf::alloc(bytes as usize).map_or(std::ptr::null_mut(), |a| a.into_raw().0)
}

/// Free an asset the engine never took (for example after a failed upload).
///
/// # Safety
/// `ptr` must come from `wt_asset_alloc(bytes)` and not have been handed to the engine.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wt_asset_free(ptr: *mut u8, bytes: u32) {
    if !ptr.is_null() {
        drop(unsafe { AssetBuf::from_raw(ptr, bytes as usize) });
    }
}
