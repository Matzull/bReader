//! Version-dependent constants from ISO/IEC 18004.

use crate::alignment::alignment_centers;
use breader_core::{BitMatrix, EcLevel};
use std::sync::OnceLock;

pub(crate) const fn dimension(version: u8) -> usize {
    17 + 4 * version as usize
}

/// Version estimated from a module count, if it is a valid QR dimension.
pub(crate) const fn version_for_dimension(dim: usize) -> Option<u8> {
    if dim < 21 || dim > 177 || !(dim - 17).is_multiple_of(4) {
        None
    } else {
        Some(((dim - 17) / 4) as u8)
    }
}

/// Error-correction block layout for a version and level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BlockLayout {
    pub ec_per_block: u8,
    /// (block count, data codewords per block), at most two groups.
    pub groups: [(u8, u8); 2],
}

const fn bl(ec: u8, g1: (u8, u8), g2: (u8, u8)) -> BlockLayout {
    BlockLayout {
        ec_per_block: ec,
        groups: [g1, g2],
    }
}

const N: (u8, u8) = (0, 0);

/// ISO/IEC 18004 Table 9, versions 1..=40, levels in the order L, M, Q, H.
#[rustfmt::skip]
const LAYOUTS: [[BlockLayout; 4]; 40] = [
    [bl(7,(1,19),N), bl(10,(1,16),N), bl(13,(1,13),N), bl(17,(1,9),N)],
    [bl(10,(1,34),N), bl(16,(1,28),N), bl(22,(1,22),N), bl(28,(1,16),N)],
    [bl(15,(1,55),N), bl(26,(1,44),N), bl(18,(2,17),N), bl(22,(2,13),N)],
    [bl(20,(1,80),N), bl(18,(2,32),N), bl(26,(2,24),N), bl(16,(4,9),N)],
    [bl(26,(1,108),N), bl(24,(2,43),N), bl(18,(2,15),(2,16)), bl(22,(2,11),(2,12))],
    [bl(18,(2,68),N), bl(16,(4,27),N), bl(24,(4,19),N), bl(28,(4,15),N)],
    [bl(20,(2,78),N), bl(18,(4,31),N), bl(18,(2,14),(4,15)), bl(26,(4,13),(1,14))],
    [bl(24,(2,97),N), bl(22,(2,38),(2,39)), bl(22,(4,18),(2,19)), bl(26,(4,14),(2,15))],
    [bl(30,(2,116),N), bl(22,(3,36),(2,37)), bl(20,(4,16),(4,17)), bl(24,(4,12),(4,13))],
    [bl(18,(2,68),(2,69)), bl(26,(4,43),(1,44)), bl(24,(6,19),(2,20)), bl(28,(6,15),(2,16))],
    [bl(20,(4,81),N), bl(30,(1,50),(4,51)), bl(28,(4,22),(4,23)), bl(24,(3,12),(8,13))],
    [bl(24,(2,92),(2,93)), bl(22,(6,36),(2,37)), bl(26,(4,20),(6,21)), bl(28,(7,14),(4,15))],
    [bl(26,(4,107),N), bl(22,(8,37),(1,38)), bl(24,(8,20),(4,21)), bl(22,(12,11),(4,12))],
    [bl(30,(3,115),(1,116)), bl(24,(4,40),(5,41)), bl(20,(11,16),(5,17)), bl(24,(11,12),(5,13))],
    [bl(22,(5,87),(1,88)), bl(24,(5,41),(5,42)), bl(30,(5,24),(7,25)), bl(24,(11,12),(7,13))],
    [bl(24,(5,98),(1,99)), bl(28,(7,45),(3,46)), bl(24,(15,19),(2,20)), bl(30,(3,15),(13,16))],
    [bl(28,(1,107),(5,108)), bl(28,(10,46),(1,47)), bl(28,(1,22),(15,23)), bl(28,(2,14),(17,15))],
    [bl(30,(5,120),(1,121)), bl(26,(9,43),(4,44)), bl(28,(17,22),(1,23)), bl(28,(2,14),(19,15))],
    [bl(28,(3,113),(4,114)), bl(26,(3,44),(11,45)), bl(26,(17,21),(4,22)), bl(26,(9,13),(16,14))],
    [bl(28,(3,107),(5,108)), bl(26,(3,41),(13,42)), bl(30,(15,24),(5,25)), bl(28,(15,15),(10,16))],
    [bl(28,(4,116),(4,117)), bl(26,(17,42),N), bl(28,(17,22),(6,23)), bl(30,(19,16),(6,17))],
    [bl(28,(2,111),(7,112)), bl(28,(17,46),N), bl(30,(7,24),(16,25)), bl(24,(34,13),N)],
    [bl(30,(4,121),(5,122)), bl(28,(4,47),(14,48)), bl(30,(11,24),(14,25)), bl(30,(16,15),(14,16))],
    [bl(30,(6,117),(4,118)), bl(28,(6,45),(14,46)), bl(30,(11,24),(16,25)), bl(30,(30,16),(2,17))],
    [bl(26,(8,106),(4,107)), bl(28,(8,47),(13,48)), bl(30,(7,24),(22,25)), bl(30,(22,15),(13,16))],
    [bl(28,(10,114),(2,115)), bl(28,(19,46),(4,47)), bl(28,(28,22),(6,23)), bl(30,(33,16),(4,17))],
    [bl(30,(8,122),(4,123)), bl(28,(22,45),(3,46)), bl(30,(8,23),(26,24)), bl(30,(12,15),(28,16))],
    [bl(30,(3,117),(10,118)), bl(28,(3,45),(23,46)), bl(30,(4,24),(31,25)), bl(30,(11,15),(31,16))],
    [bl(30,(7,116),(7,117)), bl(28,(21,45),(7,46)), bl(30,(1,23),(37,24)), bl(30,(19,15),(26,16))],
    [bl(30,(5,115),(10,116)), bl(28,(19,47),(10,48)), bl(30,(15,24),(25,25)), bl(30,(23,15),(25,16))],
    [bl(30,(13,115),(3,116)), bl(28,(2,46),(29,47)), bl(30,(42,24),(1,25)), bl(30,(23,15),(28,16))],
    [bl(30,(17,115),N), bl(28,(10,46),(23,47)), bl(30,(10,24),(35,25)), bl(30,(19,15),(35,16))],
    [bl(30,(17,115),(1,116)), bl(28,(14,46),(21,47)), bl(30,(29,24),(19,25)), bl(30,(11,15),(46,16))],
    [bl(30,(13,115),(6,116)), bl(28,(14,46),(23,47)), bl(30,(44,24),(7,25)), bl(30,(59,16),(1,17))],
    [bl(30,(12,121),(7,122)), bl(28,(12,47),(26,48)), bl(30,(39,24),(14,25)), bl(30,(22,15),(41,16))],
    [bl(30,(6,121),(14,122)), bl(28,(6,47),(34,48)), bl(30,(46,24),(10,25)), bl(30,(2,15),(64,16))],
    [bl(30,(17,122),(4,123)), bl(28,(29,46),(14,47)), bl(30,(49,24),(10,25)), bl(30,(24,15),(46,16))],
    [bl(30,(4,122),(18,123)), bl(28,(13,46),(32,47)), bl(30,(48,24),(14,25)), bl(30,(42,15),(32,16))],
    [bl(30,(20,117),(4,118)), bl(28,(40,47),(7,48)), bl(30,(43,24),(22,25)), bl(30,(10,15),(67,16))],
    [bl(30,(19,118),(6,119)), bl(28,(18,47),(31,48)), bl(30,(34,24),(34,25)), bl(30,(20,15),(61,16))],
];

