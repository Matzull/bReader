use std::path::Path;

use breader::{Effort, Reader, ScanOptions, Symbology};

pub trait Engine {
    fn name(&self) -> &str;
    fn scan_file(&mut self, path: &Path) -> Result<Vec<(Symbology, Vec<u8>)>, String>;
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

    fn scan_file(&mut self, path: &Path) -> Result<Vec<(Symbology, Vec<u8>)>, String> {
        let symbols = self.reader.scan_file(path).map_err(|e| e.to_string())?;
        Ok(symbols.into_iter().map(|s| (s.symbology, s.data)).collect())
    }
}
