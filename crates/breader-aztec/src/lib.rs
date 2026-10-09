//! Aztec Code reader (ISO/IEC 24778): bullseye detection, core fit,
//! orientation and layer sampling with the reference grid, then the decoder
//! (layout tables, mode message, spiral codeword extraction, Reed-Solomon
//! correction with a misdecode guard, bit unstuffing and the high-level
//! encodation modes).

#![forbid(unsafe_code)]

mod build;
mod decode;
mod detect;
#[doc(hidden)]
pub mod fuzzing;
mod reader;
mod sample;
mod spec;

#[cfg(test)]
mod testutil;

pub use reader::AztecReader;
