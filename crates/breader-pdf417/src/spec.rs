//! Symbol character tables, row indicator formulas and size limits
//! (ISO/IEC 15438).

mod clusters;
#[cfg(test)]
mod tests;

/// Rows of a symbol.
pub(crate) const MIN_ROWS: usize = 3;
pub(crate) const MAX_ROWS: usize = 90;
/// Data columns of a symbol (row indicators not counted).
pub(crate) const MIN_COLS: usize = 1;
pub(crate) const MAX_COLS: usize = 30;
/// Highest error correction level.
pub(crate) const MAX_EC_LEVEL: u8 = 8;
/// Codewords of the largest symbol: data, padding and check codewords.
pub(crate) const MAX_CODEWORDS: usize = 928;
/// Codeword values are 0 to 928.
pub(crate) const CODEWORD_COUNT: usize = 929;

/// Check codewords of an error correction level.
pub(crate) const fn ec_codewords(level: u8) -> usize {
    2 << level
}

/// Pattern tables of the clusters 0, 3 and 6, indexed by `cluster / 3`.
const CLUSTERS: [&[u16; CODEWORD_COUNT]; 3] = [
    &clusters::CLUSTER_0,
    &clusters::CLUSTER_3,
    &clusters::CLUSTER_6,
];

/// The 17 modules of codeword `value` in cluster `3 * cluster` (bar = 1, first
/// module in the most significant bit).
pub(crate) fn pattern(cluster: usize, value: u16) -> u32 {
    0x1_0000 | u32::from(CLUSTERS[cluster][usize::from(value)])
}

/// The modules of a pattern given as 8 alternating bar and space widths,
/// starting with a bar. `None` unless the widths are 1 to 6 and sum to 17.
pub(crate) fn pattern_of_widths(widths: [u8; 8]) -> Option<u32> {
    let mut bits = 0u32;
    let mut total = 0u32;
    for (i, &w) in widths.iter().enumerate() {
        if !(1..=6).contains(&w) {
            return None;
        }
        bits = bits << w | if i % 2 == 0 { (1 << w) - 1 } else { 0 };
        total += u32::from(w);
    }
    (total == 17).then_some(bits)
}

/// Cluster of a codeword, `0`, `3` or `6`, from its element widths:
/// `(b1 - b2 + b3 - b4 + 9) mod 9`.
#[cfg(test)]
pub(crate) fn cluster_of_widths(widths: [u8; 8]) -> usize {
    let [b1, _, b2, _, b3, _, b4, _] = widths.map(i32::from);
    // The bars are the even elements: b1 w1 b2 w2 b3 w3 b4 w4.
    (b1 - b2 + b3 - b4 + 9).rem_euclid(9) as usize
}

const HASH_BITS: u32 = 13;
const HASH_SIZE: usize = 1 << HASH_BITS;

const fn slot(pattern: u32) -> usize {
    ((pattern ^ pattern >> 7).wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
}

/// Open-addressing table of all 2787 patterns: 0 is empty, otherwise
/// `0x8000 | cluster / 3 << 10 | value`.
static LOOKUP: [u16; HASH_SIZE] = build_lookup();

const fn build_lookup() -> [u16; HASH_SIZE] {
    let mut table = [0u16; HASH_SIZE];
    let mut k = 0;
    while k < 3 {
        let mut value = 0;
        while value < CODEWORD_COUNT {
            let p = 0x1_0000 | CLUSTERS[k][value] as u32;
            let mut i = slot(p);
            while table[i] != 0 {
                i = (i + 1) % HASH_SIZE;
            }
            table[i] = 0x8000 | (k as u16) << 10 | value as u16;
            value += 1;
        }
        k += 1;
    }
    table
}

/// The codeword with these 17 modules: (cluster / 3, value).
pub(crate) fn lookup(modules: u32) -> Option<(usize, u16)> {
    let mut i = slot(modules);
    loop {
        let entry = LOOKUP[i];
        if entry == 0 {
            return None;
        }
        let (k, value) = (usize::from(entry >> 10 & 3), entry & 0x3FF);
        if pattern(k, value) == modules {
            return Some((k, value));
        }
        i = (i + 1) % HASH_SIZE;
    }
}

/// Dimensions and error correction level of a symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    pub rows: usize,
    pub cols: usize,
    pub ec_level: u8,
}

/// What one row indicator codeword carries besides the row group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IndicatorField {
    /// `(rows - 1) / 3`.
    RowsDiv3(u8),
    /// Error correction level and `(rows - 1) % 3`.
    LevelRowsMod3 { level: u8, rows_mod3: u8 },
    /// `cols - 1`.
    ColsMinus1(u8),
}

impl Layout {
    /// The layout in range, or `None`.
    pub(crate) fn new(rows: usize, cols: usize, ec_level: u8) -> Option<Self> {
        ((MIN_ROWS..=MAX_ROWS).contains(&rows)
            && (MIN_COLS..=MAX_COLS).contains(&cols)
            && ec_level <= MAX_EC_LEVEL)
            .then_some(Self {
                rows,
                cols,
                ec_level,
            })
    }

    /// Value of the left row indicator of `row`. The clusters carry, in
    /// turn, `(rows - 1) / 3`, `3 * level + (rows - 1) % 3` and `cols - 1`
    /// on top of `30 * (row / 3)`; the right indicator rotates the roles.
    #[cfg(test)]
    pub(crate) fn left_indicator(&self, row: usize) -> u16 {
        self.indicator(row, row % 3)
    }

    /// Value of the right row indicator of `row`.
    pub(crate) fn right_indicator(&self, row: usize) -> u16 {
        self.indicator(row, (row + 2) % 3)
    }

    fn indicator(&self, row: usize, role: usize) -> u16 {
        let x = match role {
            0 => (self.rows - 1) / 3,
            1 => 3 * usize::from(self.ec_level) + (self.rows - 1) % 3,
            _ => self.cols - 1,
        };
        (30 * (row / 3) + x) as u16
    }
}

/// Reads an indicator codeword of `row`: the field it carries, or `None`
/// when the value is outside the symbology's ranges or its row group is not
/// `row / 3`.
pub(crate) fn parse_indicator(row: usize, left: bool, value: u16) -> Option<IndicatorField> {
    if usize::from(value) / 30 != row / 3 || usize::from(value) >= 900 {
        return None;
    }
    let x = (value % 30) as u8;
    let role = if left { row % 3 } else { (row + 2) % 3 };
    match role {
        0 => Some(IndicatorField::RowsDiv3(x)),
        1 => (x <= 3 * MAX_EC_LEVEL + 2).then_some(IndicatorField::LevelRowsMod3 {
            level: x / 3,
            rows_mod3: x % 3,
        }),
        _ => Some(IndicatorField::ColsMinus1(x)),
    }
}

/// The layout described by the three fields, or `None` when out of range.
pub(crate) fn combine_fields(
    rows_div3: u8,
    level: u8,
    rows_mod3: u8,
    cols_minus1: u8,
) -> Option<Layout> {
    Layout::new(
        usize::from(rows_div3) * 3 + usize::from(rows_mod3) + 1,
        usize::from(cols_minus1) + 1,
        level,
    )
}
