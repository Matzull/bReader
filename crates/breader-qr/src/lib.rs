//! QR Code reader.

#![forbid(unsafe_code)]

mod alignment;
mod check;
mod decode;
mod detect;
mod plane;
mod reader;
mod sample;
mod spec;

pub use reader::QrReader;