pub(crate) fn block_layout(version: u8, level: EcLevel) -> Option<BlockLayout> {
    let row = LAYOUTS.get(usize::from(version).checked_sub(1)?)?;
    Some(
        row[match level {
            EcLevel::L => 0,
            EcLevel::M => 1,
            EcLevel::Q => 2,
            EcLevel::H => 3,
        }],
    )
}

/// Misdecode protection codewords reserved by ISO/IEC 18004 for small
/// symbols; correctable errors per block are `(ec - p) / 2`.
pub(crate) const fn misdecode_protection(version: u8, level: EcLevel) -> u8 {
    match (version, level) {
        (1, EcLevel::L) => 3,
        (1, EcLevel::M) => 2,
        (1, EcLevel::Q | EcLevel::H) => 1,
        (2, EcLevel::L) => 2,
        (3, EcLevel::L) => 1,
        _ => 0,
    }
}

impl BlockLayout {
    pub(crate) fn block_count(&self) -> usize {
        usize::from(self.groups[0].0) + usize::from(self.groups[1].0)
    }

    pub(crate) fn data_codewords(&self) -> usize {
        self.groups
            .iter()
            .map(|&(n, k)| usize::from(n) * usize::from(k))
            .sum()
    }

    pub(crate) fn total_codewords(&self) -> usize {
        self.data_codewords() + self.block_count() * usize::from(self.ec_per_block)
    }
}

