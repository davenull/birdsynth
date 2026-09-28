// Index loops over parallel buffers read more clearly than zipped iterators in DSP code.
#![allow(clippy::needless_range_loop)]
//! Offline helpers that run in the tools Web Worker (tools.wasm) and in
//! native tests: mip-map building, factory wavetables, resampling cycles to
//! the frame size, and previews through the engine's warp math.

pub mod factory;
pub mod ir;
pub mod mips;
pub mod multis;
pub mod noise;
pub mod onsets;
pub mod preview;
pub mod recording;
pub mod resample;
pub mod spectral;
pub mod wt;
