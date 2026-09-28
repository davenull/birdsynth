//! The synth engine: voices, modulation and effects, driven by the binary
//! commands described in schema/protocol.toml.

mod engine;
mod env;
mod events;
pub mod spec;
pub mod params;
mod voice;

pub use engine::{Engine, RenderError};
pub use spec::protocol::Command;