/// Number of modules that carry codewords or remainder bits
/// (everything except function patterns, format and version information).
pub(crate) fn raw_data_modules(version: u8) -> usize {
    let v = usize::from(version);
    let mut n = (16 * v + 128) * v + 64;
    if v >= 2 {
        let a = v / 7 + 2;
        n -= (25 * a - 10) * a - 55;
        if v >= 7 {
            n -= 36;
        }
    }
    n
}

/// Total codewords (data + EC) of a version, independent of level.
pub(crate) fn total_codewords(version: u8) -> usize {
    raw_data_modules(version) / 8
}

/// Marks every function module (finders with separators, timing, alignment
/// patterns, format and version information, dark module).
pub(crate) fn function_mask(version: u8) -> BitMatrix {
    let d = dimension(version);
    let mut m = BitMatrix::new(d, d);
    let mut fill = |x0: usize, y0: usize, w: usize, h: usize| {
        for y in y0..(y0 + h).min(d) {
            for x in x0..(x0 + w).min(d) {
                m.set(x, y, true);
            }
        }
    };
    fill(0, 0, 9, 9);
    fill(d - 8, 0, 8, 9);
    fill(0, d - 8, 9, 8);
    fill(6, 0, 1, d);
    fill(0, 6, d, 1);
    let centers = alignment_centers(version);
    if let (Some(&first), Some(&last)) = (centers.first(), centers.last()) {
        for &cy in centers {
            for &cx in centers {
                let corner =
                    (cx == first && (cy == first || cy == last)) || (cx == last && cy == first);
                if !corner {
                    fill(usize::from(cx) - 2, usize::from(cy) - 2, 5, 5);
                }
            }
        }
    }
    if version >= 7 {
        fill(d - 11, 0, 3, 6);
        fill(0, d - 11, 6, 3);
    }
    m
}

/// Coordinates (x, y) of data modules in codeword placement order.
pub(crate) fn data_positions(version: u8) -> &'static [(usize, usize)] {
    static CACHE: [OnceLock<Vec<(usize, usize)>>; 40] = [const { OnceLock::new() }; 40];
    match usize::from(version)
        .checked_sub(1)
        .and_then(|i| CACHE.get(i))
    {
        Some(cell) => cell.get_or_init(|| build_data_positions(version)),
        None => &[],
    }
}

fn build_data_positions(version: u8) -> Vec<(usize, usize)> {
    let d = dimension(version);
    let func = function_mask(version);
    let mut out = Vec::with_capacity(raw_data_modules(version));
    let mut x = d - 1;
    let mut upward = true;
    loop {
        if x == 6 {
            x -= 1;
        }
        for i in 0..d {
            let y = if upward { d - 1 - i } else { i };
            for col in [x, x - 1] {
                if !func.get(col, y) {
                    out.push((col, y));
                }
            }
        }
        upward = !upward;
        if x < 3 {
            break;
        }
        x -= 2;
    }
    out
}

/// Format information bit positions (index = bit number, 0 = LSB) for the
/// copy around the top-left finder and the split copy.
pub(crate) fn format_positions(d: usize) -> [[(usize, usize); 15]; 2] {
    let mut a = [(0, 0); 15];
    let mut b = [(0, 0); 15];
    for i in 0..15 {
        a[i] = match i {
            0..=5 => (8, i),
            6 => (8, 7),
            7 => (8, 8),
            8 => (7, 8),
            _ => (14 - i, 8),
        };
        b[i] = if i < 8 {
            (d - 1 - i, 8)
        } else {
            (8, d - 15 + i)
        };
    }
    [a, b]
}

