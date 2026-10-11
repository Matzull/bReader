//! rMQR constants from ISO/IEC 23941: sizes, block layouts, function patterns
//! and format information.

use std::sync::OnceLock;

/// Number of rMQR versions.
pub(crate) const VERSIONS: usize = 32;

/// `(height, width)` of the 32 versions in version-indicator order.
pub(crate) const SIZES: [(u8, u8); VERSIONS] = [
    (7, 43),
    (7, 59),
    (7, 77),
    (7, 99),
    (7, 139),
    (9, 43),
    (9, 59),
    (9, 77),
    (9, 99),
    (9, 139),
    (11, 27),
    (11, 43),
    (11, 59),
    (11, 77),
    (11, 99),
    (11, 139),
    (13, 27),
    (13, 43),
    (13, 59),
    (13, 77),
    (13, 99),
    (13, 139),
    (15, 43),
    (15, 59),
    (15, 77),
    (15, 99),
    (15, 139),
    (17, 43),
    (17, 59),
    (17, 77),
    (17, 99),
    (17, 139),
];

/// Version indicator of a module count, if it is an rMQR size.
pub(crate) fn version_for_size(width: usize, height: usize) -> Option<usize> {
    SIZES
        .iter()
        .position(|&(h, w)| usize::from(h) == height && usize::from(w) == width)
}

/// Centers of the vertical timing columns (alignment patterns) by width.
pub(crate) const fn align_centers(width: usize) -> &'static [u8] {
    match width {
        43 => &[21],
        59 => &[19, 39],
        77 => &[25, 51],
        99 => &[23, 49, 75],
        139 => &[27, 55, 83, 111],
        _ => &[],
    }
}

/// Codewords and Reed-Solomon blocks of a version at one level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    pub total: usize,
    pub blocks: usize,
    /// Check codewords per block.
    pub ec: usize,
}

impl Layout {
    pub fn data_codewords(&self) -> usize {
        self.total - self.blocks * self.ec
    }

    /// Data codewords of block `i`: the last `data % blocks` blocks are one longer.
    pub fn block_data(&self, i: usize) -> usize {
        let d = self.data_codewords();
        d / self.blocks + usize::from(i >= self.blocks - d % self.blocks)
    }
}

const fn lay(total: usize, m: (usize, usize), h: (usize, usize)) -> [Layout; 2] {
    [
        Layout {
            total,
            blocks: m.0,
            ec: m.1,
        },
        Layout {
            total,
            blocks: h.0,
            ec: h.1,
        },
    ]
}

/// Codewords, then (blocks, check codewords per block) for levels M and H.
#[rustfmt::skip]
const LAYOUTS: [[Layout; 2]; VERSIONS] = [
    lay(13, (1, 7), (1, 10)),
    lay(21, (1, 9), (1, 14)),
    lay(32, (1, 12), (1, 22)),
    lay(44, (1, 16), (1, 30)),
    lay(68, (1, 24), (2, 22)),
    lay(21, (1, 9), (1, 14)),
    lay(33, (1, 12), (1, 22)),
    lay(49, (1, 18), (2, 16)),
    lay(66, (1, 24), (2, 22)),
    lay(99, (2, 18), (3, 22)),
    lay(15, (1, 8), (1, 10)),
    lay(31, (1, 12), (1, 20)),
    lay(47, (1, 16), (2, 16)),
    lay(67, (1, 24), (2, 22)),
    lay(89, (2, 16), (2, 30)),
    lay(132, (2, 24), (3, 30)),
    lay(21, (1, 9), (1, 14)),
    lay(41, (1, 14), (1, 28)),
    lay(60, (1, 22), (2, 20)),
    lay(85, (2, 16), (2, 28)),
    lay(113, (2, 20), (3, 26)),
    lay(166, (3, 20), (4, 28)),
    lay(51, (1, 18), (2, 18)),
    lay(74, (1, 26), (2, 24)),
    lay(103, (2, 18), (3, 24)),
    lay(136, (2, 24), (4, 22)),
    lay(199, (3, 24), (5, 26)),
    lay(61, (1, 22), (2, 20)),
    lay(88, (2, 16), (2, 30)),
    lay(122, (2, 22), (3, 28)),
    lay(160, (3, 20), (4, 26)),
    lay(232, (4, 20), (6, 26)),
];

