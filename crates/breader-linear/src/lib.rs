//! Linear (1D) symbology reader.

#![forbid(unsafe_code)]

mod aggregate;
mod decoder;
mod ean;
mod edges;
mod plan;
mod profile;
mod reader;
#[cfg(test)]
mod robustness;
mod runs;
#[cfg(test)]
mod testutil;

pub use aggregate::{Aggregator, PlacedHit, Settings};
pub use decoder::{LineHit, LinearDecoder};
pub use ean::EanDecoder;
pub use edges::{Edge, EdgeDetector, EdgeParams, Edges};
pub use plan::{ScanPlan, Scanline};
pub use reader::LinearReader;
pub use runs::Runs;

#[cfg(test)]
mod tests {
    use super::*;
    use breader_core::{Symbology, SymbologyReader};

    #[test]
    fn symbology_set() {
        let set = LinearReader.symbologies();
        assert_eq!(set.iter().count(), 7);
        assert!(!set.contains(Symbology::Qr));
        assert!(set.contains(Symbology::Itf));
        assert_eq!(LinearReader.name(), "linear");
        assert!(!LinearReader.uses_binary());
    }
}
