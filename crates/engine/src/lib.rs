// Index loops over parallel buffers read more clearly than zipped iterators in DSP code.
#![allow(clippy::needless_range_loop)]
//! The synth engine: voices, modulation and effects, driven by the binary
//! commands described in schema/protocol.toml.

mod engine;
mod env;
mod events;
pub mod filter;
pub mod fx;
pub mod lfo;
pub mod modmatrix;
pub mod osc;
pub mod params;
pub mod samples;
pub mod spec;
pub mod tables;
mod voice;

#[cfg(test)]
mod tests;

pub use engine::{Engine, RenderError};
pub use spec::protocol::Command;
