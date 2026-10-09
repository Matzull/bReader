//! Barcode and QR code reader.
//!
//! Supported symbologies: QR Code, Data Matrix (ECC 200, rectangular and DMRE), PDF417 (including compact),
//! EAN-13, EAN-8, UPC-A, UPC-E, Code 128, Code 39 and Interleaved 2 of 5. GS1 element strings and character sets
//! are interpreted by the content post-processors.
//!
//! The entry point is [`Reader`]: configure it with [`ScanOptions`], feed it
//! pixels and get back [`Symbol`]s. A `Reader` keeps scratch buffers between
//! calls, so reuse one per thread. For video use [`Tracker`].
//!
//! # Quick start
//!
//! ```
//! use breader::{Effort, PixelFormat, Reader, ScanOptions, Symbology};
//!
//! let mut reader = Reader::with_options(ScanOptions::with_effort(Effort::Fast));
//!
//! // Any supported pixel layout; here a blank 64x48 RGB frame.
//! let (width, height) = (64, 48);
//! let rgb = vec![255u8; width * height * 3];
//! let symbols = reader
//!     .scan_pixels(&rgb, PixelFormat::Rgb8, width, height, width * 3)
//!     .expect("buffer matches the declared geometry");
//!
//! for symbol in &symbols {
//!     match symbol.symbology {
//!         Symbology::Qr => println!("QR: {:?}", symbol.text),
//!         other => println!("{}: {:?}", other.name(), symbol.text),
//!     }
//! }
//! assert!(symbols.is_empty());
//! ```
//!
//! # Cargo features
//!
//! - `qr`, `linear`, `datamatrix`, `pdf417` (default): the QR, 1D, Data Matrix and PDF417 readers.
//! - `image`: `Reader::scan_file` and `FileError`, decoding PNG and JPEG with the `image` crate.
//! - `rayon`: multi-threaded kernels and scanning with identical output. Bound
//!   the thread count by running scans inside `rayon::ThreadPool::install`.
//!
//! # Extending
//!
//! A [`Pipeline`] is assembled from [`Preprocessor`], [`Binarizer`],
//! [`SymbologyReader`] and [`PostProcessor`] modules; pass a custom one to
//! [`Reader::from_pipeline`].

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use breader_imgproc::{luma_view, to_luma};

mod tracker;
pub use tracker::{FrameUpdate, Track, TrackEvent, Tracker, TrackerConfig};

pub use breader_core::{
    Binarizer, BitMatrix, Charset, Code39Options, Content, DataMatrixMeta, DataSegment, Detector,
    EanAddonPolicy, EcLevel, Effort, Error, Escalation, FrameContext, Gs1Element, ImageBuffer,
    ImageView, ItfOptions, LinearMeta, MacroPdf417, Pass, Pdf417Meta, Pipeline, PipelineBuilder,
    PixelFormat, Point, PostProcessor, Preprocessor, QrMeta, Quad, Reject, ScanOptions, Scratch,
    Symbol, SymbolDecoder, SymbolMeta, Symbology, SymbologyReader, SymbologySet,
};

pub use breader_content::{Gs1Error, charset, gs1};

/// Ready-made pipeline modules, for building custom pipelines.
pub mod modules {
    pub use breader_content::{ContentClassifier, Dedup, TextDecoder};
    #[cfg(feature = "datamatrix")]
    pub use breader_datamatrix::DataMatrixReader;
    pub use breader_imgproc::{Downscale, LocalBinarizer, OtsuBinarizer, Upscale};
    #[cfg(feature = "linear")]
    pub use breader_linear::LinearReader;
    #[cfg(feature = "pdf417")]
    pub use breader_pdf417::Pdf417Reader;
    #[cfg(feature = "qr")]
    pub use breader_qr::QrReader;
}

/// The standard pipeline for the enabled features: local and Otsu binarizers,
/// a 2x downscale preprocessor, a 3x up-scale for the retry pass, the QR, Data Matrix, linear and PDF417 readers, then dedup, text
/// decoding and content classification.
pub fn default_pipeline() -> Pipeline {
    let builder = Pipeline::builder()
        .binarizer(modules::LocalBinarizer::default())
        .binarizer(modules::OtsuBinarizer)
        .preprocessor(modules::Downscale::new(2))
        .upscale(modules::Upscale);
    #[cfg(feature = "qr")]
    let builder = builder.reader(modules::QrReader);
    #[cfg(feature = "datamatrix")]
    let builder = builder.reader(modules::DataMatrixReader);
    #[cfg(feature = "linear")]
    let builder = builder.reader(modules::LinearReader);
    // After the linear reader, whose scan records the patterns it starts from.
    #[cfg(feature = "pdf417")]
    let builder = builder.reader(modules::Pdf417Reader);
    builder
        .post_processor(modules::Dedup)
        .post_processor(modules::TextDecoder)
        .post_processor(modules::ContentClassifier)
        .build()
        .expect("pipeline always has a binarizer")
}

