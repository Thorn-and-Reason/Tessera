//! Passive UTF-8 JSON projects. Opening never resolves provenance or runs an analysis.
mod bounded;
mod decode;
mod dimensions;
mod encode;
mod validation;
pub(crate) mod wire;
mod wire_v2;
pub use decode::{decode, decode_with_budget};
pub use encode::{encode, encode_with_budget};
pub const MAX_PROJECT_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_DEPTH: usize = 64;

mod wire_v3;

mod wire_v4;

mod wire_v5;

mod wire_v6;
