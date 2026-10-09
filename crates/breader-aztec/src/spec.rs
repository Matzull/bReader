//! Aztec Code geometry and capacity tables (ISO/IEC 24778): layer counts,
//! codeword sizes, mode message layout, reference grid and the spiral order
//! of the data modules.
//!
//! Coordinates are module columns and rows from the top-left corner of the
//! symbol, in the orientation where the bullseye orientation marks are three
//! dark modules at the top-left corner of the mode message ring, two at the
//! top-right, one at the bottom-right and none at the bottom-left.

/// Side of an Aztec Rune (a compact symbol without data layers).
pub(crate) const RUNE_SIZE: usize = 11;
/// Side of the core around the center that holds the bullseye, the
/// orientation marks and the mode message of either kind of symbol.
pub(crate) const CORE_SIZE: usize = 15;
/// Largest layer count of a compact symbol.
pub(crate) const MAX_COMPACT_LAYERS: usize = 4;
/// Largest layer count of a full-range symbol.
pub(crate) const MAX_FULL_LAYERS: usize = 32;
/// Fewest check codewords the standard allows.
pub(crate) const MIN_CHECK_WORDS: usize = 3;
/// Mode message check words: compact 5, full-range 6.
pub(crate) const COMPACT_MODE_CHECK: usize = 5;
pub(crate) const FULL_MODE_CHECK: usize = 6;
/// Pattern XORed onto the 28-bit mode message of an Aztec Rune: 1010... .
pub(crate) const RUNE_MASK: u32 = 0xAAA_AAAA;

/// Compact or full-range symbol with its number of data layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Layout {
    pub compact: bool,
    pub layers: usize,
}

impl Layout {
    /// All layouts whose symbol has `size` modules per side (a compact and a
    /// full-range symbol can share a size).
    pub(crate) fn for_size(size: usize) -> impl Iterator<Item = Layout> {
        let compact = (1..=MAX_COMPACT_LAYERS).map(|layers| Layout {
            compact: true,
            layers,
        });
        let full = (1..=MAX_FULL_LAYERS).map(|layers| Layout {
            compact: false,
            layers,
        });
        compact.chain(full).filter(move |l| l.size() == size)
    }

    /// Side length without the reference grid lines.
    pub(crate) fn base_size(self) -> usize {
        (if self.compact { 11 } else { 14 }) + 4 * self.layers
    }

    /// Side length of the symbol in modules.
    pub(crate) fn size(self) -> usize {
        let base = self.base_size();
        if self.compact {
            base
        } else {
            base + 1 + 2 * ((base / 2 - 1) / 15)
        }
    }

    /// Codeword size in bits.
    pub(crate) fn word_size(self) -> usize {
        match self.layers {
            0..=2 => 6,
            3..=8 => 8,
            9..=22 => 10,
            _ => 12,
        }
    }

    /// Modules of the data layers.
    pub(crate) fn total_bits(self) -> usize {
        (if self.compact { 88 } else { 112 } + 16 * self.layers) * self.layers
    }

    /// Whole codewords of the data layers.
    pub(crate) fn total_codewords(self) -> usize {
        self.total_bits() / self.word_size()
    }

    /// Modules at the start of the spiral that belong to no codeword.
    pub(crate) fn start_pad(self) -> usize {
        self.total_bits() % self.word_size()
    }
}

/// Offsets (dx, dy) from the center module of the mode message bits, in
/// message order: clockwise from the top-left, four sides of 7 (compact) or
/// 10 (full-range, skipping the reference grid line through the center)
/// modules. The corners of the ring hold the orientation marks.
pub(crate) fn mode_positions(compact: bool) -> Vec<(i32, i32)> {
    let mut out = Vec::new();
    if compact {
        for i in 0..7 {
            out.push((i - 3, -5));
        }
        for i in 0..7 {
            out.push((5, i - 3));
        }
        for i in (0..7).rev() {
            out.push((i - 3, 5));
        }
        for i in (0..7).rev() {
            out.push((-5, i - 3));
        }
    } else {
        let o = |i: i32| i - 5 + i / 5;
        for i in 0..10 {
            out.push((o(i), -7));
        }
        for i in 0..10 {
            out.push((7, o(i)));
        }
        for i in (0..10).rev() {
            out.push((o(i), 7));
        }
        for i in (0..10).rev() {
            out.push((-7, o(i)));
        }
    }
    out
}

/// Whether the module at (`x`, `y`) of a full-range symbol of side `size`
/// lies on a reference grid line (lines through the center and every 16
/// modules from it).
pub(crate) fn on_grid_line(size: usize, x: usize, y: usize) -> bool {
    let c = size / 2;
    x.abs_diff(c).is_multiple_of(16) || y.abs_diff(c).is_multiple_of(16)
}

/// The orientation marks around the mode ring at Chebyshev distance `r`
/// from the center (5 compact, 7 full-range): module offsets and whether
/// they are dark. Three dark modules at the top-left corner, two at the
/// top-right, one at the bottom-right, none at the bottom-left.
pub(crate) fn orientation_marks(r: i32) -> [(i32, i32, bool); 12] {
    [
        (-r, -r, true),
        (-r + 1, -r, true),
        (-r, -r + 1, true),
        (r, -r, true),
        (r, -r + 1, true),
        (r - 1, -r, false),
        (r, r - 1, true),
        (r, r, false),
        (r - 1, r, false),
        (-r, r, false),
        (-r + 1, r, false),
        (-r, r - 1, false),
    ]
}

