//! Micro QR Code (M1 to M4).

mod bits;
pub(crate) mod decode;
pub(crate) mod geom;
pub(crate) mod reader;
#[cfg(test)]
mod reader_tests;
mod refine;
pub(crate) mod spec;
#[cfg(test)]
mod tests;
