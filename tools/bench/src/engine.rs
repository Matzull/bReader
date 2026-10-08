use breader::charset;
use breader::{Charset, Effort, Escalation, ImageView, Reader, ScanOptions, Symbology};
use image::GrayImage;

#[cfg(feature = "baselines")]
pub use crate::baselines::{RqrrEngine, RxingEngine};

/// One decoded symbol: reported symbology and payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decoded {
    pub symbology: Symbology,
    pub data: Vec<u8>,
    /// Other forms a dataset may label instead of `data`: charset-decoded UTF-8 text.
    pub alts: Vec<Vec<u8>>,
}

impl Decoded {
    pub fn new(symbology: Symbology, data: Vec<u8>) -> Self {
        Self {
            symbology,
            data,
            alts: Vec::new(),
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
        let options = ScanOptions {
            escalation,
            ..ScanOptions::with_effort(effort)
        };
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
                let alts = s
                    .text
                    .into_iter()
                    .chain(sjis)
                    .map(String::into_bytes)
                    .filter(|t| *t != s.data)
                    .collect();
                Decoded {
                    symbology: s.symbology,
                    data: s.data,
                    alts,
                }
            })
            .collect())
    }
}
