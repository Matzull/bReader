//! Data Matrix ECC 200 reader (ISO/IEC 16022, DMRE per ISO/IEC 21471).

#![forbid(unsafe_code)]

mod check;
mod decode;
mod detect;
#[doc(hidden)]
pub mod fuzzing;
mod place;
mod reader;
mod sample;
mod spec;

pub use reader::DataMatrixReader;

#[cfg(test)]
mod testutil;