/// Block layout of a version indicator at level M (`high == false`) or H.
pub(crate) fn layout(version: usize, high: bool) -> Layout {
    LAYOUTS[version][usize::from(high)]
}

/// Character count indicator widths of the numeric, alphanumeric, byte and
/// kanji modes by version indicator.
#[rustfmt::skip]
const COUNT_BITS: [[u8; 4]; VERSIONS] = [
    [4, 3, 3, 2], [5, 5, 4, 3], [6, 5, 5, 4], [7, 6, 5, 5], [7, 6, 6, 5],
    [5, 5, 4, 3], [6, 5, 5, 4], [7, 6, 5, 5], [7, 6, 6, 5], [8, 7, 6, 6],
    [4, 4, 3, 2], [6, 5, 5, 4], [7, 6, 5, 5], [7, 6, 6, 5], [8, 7, 6, 6], [8, 7, 7, 6],
    [5, 5, 4, 3], [6, 6, 5, 5], [7, 6, 6, 5], [7, 7, 6, 6], [8, 7, 7, 6], [8, 8, 7, 7],
    [7, 6, 6, 5], [7, 7, 6, 5], [8, 7, 7, 6], [8, 7, 7, 6], [9, 8, 7, 7],
    [7, 6, 6, 5], [8, 7, 6, 6], [8, 7, 7, 6], [8, 8, 7, 6], [9, 8, 8, 7],
];

/// Character count width for a mode (0 numeric, 1 alphanumeric, 2 byte, 3 kanji).
pub(crate) fn count_bits(version: usize, mode: usize) -> usize {
    usize::from(COUNT_BITS[version][mode])
}

/// Generator of the BCH(18,6) code of the format information.
const FORMAT_GENERATOR: u32 = 0x1F25;
/// XOR masks of the two copies of the format information.
pub(crate) const FORMAT_MASK_LEFT: u32 = 0x1FAB2;
pub(crate) const FORMAT_MASK_RIGHT: u32 = 0x20A7B;

/// The 64 code words of the format information: `data6 << 12 | check`, where
/// `data6` is the level (bit 5: 0 for M, 1 for H) and the version indicator.
pub(crate) const FORMAT_WORDS: [u32; 64] = {
    let mut t = [0u32; 64];
    let mut d = 0;
    while d < 64 {
        let mut v = (d as u32) << 12;
        let mut i = 17;
        while i >= 12 {
            if v >> i & 1 == 1 {
                v ^= FORMAT_GENERATOR << (i - 12);
            }
            i -= 1;
        }
        t[d] = ((d as u32) << 12) | v;
        d += 1;
    }
    t
};

/// A square of the format information: `(x, y)` in a symbol of `(w, h)`.
pub(crate) type Cell = (usize, usize);

/// Modules of the copy next to the finder, bit 0 first. The last three bits
/// may also be stored in the opposite order (see [`format_cells`]).
pub(crate) fn format_left() -> [Cell; 18] {
    let mut c = [(0, 0); 18];
    for col in 0..3 {
        for row in 0..5 {
            c[col * 5 + row] = (8 + col, 1 + row);
        }
    }
    for k in 0..3 {
        c[15 + k] = (11, 1 + k);
    }
    c
}

/// Modules of the copy next to the corner sub-finder.
pub(crate) fn format_right(w: usize, h: usize) -> [Cell; 18] {
    let mut c = [(0, 0); 18];
    for col in 0..3 {
        for row in 0..5 {
            c[col * 5 + row] = (w - 8 + col, h - 6 + row);
        }
    }
    for k in 0..3 {
        c[15 + k] = (w - 5 + k, h - 6);
    }
    c
}

/// Cell codes of a [`Template`].
pub(crate) const DATA: u8 = 0;
pub(crate) const LIGHT: u8 = 1;
pub(crate) const DARK: u8 = 2;
/// Format information: reserved, value unknown.
pub(crate) const FORMAT: u8 = 3;

/// The function patterns of one version and the data module order.
#[derive(Debug)]
pub(crate) struct Template {
    pub w: usize,
    pub h: usize,
    /// Row-major cell codes.
    pub cells: Vec<u8>,
    /// Data modules `(x, y)` in codeword order.
    pub data: Vec<(u16, u16)>,
    /// Number of cells with a known value (light or dark).
    pub known: usize,
}

