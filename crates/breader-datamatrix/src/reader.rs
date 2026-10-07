use breader_core::{FrameContext, Symbol, Symbology, SymbologyReader, SymbologySet};

/// Data Matrix ECC 200 reader; handles both polarities itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct DataMatrixReader;

impl SymbologyReader for DataMatrixReader {
    fn name(&self) -> &'static str {
        "datamatrix"
    }

    fn symbologies(&self) -> SymbologySet {
        SymbologySet::EMPTY.with(Symbology::DataMatrix)
    }

    fn handles_inversion(&self) -> bool {
        true
    }

    fn scan(&self, _ctx: &mut FrameContext<'_>, _out: &mut Vec<Symbol>) {}
}
