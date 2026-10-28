//! PDF417 and compact PDF417 reader (ISO/IEC 15438): detection from start and
//! stop patterns, row sampling, grid votes, Reed-Solomon correction and the
//! compaction modes.
//!
//! Implementation crate of breader: not covered by SemVer; depend on `breader`, which re-exports what is public (docs/API-STABILITY.md).

#![forbid(unsafe_code)]

mod decode;
mod detect;
#[doc(hidden)]
pub mod fuzzing;
mod geom;
mod grid;
mod reader;
mod row;
mod scan;
mod spec;

pub use reader::Pdf417Reader;

#[cfg(test)]
mod testutil;
