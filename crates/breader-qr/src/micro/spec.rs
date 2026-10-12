//! Micro QR Code constants from ISO/IEC 18004:2015 (symbol versions M1 to M4).

use breader_core::EcLevel;

/// Modules per side: 11, 13, 15 or 17 for versions 1 to 4.
#[cfg(test)]
pub(crate) const fn dimension(version: u8) -> usize {
    9 + 2 * version as usize
}

/// Version for a module count, if it is a Micro QR dimension.
pub(crate) const fn version_for_dimension(dim: usize) -> Option<u8> {
    if dim >= 11 && dim <= 17 && dim % 2 == 1 {
        Some(((dim - 9) / 2) as u8)
    } else {
        None
    }
}

/// One of the eight symbol numbers carried by the format information.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Variant {
    pub version: u8,
    pub level: EcLevel,
    /// Data capacity in bits; M1 and M3 end in a half (4-bit) codeword.
    pub data_bits: usize,
    /// Error correction codewords.
    pub ec: usize,
}

impl Variant {
    pub fn data_codewords(&self) -> usize {
        self.data_bits.div_ceil(8)
    }
}

const fn v(version: u8, level: EcLevel, data_bits: usize, ec: usize) -> Variant {
    Variant {
        version,
        level,
        data_bits,
        ec,
    }
}

/// Indexed by the 3-bit symbol number of the format information.
pub(crate) const VARIANTS: [Variant; 8] = [
    v(1, EcLevel::DetectionOnly, 20, 2),
    v(2, EcLevel::L, 40, 5),
    v(2, EcLevel::M, 32, 6),
    v(3, EcLevel::L, 84, 6),
    v(3, EcLevel::M, 68, 8),
    v(4, EcLevel::L, 128, 8),
    v(4, EcLevel::M, 112, 10),
    v(4, EcLevel::Q, 80, 14),
];

/// XOR applied to the BCH(15,5) format word.
pub(crate) const FORMAT_MASK: u32 = 0x4445;

/// Whether a module is part of a function pattern (finder with separator,
/// timing patterns, format information).
pub(crate) const fn is_function(x: usize, y: usize) -> bool {
    (x < 8 && y < 8)
        || (y == 0 && x >= 8)
        || (x == 0 && y >= 8)
        || (y == 8 && x >= 1 && x <= 8)
        || (x == 8 && y >= 1 && y <= 7)
}

/// Format information bit positions (index = bit number, 0 = LSB), `(x, y)`.
pub(crate) const FORMAT_POSITIONS: [(usize, usize); 15] = {
    let mut p = [(0, 0); 15];
    let mut i = 0;
    while i < 15 {
        p[i] = if i < 7 { (8, i + 1) } else { (15 - i, 8) };
        i += 1;
    }
    p
};

/// Data module positions in codeword order for a symbol of `dim` modules:
/// two-module-wide columns from the right edge, alternately upwards and
/// downwards, function modules skipped.
pub(crate) fn data_positions(dim: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::with_capacity(dim * dim);
    let mut up = true;
    let mut x = dim - 1;
    while x >= 1 {
        for k in 0..dim {
            let y = if up { dim - 1 - k } else { k };
            for dx in 0..2 {
                if !is_function(x - dx, y) {
                    out.push((x - dx, y));
                }
            }
        }
        up = !up;
        if x < 2 {
            break;
        }
        x -= 2;
    }
    out
}

/// Data mask pattern `mask` (0 to 3) at row `i`, column `j`.
pub(crate) const fn mask_bit(mask: u8, i: usize, j: usize) -> bool {
    match mask {
        0 => i.is_multiple_of(2),
        1 => (i / 2 + j / 3).is_multiple_of(2),
        2 => ((i * j) % 2 + (i * j) % 3).is_multiple_of(2),
        _ => ((i + j) % 2 + (i * j) % 3).is_multiple_of(2),
    }
}

/// Character count indicator width for a mode (0 numeric, 1 alphanumeric,
/// 2 byte, 3 kanji); 0 when the version does not have the mode.
pub(crate) const fn count_bits(version: u8, mode: usize) -> usize {
    const T: [[usize; 4]; 4] = [[3, 0, 0, 0], [4, 3, 0, 0], [5, 4, 4, 3], [6, 5, 5, 4]];
    T[version as usize - 1][mode]
}

/// Mode indicator width in bits.
pub(crate) const fn mode_bits(version: u8) -> usize {
    version as usize - 1
}

/// Terminator length in bits.
pub(crate) const fn terminator_bits(version: u8) -> usize {
    2 * version as usize + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_module_counts() {
        for (version, bits) in [(1u8, 36usize), (2, 80), (3, 132), (4, 192)] {
            let d = dimension(version);
            assert_eq!(data_positions(d).len(), bits, "M{version}");
            let funcs = (0..d * d).filter(|i| is_function(i % d, i / d)).count();
            assert_eq!(funcs + bits, d * d);
        }
    }

    #[test]
    fn capacities_fill_the_symbol() {
        for var in VARIANTS {
            let raw = data_positions(dimension(var.version)).len();
            let coded = var.data_bits + 8 * var.ec;
            assert_eq!(raw, coded, "{var:?}");
        }
    }
}
