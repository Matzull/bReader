//! Codeword placement in the mapping matrix (ISO/IEC 16022 Annex F).

use crate::spec::{self, SIZE_COUNT, SymbolSize};
use std::sync::OnceLock;

/// Mapping-matrix coordinates (row, column) of one module.
pub(crate) type Pos = (u16, u16);

/// Where the codeword bits live in a mapping matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Placement {
    pub rows: usize,
    pub cols: usize,
    /// Per codeword, the position of bits 8 (most significant) down to 1.
    pub words: Vec<[Pos; 8]>,
    /// Modules not covered by any codeword (the fixed bottom-right pattern
    /// when the area is not a multiple of 8); empty otherwise.
    pub unused: Vec<Pos>,
}

struct Walker {
    nrow: i32,
    ncol: i32,
    /// Index of the codeword owning each module, or `usize::MAX`.
    owner: Vec<usize>,
    words: Vec<[Pos; 8]>,
}

impl Walker {
    fn filled(&self, row: i32, col: i32) -> bool {
        if row < 0 || col < 0 || row >= self.nrow || col >= self.ncol {
            return true;
        }
        self.owner[(row * self.ncol + col) as usize] != usize::MAX
    }

    /// Places bit `bit` (1 = most significant) of the current codeword.
    fn module(&mut self, mut row: i32, mut col: i32, bit: usize) {
        if row < 0 {
            row += self.nrow;
            col += 4 - ((self.nrow + 4) % 8);
        }
        if col < 0 {
            col += self.ncol;
            row += 4 - ((self.ncol + 4) % 8);
        }
        // When the matrix height is a multiple of 8 and the width is not,
        // the wrapped utah lands one pattern past the edge (DMRE 26x40 and
        // 26x48): it continues at the opposite side.
        if row >= self.nrow {
            row -= self.nrow;
        }
        if col >= self.ncol {
            col -= self.ncol;
        }
        if row < 0 || col < 0 || row >= self.nrow || col >= self.ncol {
            return;
        }
        let word = self.words.len() - 1;
        self.owner[(row * self.ncol + col) as usize] = word;
        self.words[word][bit - 1] = (row as u16, col as u16);
    }

    fn begin(&mut self) {
        self.words.push([(0, 0); 8]);
    }

    fn utah(&mut self, row: i32, col: i32) {
        self.begin();
        self.module(row - 2, col - 2, 1);
        self.module(row - 2, col - 1, 2);
        self.module(row - 1, col - 2, 3);
        self.module(row - 1, col - 1, 4);
        self.module(row - 1, col, 5);
        self.module(row, col - 2, 6);
        self.module(row, col - 1, 7);
        self.module(row, col, 8);
    }

    fn corner(&mut self, cells: [(i32, i32); 8]) {
        self.begin();
        for (i, (r, c)) in cells.into_iter().enumerate() {
            self.module(r, c, i + 1);
        }
    }
}

/// Computes the placement for a `nrow` x `ncol` mapping matrix.
pub(crate) fn compute(nrow: usize, ncol: usize) -> Placement {
    let (nr, nc) = (nrow as i32, ncol as i32);
    let mut w = Walker {
        nrow: nr,
        ncol: nc,
        owner: vec![usize::MAX; nrow * ncol],
        words: Vec::with_capacity(nrow * ncol / 8),
    };
    let (mut row, mut col) = (4i32, 0i32);
    // Every pass advances the walk; the cap only guards degenerate sizes.
    for _ in 0..(nrow + ncol + 16) * 4 {
        if row == nr && col == 0 {
            w.corner([
                (nr - 1, 0),
                (nr - 1, 1),
                (nr - 1, 2),
                (0, nc - 2),
                (0, nc - 1),
                (1, nc - 1),
                (2, nc - 1),
                (3, nc - 1),
            ]);
        }
        if row == nr - 2 && col == 0 && nc % 4 != 0 {
            w.corner([
                (nr - 3, 0),
                (nr - 2, 0),
                (nr - 1, 0),
                (0, nc - 4),
                (0, nc - 3),
                (0, nc - 2),
                (0, nc - 1),
                (1, nc - 1),
            ]);
        }
        if row == nr - 2 && col == 0 && nc % 8 == 4 {
            w.corner([
                (nr - 3, 0),
                (nr - 2, 0),
                (nr - 1, 0),
                (0, nc - 2),
                (0, nc - 1),
                (1, nc - 1),
                (2, nc - 1),
                (3, nc - 1),
            ]);
        }
        if row == nr + 4 && col == 2 && nc % 8 == 0 {
            w.corner([
                (nr - 1, 0),
                (nr - 1, nc - 1),
                (0, nc - 3),
                (0, nc - 2),
                (0, nc - 1),
                (1, nc - 3),
                (1, nc - 2),
                (1, nc - 1),
            ]);
        }
        loop {
            if row < nr && col >= 0 && !w.filled(row, col) {
                w.utah(row, col);
            }
            row -= 2;
            col += 2;
            if !(row >= 0 && col < nc) {
                break;
            }
        }
        row += 1;
        col += 3;
        loop {
            if row >= 0 && col < nc && !w.filled(row, col) {
                w.utah(row, col);
            }
            row += 2;
            col -= 2;
            if !(row < nr && col >= 0) {
                break;
            }
        }
        row += 3;
        col += 1;
        if !(row < nr || col < nc) {
            break;
        }
    }
    let mut unused = Vec::new();
    for r in 0..nrow {
        for c in 0..ncol {
            if w.owner[r * ncol + c] == usize::MAX {
                unused.push((r as u16, c as u16));
            }
        }
    }
    Placement {
        rows: nrow,
        cols: ncol,
        words: w.words,
        unused,
    }
}

static CACHE: [OnceLock<Placement>; SIZE_COUNT] = [const { OnceLock::new() }; SIZE_COUNT];

/// Cached placement for one of the table sizes.
pub(crate) fn placement(size: &SymbolSize) -> Option<&'static Placement> {
    let idx = spec::index(size.rows, size.cols)?;
    let (r, c) = size.mapping();
    Some(CACHE[idx].get_or_init(|| compute(r, c)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn covers_every_size() {
        for s in spec::sizes() {
            let p = placement(s).unwrap();
            let (r, c) = s.mapping();
            assert_eq!(p.words.len(), s.total_codewords(), "{}x{}", s.rows, s.cols);
            assert_eq!(p.unused.len(), r * c - 8 * p.words.len());
            if r * c % 8 == 4 {
                let (r, c) = (r as u16, c as u16);
                let mut u = p.unused.clone();
                u.sort_unstable();
                assert_eq!(
                    u,
                    vec![
                        (r - 2, c - 2),
                        (r - 2, c - 1),
                        (r - 1, c - 2),
                        (r - 1, c - 1)
                    ]
                );
            }
            let mut seen = vec![false; r * c];
            for w in &p.words {
                for &(y, x) in w {
                    let k = usize::from(y) * c + usize::from(x);
                    assert!(!seen[k], "module reused in {}x{}", s.rows, s.cols);
                    seen[k] = true;
                }
            }
        }
    }

    #[test]
    fn first_codewords_of_10x10() {
        // Mapping 8x8: the first codeword is the utah shape around (4,0)
        // wrapped to the top-left.
        let p = compute(8, 8);
        assert_eq!(p.words.len(), 8);
        assert_eq!(p.words[0][7], (4, 0));
    }
}
