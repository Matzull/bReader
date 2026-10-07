//! Symbol size table: square, rectangular and DMRE sizes with their
//! data-region and Reed-Solomon block layouts.

/// One ECC 200 symbol size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SymbolSize {
    /// Symbol rows including finder and clock patterns.
    pub rows: u16,
    /// Symbol columns including finder and clock patterns.
    pub cols: u16,
    /// Data regions vertically and horizontally.
    pub regions: (u8, u8),
    /// Rows and columns of each data region, excluding its finder and clock.
    pub region_size: (u8, u8),
    /// Total data codewords.
    pub data_codewords: u16,
    /// Total error-correction codewords.
    pub ec_codewords: u16,
    /// Number of interleaved Reed-Solomon blocks.
    pub blocks: u8,
    /// DMRE size (ISO/IEC 21471).
    pub dmre: bool,
}

/// All sizes, square first, then rectangular, then DMRE.
pub(crate) fn sizes() -> &'static [SymbolSize] {
    &[]
}

/// Exact size lookup.
pub(crate) fn size(rows: u16, cols: u16) -> Option<&'static SymbolSize> {
    sizes().iter().find(|s| s.rows == rows && s.cols == cols)
}