/// Dark module of the bullseye at Chebyshev distance `d` from the center:
/// even rings are dark.
pub(crate) fn bullseye_dark(d: usize) -> bool {
    d.is_multiple_of(2)
}

/// Map from the index on the grid-free base lattice to the module coordinate:
/// the identity for compact symbols; for full-range symbols the lattice is
/// stretched around the center line and every 16th line.
fn lattice_map(layout: Layout) -> Vec<usize> {
    let base = layout.base_size();
    let mut map: Vec<usize> = (0..base).collect();
    if !layout.compact {
        let half = base / 2;
        let center = layout.size() / 2;
        for i in 0..half {
            let offset = i + i / 15;
            map[half - i - 1] = center - offset - 1;
            map[half + i] = center + offset + 1;
        }
    }
    map
}

/// Module coordinates (x, y) of every bit of the data layers, in spiral
/// order: layers from the outermost inwards; per layer four sides (left going
/// down, bottom going right, right going up, top going left), each made of
/// pairs of modules across the two-module-wide layer, the outer module
/// first. The first `start_pad` bits belong to no codeword.
pub(crate) fn data_positions(layout: Layout) -> Vec<(usize, usize)> {
    let map = lattice_map(layout);
    let n = layout.base_size();
    let mut out = Vec::with_capacity(layout.total_bits());
    for i in 0..layout.layers {
        let row_size = (layout.layers - i) * 4 + if layout.compact { 9 } else { 12 };
        let q = i * 2;
        for side in 0..4 {
            for j in 0..row_size {
                for k in 0..2 {
                    let (x, y) = match side {
                        0 => (q + k, q + j),
                        1 => (q + j, n - 1 - q - k),
                        2 => (n - 1 - q - k, n - 1 - q - j),
                        _ => (n - 1 - q - j, q + k),
                    };
                    out.push((map[x], map[y]));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_capacities() {
        let c = |layers| Layout {
            compact: true,
            layers,
        };
        let f = |layers| Layout {
            compact: false,
            layers,
        };
        assert_eq!([15, 19, 23, 27], [1, 2, 3, 4].map(|l| c(l).size()));
        assert_eq!(19, f(1).size());
        assert_eq!(31, f(4).size());
        assert_eq!(75, f(14).size());
        assert_eq!(151, f(32).size());
        assert_eq!(104, c(1).total_bits());
        assert_eq!(17, c(1).total_codewords());
        assert_eq!(128, f(1).total_bits());
        assert_eq!(21, f(1).total_codewords());
        assert_eq!(19968, f(32).total_bits());
        assert_eq!(1664, f(32).total_codewords());
        assert_eq!(
            [6, 6, 8, 8, 10, 12],
            [1, 2, 3, 8, 22, 23].map(|l| f(l).word_size())
        );
        // The widest symbol of each field never exceeds the field's length.
        assert!(f(2).total_codewords() <= 63);
        assert!(f(8).total_codewords() <= 255);
        assert!(f(22).total_codewords() <= 1023);
    }

    #[test]
    fn size_lookup() {
        for size in [15, 19, 23, 27, 31, 75, 151] {
            assert!(Layout::for_size(size).count() >= 1, "{size}");
        }
        assert_eq!(Layout::for_size(19).count(), 2);
        assert_eq!(Layout::for_size(11).count(), 0);
        assert_eq!(Layout::for_size(21).count(), 0);
    }

    #[test]
    fn positions_are_a_permutation_of_the_data_modules() {
        for compact in [true, false] {
            let max = if compact {
                MAX_COMPACT_LAYERS
            } else {
                MAX_FULL_LAYERS
            };
            for layers in 1..=max {
                let layout = Layout { compact, layers };
                let size = layout.size();
                let pos = data_positions(layout);
                assert_eq!(pos.len(), layout.total_bits());
                let mut seen = vec![false; size * size];
                let c = size / 2;
                let core = if compact { 5 } else { 7 };
                for &(x, y) in &pos {
                    assert!(x < size && y < size);
                    assert!(!seen[y * size + x], "duplicate {x},{y}");
                    seen[y * size + x] = true;
                    assert!(x.abs_diff(c).max(y.abs_diff(c)) > core);
                    assert!(compact || !on_grid_line(size, x, y));
                }
                // Everything outside the core and the grid is data.
                let mut expected = 0;
                for y in 0..size {
                    for x in 0..size {
                        let outside = x.abs_diff(c).max(y.abs_diff(c)) > core;
                        if outside && (compact || !on_grid_line(size, x, y)) {
                            expected += 1;
                        }
                    }
                }
                assert_eq!(expected, pos.len());
            }
        }
    }

    #[test]
    fn mode_ring_has_no_duplicates() {
        for compact in [true, false] {
            let pos = mode_positions(compact);
            assert_eq!(pos.len(), if compact { 28 } else { 40 });
            let mut v = pos.clone();
            v.sort_unstable();
            v.dedup();
            assert_eq!(v.len(), pos.len());
            let r = if compact { 5 } else { 7 };
            assert!(pos.iter().all(|&(dx, dy)| dx.abs().max(dy.abs()) == r));
        }
    }
}
