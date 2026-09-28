// Index loops over parallel buffers read more clearly than zipped iterators in DSP code.
#![allow(clippy::needless_range_loop)]
//! Shared DSP building blocks for the engine and the tools.

pub mod math;
pub mod mip;
pub mod oversample;
pub mod phase;
pub mod rng;
pub mod saw;
pub mod warp;
