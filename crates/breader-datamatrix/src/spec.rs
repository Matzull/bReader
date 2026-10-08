//! Symbol size table: square, rectangular and DMRE sizes with their
//! data-region and Reed-Solomon block layouts (ISO/IEC 16022 Table 7,
//! ISO/IEC 21471).

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

impl SymbolSize {
    /// Rows and columns of the mapping matrix (all data regions joined).
    pub(crate) fn mapping(&self) -> (usize, usize) {
        (
            usize::from(self.regions.0) * usize::from(self.region_size.0),
            usize::from(self.regions.1) * usize::from(self.region_size.1),
        )
    }

    /// Data and error-correction codewords of the symbol.
    pub(crate) fn total_codewords(&self) -> usize {
        usize::from(self.data_codewords) + usize::from(self.ec_codewords)
    }

    /// Position in the symbol of the mapping matrix module (`row`, `col`):
    /// (row, column) counted from the top-left corner of the symbol.
    pub(crate) fn symbol_position(&self, row: usize, col: usize) -> (usize, usize) {
        let (rh, rw) = (
            usize::from(self.region_size.0),
            usize::from(self.region_size.1),
        );
        (
            row / rh * (rh + 2) + row % rh + 1,
            col / rw * (rw + 2) + col % rw + 1,
        )
    }
}

const fn sz(
    rows: u16,
    cols: u16,
    regions: (u8, u8),
    data_codewords: u16,
    ec_codewords: u16,
    blocks: u8,
    dmre: bool,
) -> SymbolSize {
    SymbolSize {
        rows,
        cols,
        regions,
        region_size: (
            (rows / regions.0 as u16 - 2) as u8,
            (cols / regions.1 as u16 - 2) as u8,
        ),
        data_codewords,
        ec_codewords,
        blocks,
        dmre,
    }
}

const fn sq(n: u16, r: u8, data: u16, ec: u16, blocks: u8) -> SymbolSize {
    sz(n, n, (r, r), data, ec, blocks, false)
}

const fn rect(rows: u16, cols: u16, h: u8, data: u16, ec: u16) -> SymbolSize {
    sz(rows, cols, (1, h), data, ec, 1, false)
}

const fn dmre(rows: u16, cols: u16, h: u8, data: u16, ec: u16) -> SymbolSize {
    sz(rows, cols, (1, h), data, ec, 1, true)
}

/// Number of entries of [`sizes`].
pub(crate) const SIZE_COUNT: usize = 48;

static SIZES: [SymbolSize; SIZE_COUNT] = [
    sq(10, 1, 3, 5, 1),
    sq(12, 1, 5, 7, 1),
    sq(14, 1, 8, 10, 1),
    sq(16, 1, 12, 12, 1),
    sq(18, 1, 18, 14, 1),
    sq(20, 1, 22, 18, 1),
    sq(22, 1, 30, 20, 1),
    sq(24, 1, 36, 24, 1),
    sq(26, 1, 44, 28, 1),
    sq(32, 2, 62, 36, 1),
    sq(36, 2, 86, 42, 1),
    sq(40, 2, 114, 48, 1),
    sq(44, 2, 144, 56, 1),
    sq(48, 2, 174, 68, 1),
    sq(52, 2, 204, 84, 2),
    sq(64, 4, 280, 112, 2),
    sq(72, 4, 368, 144, 4),
    sq(80, 4, 456, 192, 4),
    sq(88, 4, 576, 224, 4),
    sq(96, 4, 696, 272, 4),
    sq(104, 4, 816, 336, 6),
    sq(120, 6, 1050, 408, 6),
    sq(132, 6, 1304, 496, 8),
    sq(144, 6, 1558, 620, 10),
    rect(8, 18, 1, 5, 7),
    rect(8, 32, 2, 10, 11),
    rect(12, 26, 1, 16, 14),
    rect(12, 36, 2, 22, 18),
    rect(16, 36, 2, 32, 24),
    rect(16, 48, 2, 49, 28),
    dmre(8, 48, 2, 18, 15),
    dmre(8, 64, 4, 24, 18),
    dmre(8, 80, 4, 32, 22),
    dmre(8, 96, 4, 38, 28),
    dmre(8, 120, 6, 49, 32),
    dmre(8, 144, 6, 63, 36),
    dmre(12, 64, 4, 43, 27),
    dmre(12, 88, 4, 64, 36),
    dmre(16, 64, 4, 62, 36),
    dmre(20, 36, 2, 44, 28),
    dmre(20, 44, 2, 56, 34),
    dmre(20, 64, 4, 84, 42),
    dmre(22, 48, 2, 72, 38),
    dmre(24, 48, 2, 80, 41),
    dmre(24, 64, 4, 108, 46),
    dmre(26, 40, 2, 70, 38),
    dmre(26, 48, 2, 90, 42),
    dmre(26, 64, 4, 118, 50),
];

/// All sizes, square first, then rectangular, then DMRE.
pub(crate) fn sizes() -> &'static [SymbolSize] {
    &SIZES
}

/// Exact size lookup.
pub(crate) fn size(rows: u16, cols: u16) -> Option<&'static SymbolSize> {
    sizes().iter().find(|s| s.rows == rows && s.cols == cols)
}

/// Index of `rows` x `cols` in [`sizes`].
pub(crate) fn index(rows: u16, cols: u16) -> Option<usize> {
    sizes()
        .iter()
        .position(|s| s.rows == rows && s.cols == cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_consistent() {
        assert_eq!(sizes().len(), SIZE_COUNT);
        assert_eq!(sizes().iter().filter(|s| s.dmre).count(), 18);
        for s in sizes() {
            let (mr, mc) = s.mapping();
            assert_eq!(mr + 2 * usize::from(s.regions.0), usize::from(s.rows));
            assert_eq!(mc + 2 * usize::from(s.regions.1), usize::from(s.cols));
            let area = mr * mc;
            assert_eq!(area / 8, s.total_codewords(), "{}x{}", s.rows, s.cols);
            assert!(area % 8 == 0 || area % 8 == 4, "{}x{}", s.rows, s.cols);
            let b = usize::from(s.blocks);
            assert_eq!(usize::from(s.ec_codewords) % b, 0);
            let ec = usize::from(s.ec_codewords) / b;
            let max_data = usize::from(s.data_codewords).div_ceil(b);
            assert!(max_data + ec <= 255, "{}x{}", s.rows, s.cols);
            assert_eq!(size(s.rows, s.cols), Some(s));
        }
        let unique: std::collections::HashSet<_> =
            sizes().iter().map(|s| (s.rows, s.cols)).collect();
        assert_eq!(unique.len(), SIZE_COUNT);
    }

    #[test]
    fn known_sizes() {
        let s = size(144, 144).unwrap();
        assert_eq!(
            (s.blocks, s.data_codewords, s.ec_codewords),
            (10, 1558, 620)
        );
        assert_eq!(s.region_size, (22, 22));
        let s = size(16, 48).unwrap();
        assert_eq!((s.regions, s.region_size), ((1, 2), (14, 22)));
        assert_eq!(size(10, 12), None);
    }
}
