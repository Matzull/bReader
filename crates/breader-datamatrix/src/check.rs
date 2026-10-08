//! Cheap plausibility checks on a sampled module grid, run before Reed-Solomon.

use breader_core::BitMatrix;

use crate::sample::Hypothesis;

/// Pattern mismatch counters of a sampled grid.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Mismatch {
    /// Wrong modules on the outer L (left column, bottom row).
    pub outer_l: u32,
    pub outer_l_total: u32,
    /// Wrong modules on the outer clock tracks (top row, right column).
    pub outer_clock: u32,
    pub outer_clock_total: u32,
    /// Wrong modules over all finder, clock and alignment patterns.
    pub all: u32,
    pub all_total: u32,
}

/// Compares the grid with the finder, clock and alignment patterns of the
/// hypothesis. The grid must have the hypothesis' sampled dimensions.
pub(crate) fn mismatch(g: &BitMatrix, hyp: &Hypothesis) -> Mismatch {
    let (rows, cols) = hyp.grid_dims();
    let (rv, rh, rr, rc) = hyp.regions();
    let mut m = Mismatch::default();
    if g.width() != cols || g.height() != rows {
        m.all = u32::MAX;
        return m;
    }
    for i in 0..rv {
        for j in 0..rh {
            let (y0, x0) = (i * (rr + 2), j * (rc + 2));
            let (y1, x1) = (y0 + rr + 1, x0 + rc + 1);
            // Left column and bottom row: solid.
            for y in y0..=y1 {
                let bad = u32::from(!g.get(x0, y));
                m.all += bad;
                m.all_total += 1;
                if j == 0 {
                    m.outer_l += bad;
                    m.outer_l_total += 1;
                }
            }
            for x in x0 + 1..=x1 {
                let bad = u32::from(!g.get(x, y1));
                m.all += bad;
                m.all_total += 1;
                if i + 1 == rv {
                    m.outer_l += bad;
                    m.outer_l_total += 1;
                }
            }
            // Top row (dark at even offsets) and right column (dark at even
            // distance from the bottom).
            for x in x0..=x1 {
                let bad = u32::from(g.get(x, y0) != ((x - x0) % 2 == 0));
                m.all += bad;
                m.all_total += 1;
                if i == 0 {
                    m.outer_clock += bad;
                    m.outer_clock_total += 1;
                }
            }
            for y in y0..y1 {
                let bad = u32::from(g.get(x1, y) != ((y1 - y) % 2 == 0));
                m.all += bad;
                m.all_total += 1;
                if j + 1 == rh {
                    m.outer_clock += bad;
                    m.outer_clock_total += 1;
                }
            }
        }
    }
    m
}

/// Whether the sampled grid looks like a Data Matrix of the hypothesis.
pub(crate) fn plausible(g: &BitMatrix, hyp: &Hypothesis) -> bool {
    let m = mismatch(g, hyp);
    if m.all == u32::MAX {
        return false;
    }
    m.outer_l * 100 <= m.outer_l_total * 15
        && m.outer_clock * 100 <= m.outer_clock_total * 25
        && m.all * 100 <= m.all_total * 20
}
