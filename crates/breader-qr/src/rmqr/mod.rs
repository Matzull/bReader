//! Rectangular Micro QR Code (rMQR, ISO/IEC 23941).

mod bits;
pub(crate) mod decode;
pub(crate) mod reader;
#[cfg(test)]
mod reader_tests;
#[cfg(test)]
mod samples;
pub(crate) mod spec;
#[cfg(test)]
mod tests;
