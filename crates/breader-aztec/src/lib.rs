//! Aztec Code reader (ISO/IEC 24778), decoding side: layout tables, mode
//! message, spiral codeword extraction, Reed-Solomon correction with a
//! misdecode guard, bit unstuffing and the high-level encodation modes.
//! Detection, sampling and the reader come later.

#![forbid(unsafe_code)]
// The bullseye detector, sampler and reader use the rest of `spec` and
// `decode` later.
#![cfg_attr(not(test), allow(dead_code))]

mod build;
mod decode;
#[doc(hidden)]
pub mod fuzzing;
mod spec;

#[cfg(test)]
mod testutil;
