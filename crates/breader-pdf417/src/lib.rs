//! PDF417 codeword decoding (ISO/IEC 15438): symbol character tables, row
//! indicators, Reed-Solomon correction and the compaction modes.

#![forbid(unsafe_code)]
// Start/stop search, sampling and the reader use the rest of `spec` later.
#![cfg_attr(not(test), allow(dead_code))]

mod decode;
#[doc(hidden)]
pub mod fuzzing;
mod spec;

#[cfg(test)]
mod testutil;