/// Version information bit positions (index = bit number, 0 = LSB) for the
/// block next to the top-right finder and the one next to the bottom-left.
pub(crate) fn version_positions(d: usize) -> [[(usize, usize); 18]; 2] {
    let mut a = [(0, 0); 18];
    let mut b = [(0, 0); 18];
    for i in 0..18 {
        a[i] = (d - 11 + i % 3, i / 3);
        b[i] = (i / 3, d - 11 + i % 3);
    }
    [a, b]
}

const fn bch_remainder(value: u32, poly: u32, poly_bits: u32) -> u32 {
    let mut rem = value << (poly_bits - 1);
    let mut bit = 31;
    while bit >= poly_bits - 1 {
        if rem & (1 << bit) != 0 {
            rem ^= poly << (bit - (poly_bits - 1));
        }
        if bit == 0 {
            break;
        }
        bit -= 1;
    }
    rem
}

/// The 32 valid format words (before the 0x5412 mask), indexed by the 5 data bits.
pub(crate) const FORMAT_WORDS: [u32; 32] = {
    let mut t = [0; 32];
    let mut i = 0;
    while i < 32 {
        t[i] = ((i as u32) << 10) | bch_remainder(i as u32, 0x537, 11);
        i += 1;
    }
    t
};

pub(crate) const FORMAT_MASK: u32 = 0x5412;

/// The valid version words for versions 7..=40 (index 0 = version 7).
pub(crate) const VERSION_WORDS: [u32; 34] = {
    let mut t = [0; 34];
    let mut i = 0;
    while i < 34 {
        let v = 7 + i as u32;
        t[i] = (v << 12) | bch_remainder(v, 0x1F25, 13);
        i += 1;
    }
    t
};

/// Two-bit EC level indicator used in the format information.
pub(crate) const fn level_from_bits(bits: u32) -> EcLevel {
    match bits & 3 {
        1 => EcLevel::L,
        0 => EcLevel::M,
        3 => EcLevel::Q,
        _ => EcLevel::H,
    }
}

/// Character count indicator width for a mode (0 numeric, 1 alphanumeric,
/// 2 byte, 3 kanji).
pub(crate) const fn count_bits(version: u8, mode: usize) -> usize {
    let class = if version <= 9 {
        0
    } else if version <= 26 {
        1
    } else {
        2
    };
    const T: [[usize; 3]; 4] = [[10, 12, 14], [9, 11, 13], [8, 16, 16], [8, 10, 12]];
    T[mode][class]
}

pub(crate) const ALNUM: &[u8; 45] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

#[cfg(test)]
mod tests {
    use super::*;

    const LEVELS: [EcLevel; 4] = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H];

    #[test]
    fn layouts_match_capacity() {
        for v in 1..=40u8 {
            for l in LEVELS {
                let b = block_layout(v, l).unwrap();
                assert_eq!(b.total_codewords(), total_codewords(v), "v{v} {l:?}");
                // block data sizes must be non-decreasing between groups
                if b.groups[1].0 > 0 {
                    assert_eq!(b.groups[1].1, b.groups[0].1 + 1, "v{v} {l:?}");
                }
            }
        }
        assert!(block_layout(0, EcLevel::L).is_none());
        assert!(block_layout(41, EcLevel::L).is_none());
    }

    #[test]
    fn known_codewords() {
        assert_eq!(total_codewords(1), 26);
        assert_eq!(total_codewords(7), 196);
        assert_eq!(total_codewords(40), 3706);
        assert_eq!(raw_data_modules(1), 208);
        assert_eq!(raw_data_modules(40), 29648);
    }

    #[test]
    fn data_positions_cover_raw_modules() {
        for v in 1..=40u8 {
            assert_eq!(data_positions(v).len(), raw_data_modules(v), "v{v}");
        }
    }

    #[test]
    fn bch_tables() {
        assert_eq!(FORMAT_WORDS[0], 0);
        // level L, mask 0 is the well-known 0x77C4
        assert_eq!(FORMAT_WORDS[0b01_000] ^ FORMAT_MASK, 0x77C4);
        assert_eq!(VERSION_WORDS[0], 0x07C94);
        assert_eq!(VERSION_WORDS[1], 0x085BC);
        assert_eq!(VERSION_WORDS[33], 0x28C69);
        let mut min = u32::MAX;
        for (i, a) in FORMAT_WORDS.iter().enumerate() {
            for b in &FORMAT_WORDS[i + 1..] {
                min = min.min((a ^ b).count_ones());
            }
        }
        assert_eq!(min, 7);
    }
}
