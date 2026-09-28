//! The voice filters live in `wt_dsp::filter` (shared with the tools, which
//! draw their response curves from the same code); this re-exports them.

pub use wt_dsp::filter::*;

/// Internal block length at 4× oversampling.
pub const MAX_N: usize = MAX_BLOCK;
const _: () = assert!(MAX_N == crate::spec::protocol::SUB_BLOCK * 4);
