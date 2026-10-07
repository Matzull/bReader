#![forbid(unsafe_code)]

mod bitmatrix;
mod context;
mod error;
mod geometry;
mod image;
mod module;
mod options;
mod pipeline;
mod symbol;
mod symbology;

pub use bitmatrix::BitMatrix;
pub use context::{FrameContext, Pass, Scratch};
pub use error::Error;
pub use geometry::{Perspective, Point, Quad};
pub use image::{ImageBuffer, ImageView, PixelFormat};
pub use module::{
    Binarizer, Detector, PostProcessor, Preprocessor, Reject, SymbolDecoder, SymbologyReader,
};
pub use options::{Code39Options, EanAddonPolicy, Effort, Escalation, ItfOptions, ScanOptions};
pub use pipeline::{Pipeline, PipelineBuilder};
pub use symbol::{Content, EcLevel, Gs1Element, LinearMeta, QrMeta, Symbol, SymbolMeta};
pub use symbology::{Symbology, SymbologySet};
