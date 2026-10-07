//! Barcode and QR code reader.

#![forbid(unsafe_code)]

use breader_imgproc::{luma_view, to_luma};

mod tracker;
pub use tracker::{FrameUpdate, Track, TrackEvent, Tracker, TrackerConfig};

pub use breader_core::{
    Binarizer, BitMatrix, Charset, Code39Options, Content, DataSegment, Detector, EanAddonPolicy,
    EcLevel, Effort, Error, Escalation, FrameContext, Gs1Element, ImageBuffer, ImageView,
    ItfOptions, LinearMeta, Pass, Pipeline, PipelineBuilder, PixelFormat, Point, PostProcessor,
    Preprocessor, QrMeta, Quad, Reject, ScanOptions, Scratch, Symbol, SymbolDecoder, SymbolMeta,
    Symbology, SymbologyReader, SymbologySet,
};

pub use breader_content::{Gs1Error, charset, gs1};

pub mod modules {
    pub use breader_content::{ContentClassifier, Dedup, TextDecoder};
    pub use breader_imgproc::{Downscale, LocalBinarizer, OtsuBinarizer};
    #[cfg(feature = "linear")]
    pub use breader_linear::LinearReader;
    #[cfg(feature = "qr")]
    pub use breader_qr::QrReader;
}

pub fn default_pipeline() -> Pipeline {
    let builder = Pipeline::builder()
        .binarizer(modules::LocalBinarizer::default())
        .binarizer(modules::OtsuBinarizer)
        .preprocessor(modules::Downscale::new(2));
    #[cfg(feature = "qr")]
    let builder = builder.reader(modules::QrReader);
    #[cfg(feature = "linear")]
    let builder = builder.reader(modules::LinearReader);
    builder
        .post_processor(modules::Dedup)
        .post_processor(modules::TextDecoder)
        .post_processor(modules::ContentClassifier)
        .build()
        .expect("pipeline always has a binarizer")
}

#[derive(Debug)]
pub struct Reader {
    pipeline: Pipeline,
    options: ScanOptions,
    scratch: Scratch,
    luma: ImageBuffer,
}

impl Reader {
    pub fn new() -> Self {
        Self::with_options(ScanOptions::default())
    }

    pub fn with_options(options: ScanOptions) -> Self {
        Self::from_pipeline(default_pipeline(), options)
    }

    pub fn from_pipeline(pipeline: Pipeline, options: ScanOptions) -> Self {
        Self {
            pipeline,
            options,
            scratch: Scratch::new(),
            luma: ImageBuffer::default(),
        }
    }

    pub fn options(&self) -> &ScanOptions {
        &self.options
    }

    pub fn options_mut(&mut self) -> &mut ScanOptions {
        &mut self.options
    }

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

#[cfg(feature = "image")]
#[derive(Debug)]
pub enum FileError {
    Image(image::ImageError),
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
