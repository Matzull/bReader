/// Barcode symbologies the library can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
#[repr(u8)]
pub enum Symbology {
    /// QR Code (Model 2).
    Qr,
    /// EAN-13.
    Ean13,
    /// EAN-8.
    Ean8,
    /// UPC-A.
    UpcA,
    /// UPC-E.
    UpcE,
    /// Code 128, including GS1-128.
    Code128,
    /// Code 39.
    Code39,
    /// Interleaved 2 of 5.
    Itf,
    /// Data Matrix ECC 200, including rectangular and DMRE sizes.
    DataMatrix,
    /// PDF417, including compact PDF417 and Macro PDF417.
    Pdf417,
    /// Aztec Code, including compact symbols and Aztec Runes.
    Aztec,
    /// Codabar.
    Codabar,
    /// Micro QR Code, versions M1 to M4.
    MicroQr,
    /// Rectangular Micro QR Code (rMQR), ISO/IEC 23941.
    Rmqr,
    /// GS1 DataBar Omnidirectional and Truncated (one row).
    DataBar,
    /// GS1 DataBar Limited.
    DataBarLimited,
    /// GS1 DataBar Expanded (one row).
    DataBarExpanded,
}

impl Symbology {
    /// Every symbology, in declaration order.
    pub const ALL: &'static [Self] = &[
        Self::Qr,
        Self::Ean13,
        Self::Ean8,
        Self::UpcA,
        Self::UpcE,
        Self::Code128,
        Self::Code39,
        Self::Itf,
        Self::DataMatrix,
        Self::Pdf417,
        Self::Aztec,
        Self::Codabar,
        Self::MicroQr,
        Self::Rmqr,
        Self::DataBar,
        Self::DataBarLimited,
        Self::DataBarExpanded,
    ];

    /// True for 1D symbologies.
    pub const fn is_linear(self) -> bool {
        !matches!(
            self,
            Self::Qr | Self::DataMatrix | Self::Pdf417 | Self::Aztec | Self::MicroQr | Self::Rmqr
        )
    }

    /// Human-readable name, e.g. "EAN-13".
    pub const fn name(self) -> &'static str {
        match self {
            Self::Qr => "QR Code",
            Self::Ean13 => "EAN-13",
            Self::Ean8 => "EAN-8",
            Self::UpcA => "UPC-A",
            Self::UpcE => "UPC-E",
            Self::Code128 => "Code 128",
            Self::Code39 => "Code 39",
            Self::Itf => "ITF",
            Self::DataMatrix => "Data Matrix",
            Self::Pdf417 => "PDF417",
            Self::Aztec => "Aztec",
            Self::Codabar => "Codabar",
            Self::MicroQr => "Micro QR Code",
            Self::Rmqr => "rMQR Code",
            Self::DataBar => "GS1 DataBar",
            Self::DataBarLimited => "GS1 DataBar Limited",
            Self::DataBarExpanded => "GS1 DataBar Expanded",
        }
    }
}

/// Set of [`Symbology`] values stored as a bit mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SymbologySet(u64);

impl SymbologySet {
    /// The empty set.
    pub const EMPTY: Self = Self(0);

    /// Every symbology.
    pub fn all() -> Self {
        Symbology::ALL.iter().copied().collect()
    }

    /// The set plus `s`.
    pub const fn with(self, s: Symbology) -> Self {
        Self(self.0 | 1 << s as u8)
    }

    /// Whether `s` is in the set.
    pub const fn contains(self, s: Symbology) -> bool {
        self.0 & (1 << s as u8) != 0
    }

    /// Whether the two sets share a member.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether the set has no members.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Members in declaration order.
    pub fn iter(self) -> impl Iterator<Item = Symbology> {
        Symbology::ALL
            .iter()
            .copied()
            .filter(move |s| self.contains(*s))
    }
}

impl FromIterator<Symbology> for SymbologySet {
    fn from_iter<I: IntoIterator<Item = Symbology>>(iter: I) -> Self {
        iter.into_iter().fold(Self::EMPTY, Self::with)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_ops() {
        let s = SymbologySet::EMPTY.with(Symbology::Qr).with(Symbology::Itf);
        assert!(s.contains(Symbology::Qr));
        assert!(!s.contains(Symbology::Ean8));
        assert_eq!(s.iter().count(), 2);
        assert_eq!(SymbologySet::all().iter().count(), Symbology::ALL.len());
    }
}