impl Template {
    pub fn cell(&self, x: usize, y: usize) -> u8 {
        self.cells[y * self.w + x]
    }

    fn build(version: usize) -> Self {
        let (h, w) = (usize::from(SIZES[version].0), usize::from(SIZES[version].1));
        let mut cells = vec![DATA; w * h];
        let mut set = |x: usize, y: usize, dark: bool| {
            cells[y * w + x] = if dark { DARK } else { LIGHT };
        };
        // Timing patterns along the four edges.
        for x in 0..w {
            set(x, 0, x % 2 == 0);
            set(x, h - 1, x % 2 == 0);
        }
        for y in 0..h {
            set(0, y, y % 2 == 0);
            set(w - 1, y, y % 2 == 0);
        }
        // Corner patterns: three dark modules along the edge plus two on the
        // adjacent edge, and a light module inside.
        for x in w - 3..w {
            set(x, 0, true);
        }
        for y in 1..3 {
            set(w - 1, y, true);
        }
        set(w - 2, 1, false);
        for x in 0..3 {
            set(x, h - 1, true);
        }
        for y in h - 3..h - 1 {
            set(0, y, true);
        }
        set(1, h - 2, false);
        // Alignment patterns: a vertical timing line capped by 3x3 rings.
        for &c in align_centers(w) {
            let c = usize::from(c);
            for y in 0..h {
                set(c, y, y % 2 == 0);
            }
            for dy in 0..3 {
                for dx in [c - 1, c + 1] {
                    set(dx, dy, true);
                    set(dx, h - 1 - dy, true);
                }
            }
            set(c, 1, false);
            set(c, h - 2, false);
        }
        // Corner sub-finder.
        for dy in 0..5usize {
            for dx in 0..5usize {
                let ring = (dx as i32 - 2).abs().max((dy as i32 - 2).abs());
                set(w - 5 + dx, h - 5 + dy, ring != 1);
            }
        }
        // Finder with its separators.
        for y in 0..7usize {
            for x in 0..7usize {
                let ring = (x as i32 - 3).abs().max((y as i32 - 3).abs());
                set(x, y, ring != 2);
            }
            set(7, y, false);
        }
        if h > 7 {
            for x in 0..8 {
                set(x, 7, false);
            }
        }
        for (x, y) in format_left().into_iter().chain(format_right(w, h)) {
            cells[y * w + x] = FORMAT;
        }
        let known = cells.iter().filter(|&&c| c == LIGHT || c == DARK).count();
        // Two-module wide columns from the right, alternately up and down,
        // starting next to the right edge.
        let mut data = Vec::with_capacity(w * h - known);
        let mut up = true;
        let mut x = w - 2;
        while x >= 1 {
            for k in 0..h {
                let y = if up { h - 1 - k } else { k };
                for xx in [x, x - 1] {
                    if cells[y * w + xx] == DATA {
                        data.push((xx as u16, y as u16));
                    }
                }
            }
            up = !up;
            if x < 3 {
                break;
            }
            x -= 2;
        }
        Self {
            w,
            h,
            cells,
            data,
            known,
        }
    }
}

/// Template of a version indicator, built on first use.
pub(crate) fn template(version: usize) -> &'static Template {
    static T: [OnceLock<Template>; VERSIONS] = [const { OnceLock::new() }; VERSIONS];
    T[version].get_or_init(|| Template::build(version))
}

/// The one fixed data mask: flips the module when it returns true.
pub(crate) const fn mask_bit(x: usize, y: usize) -> bool {
    (y / 2 + x / 3).is_multiple_of(2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_modules_match_the_codeword_counts() {
        for v in 0..VERSIONS {
            let t = template(v);
            for high in [false, true] {
                let l = layout(v, high);
                assert!(t.data.len() >= 8 * l.total, "{v}");
                assert!(t.data.len() < 8 * l.total + 8, "{v}");
                let sum: usize = (0..l.blocks).map(|i| l.block_data(i)).sum();
                assert_eq!(sum, l.data_codewords());
            }
        }
    }

    #[test]
    fn format_words_have_distance_eight() {
        let mut min = 99;
        for (i, a) in FORMAT_WORDS.iter().enumerate() {
            for b in &FORMAT_WORDS[i + 1..] {
                min = min.min((a ^ b).count_ones());
            }
        }
        assert_eq!(min, 8);
    }
}
