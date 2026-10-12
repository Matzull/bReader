//! QR Code reader.

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
