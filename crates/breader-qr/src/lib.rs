//! QR Code reader.

use breader_core::{FrameContext, Symbol, Symbology, SymbologyReader, SymbologySet};

#[derive(Debug, Clone, Copy, Default)]
pub struct QrReader;

impl SymbologyReader for QrReader {
    fn name(&self) -> &'static str {
        "qr"
    }

    fn symbologies(&self) -> SymbologySet {
        SymbologySet::EMPTY.with(Symbology::Qr)
    }

    fn scan(&self, _ctx: &mut FrameContext<'_>, _out: &mut Vec<Symbol>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbology_set() {
        let set = QrReader.symbologies();
        assert_eq!(set.iter().collect::<Vec<_>>(), [Symbology::Qr]);
        assert_eq!(QrReader.name(), "qr");
    }
}
