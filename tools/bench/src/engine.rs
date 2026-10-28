use breader::charset;
use breader::{Charset, Effort, Escalation, ImageView, Reader, ScanOptions, Symbology};
use image::GrayImage;

#[cfg(feature = "baselines")]
pub use crate::baselines::{RqrrEngine, RxingEngine};

/// One decoded symbol: reported symbology and payload bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub symbology: Symbology,
    pub data: Vec<u8>,
    /// Other forms a dataset may label instead of `data`: charset-decoded UTF-8 text, or the
    /// Code 39 Full ASCII expansion.
    pub alts: Vec<Vec<u8>>,
    /// Corners of the symbol in image pixels; empty when the engine reports none. Only used to
    /// tell whether an unexpected read lies on an undecodable ground-truth region.
    pub quad: Vec<[f64; 2]>,
}

impl Decoded {
    pub fn new(symbology: Symbology, data: Vec<u8>) -> Self {
        Self {
            symbology,
            data,
            alts: Vec::new(),
            quad: Vec::new(),
        }
    }
}

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
    pub fn new(effort: Effort, escalation: Option<Escalation>) -> Self {
        let mut options = ScanOptions::with_effort(effort);
        options.escalation = escalation;
        Self {
            reader: Reader::with_options(options),
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
        Ok(symbols
            .into_iter()
            .map(|s| {
                // Undeclared byte-mode text has no certain charset: labels may follow the
                // writer's Shift JIS reading even when the reader settles for another guess.
                let sjis = std::str::from_utf8(&s.data)
                    .is_err()
                    .then(|| charset::decode(Charset::ShiftJis, &s.data))
                    .flatten();
                let full_ascii = (s.symbology == Symbology::Code39)
                    .then(|| crate::matching::code39_full_ascii(&s.data))
                    .flatten()
                    .and_then(|v| String::from_utf8(v).ok());
                let alts = s
                    .text
                    .into_iter()
                    .chain(sjis)
                    .chain(full_ascii)
                    .map(String::into_bytes)
                    .filter(|t| *t != s.data)
                    .collect();
                Decoded {
                    symbology: s.symbology,
                    data: s.data,
                    alts,
                    quad: s
                        .quad
                        .corners
                        .iter()
                        .map(|p| [f64::from(p.x), f64::from(p.y)])
                        .collect(),
                }
            })
            .collect())
    }
}
