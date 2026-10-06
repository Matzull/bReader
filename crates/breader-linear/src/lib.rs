//! Linear (1D) symbology reader.

use breader_core::{FrameContext, Symbol, Symbology, SymbologyReader, SymbologySet};

#[derive(Debug, Clone, Copy, Default)]
pub struct LinearReader;

impl SymbologyReader for LinearReader {
    fn name(&self) -> &'static str {
        "linear"
    }

    fn symbologies(&self) -> SymbologySet {
        [
            Symbology::Ean13,
            Symbology::Ean8,
            Symbology::UpcA,
            Symbology::UpcE,
            Symbology::Code128,
            Symbology::Code39,
            Symbology::Itf,
        ]
        .into_iter()
        .collect()
    }

    fn scan(&self, _ctx: &mut FrameContext<'_>, _out: &mut Vec<Symbol>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbology_set() {
        let set = LinearReader.symbologies();
        assert_eq!(set.iter().count(), 7);
        assert!(!set.contains(Symbology::Qr));
        assert!(set.contains(Symbology::Itf));
        assert_eq!(LinearReader.name(), "linear");
    }
}
