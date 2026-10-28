//! Shared types and the scanning pipeline of breader.
//!
//! This crate defines the data model ([`Symbol`], [`Symbology`], [`ScanOptions`]),
//! the image and geometry primitives, the extension traits ([`Preprocessor`],
//! [`Binarizer`], [`SymbologyReader`], [`PostProcessor`]) and the [`Pipeline`]
//! that runs them. It contains no symbology decoders; those live in the family
//! crates and are assembled by the `breader` facade crate.
//!
//! Depend on `breader`, which re-exports the public items of this crate: they
//! are covered by its SemVer promise (docs/API-STABILITY.md). Items that the
//! facade does not re-export are `#[doc(hidden)]` plumbing between the
//! workspace crates.

#![warn(missing_docs)]
#![forbid(unsafe_code)]

mod bitmatrix;
mod context;
mod error;
mod geometry;
mod image;
mod module;
mod options;
#[doc(hidden)]
pub mod parallel;
mod pipeline;
mod plane;
mod symbol;
mod symbology;

pub use bitmatrix::BitMatrix;
/// Plumbing shared between the reader crates; not covered by SemVer.
#[doc(hidden)]
pub use context::FrameCache;
pub use context::{FrameContext, Pass, Scratch};
pub use error::Error;
/// Plumbing shared between the reader crates; not covered by SemVer.
#[doc(hidden)]
pub use geometry::{Homography, Perspective};
pub use geometry::{Point, Quad};
pub use image::{ImageBuffer, ImageView, PixelFormat};
pub use module::{
    Binarizer, Detector, PostProcessor, Preprocessor, Reject, SymbolDecoder, SymbologyReader,
};
pub use options::{
    Code39Options, DEFAULT_UPSCALE_MAX_SIDE, EanAddonPolicy, Effort, Escalation, ItfOptions,
    ScanOptions,
};
pub use pipeline::{Pipeline, PipelineBuilder};
/// Plumbing shared between the reader crates; not covered by SemVer.
#[doc(hidden)]
pub use plane::Plane;
pub use symbol::{
    AztecMeta, Charset, Content, DataMatrixMeta, DataSegment, EcLevel, Gs1Element, LinearMeta,
    MacroPdf417, Pdf417Meta, QrMeta, Symbol, SymbolMeta,
};
pub use symbology::{Symbology, SymbologySet};
