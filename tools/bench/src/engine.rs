use breader::{Effort, ImageView, Reader, ScanOptions, Symbology};
use image::GrayImage;

#[cfg(feature = "baselines")]
pub use crate::baselines::{RqrrEngine, RxingEngine};

/// One decoded symbol: reported symbology and payload bytes.
pub type Decoded = (Symbology, Vec<u8>);

pub trait Engine {
    fn name(&self) -> &str;
    /// Decodes an already loaded grayscale image; this call is what gets timed.
    fn decode(&mut self, img: &GrayImage) -> Result<Vec<Decoded>, String>;
}

#[derive(Debug)]
pub struct BreaderEngine {
    reader: Reader,
}

impl BreaderEngine {
    pub fn new(effort: Effort) -> Self {
        Self {
            reader: Reader::with_options(ScanOptions::with_effort(effort)),
        }
    }
}

impl Engine for BreaderEngine {
    fn name(&self) -> &str {
        "breader"
    }

    fn decode(&mut self, img: &GrayImage) -> Result<Vec<Decoded>, String> {
        let view =
            ImageView::from_packed(img.as_raw(), img.width() as usize, img.height() as usize)
                .map_err(|e| e.to_string())?;
        let symbols = self.reader.scan(view);
        Ok(symbols.into_iter().map(|s| (s.symbology, s.data)).collect())
    }
}
