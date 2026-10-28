//! QR Code reader.
//!
//! Implementation crate of breader: not covered by SemVer; depend on `breader`, which re-exports what is public (docs/API-STABILITY.md).

#![forbid(unsafe_code)]

mod alignment;
mod check;
mod decode;
mod detect;
#[doc(hidden)]
pub mod fuzzing;
mod micro;
mod reader;
mod rmqr;
mod sample;
mod spec;
#[cfg(test)]
mod testrender;

pub use reader::QrReader;
