//! Linear (1D) symbology reader.

#![forbid(unsafe_code)]

mod aggregate;
mod code128;
mod code39;
mod common;
mod decoder;
mod ean;
mod edges;
mod itf;
mod plan;
mod profile;
mod reader;
#[cfg(test)]
mod robustness;
mod runs;
#[cfg(test)]
mod testutil;

pub use reader::LinearReader;

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
