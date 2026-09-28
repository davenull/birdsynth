//! Wavetables: the built-in saw plus one host-loaded table per oscillator.
//!
//! The host allocates an asset with `wt_asset_alloc`, fills it with
//! mip-mapped frames (built by the tools worker), and hands it over with a
//! LoadTable command. The engine owns it from then on and frees the old table
//! when a new one replaces it. Commands are applied between renders, so no
//! voice can be reading a table while it's swapped.

use std::alloc::{Layout, alloc_zeroed, dealloc};

use wt_dsp::mip::FRAME_STRIDE;
use wt_dsp::saw;

use crate::spec::protocol::{MAX_FRAMES, OSC_COUNT};

pub const ASSET_ALIGN: usize = 16;

/// A block of host-filled memory owned by the engine.
pub struct AssetBuf {
    ptr: *mut u8,
    bytes: usize,
}

impl AssetBuf {
    /// Allocate a zeroed asset. None for zero bytes or when memory runs out.
    pub fn alloc(bytes: usize) -> Option<AssetBuf> {
        let layout = Layout::from_size_align(bytes, ASSET_ALIGN).ok().filter(|l| l.size() > 0)?;
        let ptr = unsafe { alloc_zeroed(layout) };
        (!ptr.is_null()).then_some(AssetBuf { ptr, bytes })
    }

    /// Take ownership of memory from `alloc` (via `into_raw`).
    ///
    /// # Safety
    /// `ptr` and `bytes` must come from `AssetBuf::alloc(bytes).into_raw()` and
    /// must not be owned anywhere else.
    pub unsafe fn from_raw(ptr: *mut u8, bytes: usize) -> AssetBuf {
        AssetBuf { ptr, bytes }
    }

    /// Give up ownership, e.g. to hand the pointer to the host.
    pub fn into_raw(self) -> (*mut u8, usize) {
        let out = (self.ptr, self.bytes);
        std::mem::forget(self);
        out
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }

    pub fn as_f32(&self) -> &[f32] {
        unsafe { std::slice::from_raw_parts(self.ptr as *const f32, self.bytes / 4) }
    }

    pub fn as_f32_mut(&mut self) -> &mut [f32] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr as *mut f32, self.bytes / 4) }
    }
}

impl Drop for AssetBuf {
    fn drop(&mut self) {
        unsafe { dealloc(self.ptr, Layout::from_size_align_unchecked(self.bytes, ASSET_ALIGN)) }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum TableError {
    BadOsc,
    BadFrames,
    BadSize,
}

pub struct Tables {
    saw: Vec<f32>,
    osc: [Option<(AssetBuf, usize)>; OSC_COUNT],
}

impl Default for Tables {
    fn default() -> Self {
        Tables { saw: saw::saw_frame(), osc: [None, None, None] }
    }
}

impl Tables {
    /// The oscillator's table (frames × FRAME_STRIDE floats) and its frame count.
    #[inline]
    pub fn get(&self, osc: usize) -> (&[f32], usize) {
        match &self.osc[osc] {
            Some((buf, frames)) => (buf.as_f32(), *frames),
            None => (&self.saw, 1),
        }
    }

    /// The built-in band-limited saw (one frame block).
    pub fn saw(&self) -> &[f32] {
        &self.saw
    }

    pub fn frames(&self, osc: usize) -> usize {
        self.get(osc).1
    }

    /// Install a table. On error the asset is freed.
    pub fn load(&mut self, osc: usize, asset: AssetBuf, frames: usize) -> Result<(), TableError> {
        if osc >= OSC_COUNT {
            return Err(TableError::BadOsc);
        }
        if frames == 0 || frames > MAX_FRAMES {
            return Err(TableError::BadFrames);
        }
        if asset.bytes() != frames * FRAME_STRIDE * 4 {
            return Err(TableError::BadSize);
        }
        self.osc[osc] = Some((asset, frames)); // drops (frees) the previous table
        Ok(())
    }

    /// Overwrite one frame from a FRAME_STRIDE-float source.
    pub fn update_frame(&mut self, osc: usize, frame: usize, src: &[f32]) -> Result<(), TableError> {
        let Some((buf, frames)) = self.osc.get_mut(osc).ok_or(TableError::BadOsc)? else {
            return Err(TableError::BadOsc);
        };
        if frame >= *frames {
            return Err(TableError::BadFrames);
        }
        if src.len() != FRAME_STRIDE {
            return Err(TableError::BadSize);
        }
        buf.as_f32_mut()[frame * FRAME_STRIDE..(frame + 1) * FRAME_STRIDE].copy_from_slice(src);
        Ok(())
    }

    pub fn reset(&mut self, osc: usize) {
        if osc < OSC_COUNT {
            self.osc[osc] = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_validates_and_replaces() {
        let mut t = Tables::default();
        assert_eq!(t.frames(0), 1);
        let a = AssetBuf::alloc(2 * FRAME_STRIDE * 4).unwrap();
        assert!(t.load(0, a, 2).is_ok());
        assert_eq!(t.frames(0), 2);
        let bad = AssetBuf::alloc(100).unwrap();
        assert_eq!(t.load(0, bad, 2), Err(TableError::BadSize));
        assert_eq!(t.frames(0), 2, "a bad load keeps the old table");
        let src = vec![0.5f32; FRAME_STRIDE];
        assert!(t.update_frame(0, 1, &src).is_ok());
        assert_eq!(t.get(0).0[FRAME_STRIDE + 10], 0.5);
        assert_eq!(t.update_frame(0, 2, &src), Err(TableError::BadFrames));
        t.reset(0);
        assert_eq!(t.frames(0), 1);
    }

    #[test]
    fn raw_round_trip() {
        let a = AssetBuf::alloc(64).unwrap();
        let (p, n) = a.into_raw();
        let b = unsafe { AssetBuf::from_raw(p, n) };
        assert_eq!(b.bytes(), 64);
        assert!(b.as_f32().iter().all(|&v| v == 0.0));
    }
}