/// Scans images with a pipeline and a set of options, reusing buffers between calls.
///
/// Scanning needs `&mut self`; use one `Reader` per thread.
#[derive(Debug)]
pub struct Reader {
    pipeline: Pipeline,
    options: ScanOptions,
    scratch: Scratch,
    luma: ImageBuffer,
}

impl Reader {
    /// A reader with the default pipeline and default options.
    pub fn new() -> Self {
        Self::with_options(ScanOptions::default())
    }

    /// A reader with the default pipeline and the given options.
    pub fn with_options(options: ScanOptions) -> Self {
        Self::from_pipeline(default_pipeline(), options)
    }

    /// A reader over a custom pipeline.
    pub fn from_pipeline(pipeline: Pipeline, options: ScanOptions) -> Self {
        Self {
            pipeline,
            options,
            scratch: Scratch::new(),
            luma: ImageBuffer::default(),
        }
    }

    /// Current options.
    pub fn options(&self) -> &ScanOptions {
        &self.options
    }

    /// Mutable options; changes apply to the next scan.
    pub fn options_mut(&mut self) -> &mut ScanOptions {
        &mut self.options
    }

    /// Scans a luma image and returns the symbols found.
    pub fn scan(&mut self, image: ImageView<'_>) -> Vec<Symbol> {
        self.pipeline.scan(image, &self.options, &mut self.scratch)
    }

    /// Scans a raw pixel buffer; `stride` is the source row pitch in bytes.
    /// Formats with a luma plane are read in place, others are converted
    /// into a reused internal buffer.
    pub fn scan_pixels(
        &mut self,
        data: &[u8],
        format: PixelFormat,
        width: usize,
        height: usize,
        stride: usize,
    ) -> Result<Vec<Symbol>, Error> {
        if let Some(view) = luma_view(data, format, width, height, stride) {
            return Ok(self.scan(view?));
        }
        to_luma(data, format, width, height, stride, &mut self.luma)?;
        Ok(self
            .pipeline
            .scan(self.luma.view(), &self.options, &mut self.scratch))
    }

    /// Decodes the image file at `path` (PNG or JPEG), converts it to luma and scans it.
    #[cfg(feature = "image")]
    pub fn scan_file(
        &mut self,
        path: impl AsRef<std::path::Path>,
    ) -> Result<Vec<Symbol>, FileError> {
        let gray = image::open(path)?.into_luma8();
        let (width, height) = (gray.width() as usize, gray.height() as usize);
        let view = ImageView::from_packed(gray.as_raw(), width, height)?;
        Ok(self.scan(view))
    }
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

/// Failure of `Reader::scan_file`.
#[cfg(feature = "image")]
#[derive(Debug)]
pub enum FileError {
    /// The file could not be read or decoded.
    Image(image::ImageError),
    /// The decoded image was rejected (e.g. zero size).
    Core(Error),
}

#[cfg(feature = "image")]
impl std::fmt::Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Image(e) => write!(f, "image error: {e}"),
            Self::Core(e) => write!(f, "{e}"),
        }
    }
}

#[cfg(feature = "image")]
impl std::error::Error for FileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Image(e) => Some(e),
            Self::Core(e) => Some(e),
        }
    }
}

#[cfg(feature = "image")]
impl From<image::ImageError> for FileError {
    fn from(e: image::ImageError) -> Self {
        Self::Image(e)
    }
}

#[cfg(feature = "image")]
impl From<Error> for FileError {
    fn from(e: Error) -> Self {
        Self::Core(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_image_yields_nothing_for_every_format() {
        let mut reader = Reader::new();
        for format in [
            PixelFormat::Luma8,
            PixelFormat::Rgb8,
            PixelFormat::Rgba8,
            PixelFormat::Bgra8,
            PixelFormat::Nv12,
            PixelFormat::Nv21,
            PixelFormat::I420,
        ] {
            let stride = 32 * format.bytes_per_pixel();
            let data = vec![255u8; stride * 48];
            let found = reader.scan_pixels(&data, format, 32, 32, stride).unwrap();
            assert!(found.is_empty(), "{format:?}");
        }
    }

    #[test]
    fn rejects_small_buffers() {
        let mut reader = Reader::default();
        for format in [PixelFormat::Luma8, PixelFormat::Rgb8] {
            let stride = 32 * format.bytes_per_pixel();
            let data = vec![0u8; stride * 31];
            assert!(matches!(
                reader.scan_pixels(&data, format, 32, 32, stride),
                Err(Error::BufferTooSmall { .. })
            ));
        }
        assert!(
            reader
                .scan_pixels(&[], PixelFormat::Luma8, 0, 0, 0)
                .is_err()
        );
    }
}
