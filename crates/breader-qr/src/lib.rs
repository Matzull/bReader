//! QR Code reader.

#![forbid(unsafe_code)]

mod alignment;
mod decode;
mod detect;
mod reader;
mod sample;
mod spec;

pub use reader::QrReader;
