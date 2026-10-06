#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
#[repr(u8)]
pub enum Symbology {
    Qr,
    Ean13,
    Ean8,
    UpcA,
    UpcE,
    Code128,
    Code39,
    Itf,
}

impl Symbology {
    pub const ALL: &'static [Self] = &[
        Self::Qr,
        Self::Ean13,
        Self::Ean8,
        Self::UpcA,
        Self::UpcE,
        Self::Code128,
        Self::Code39,
        Self::Itf,
    ];

    pub const fn is_linear(self) -> bool {
        !matches!(self, Self::Qr)
    }

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
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SymbologySet(u64);

impl SymbologySet {
    pub const EMPTY: Self = Self(0);

    pub fn all() -> Self {
        Symbology::ALL.iter().copied().collect()
    }

    pub const fn with(self, s: Symbology) -> Self {
        Self(self.0 | 1 << s as u8)
    }

    pub const fn contains(self, s: Symbology) -> bool {
        self.0 & (1 << s as u8) != 0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

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
