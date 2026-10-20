//! Linear (1D) symbology reader.

#![forbid(unsafe_code)]

mod aggregate;
mod codabar;
mod code128;
mod code39;
mod code93;
mod common;
mod databar;
mod decoder;
mod ean;
mod edges;
#[doc(hidden)]
pub mod fuzzing;
mod itf;
mod marks;
mod plan;
mod profile;
mod reader;
#[cfg(test)]
mod robustness;
mod runs;
#[cfg(test)]
mod stack_testutil;
#[cfg(test)]
mod testutil;

pub use reader::LinearReader;

/// Scanline machinery that the PDF417 reader builds on.
#[doc(hidden)]
pub mod stacked {
    pub use crate::edges::{EdgeDetector, EdgeParams, Edges};
    pub use crate::marks::{Mark, MarkKind, MarkSet, RawMark, find};
    pub use crate::plan::{ScanPlan, Scanline};
    pub use crate::profile::Oblique;
    pub use crate::reader::scan_marks;
    pub use crate::runs::Runs;
}

#[cfg(test)]
mod tests {
    use super::*;
    use breader_core::{Symbology, SymbologyReader};

    #[test]
    fn symbology_set() {
        let set = LinearReader.symbologies();
        assert_eq!(set.iter().count(), 12);
        assert!(set.contains(Symbology::Code93));
        assert!(set.contains(Symbology::DataBarExpanded));
        assert!(!set.contains(Symbology::Qr));
        assert!(set.contains(Symbology::Itf));
        assert_eq!(LinearReader.name(), "linear");
        assert!(!LinearReader.uses_binary());
    }
}
