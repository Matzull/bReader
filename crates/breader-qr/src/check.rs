//! Cheap plausibility checks on a sampled module grid, run before Reed-Solomon.

use breader_core::BitMatrix;

/// Mismatches against the three finder patterns plus separators (8x8 each,
/// 192 modules in total).
pub(crate) fn finder_mismatch(g: &BitMatrix) -> u32 {
    let d = g.width();
    let mut bad = 0;
    for (ox, oy) in [(0, 0), (d - 7, 0), (0, d - 7)] {
        for j in 0..7usize {
            for i in 0..7usize {
                let r = (i as i32 - 3).abs().max((j as i32 - 3).abs());
                let want = r != 2;
                bad += u32::from(g.get(ox + i, oy + j) != want);
            }
        }
    }
    // Separators: light strips beside the finders.
    for k in 0..8usize {
        for (x, y) in [
            (7, k),
            (k, 7),
            (d - 8, k),
            (d - 8 + k, 7),
            (7, d - 8 + k),
            (k, d - 8),
        ] {
            bad += u32::from(g.get(x, y));
        }
    }
    bad
}

/// Mismatches of row 6 and column 6 against the alternating timing pattern
/// between the finders; returns (mismatches, modules checked).
pub(crate) fn timing_mismatch(g: &BitMatrix) -> (u32, u32) {
    let d = g.width();
    let mut bad = 0;
    for i in 8..d - 8 {
        let want = i % 2 == 0;
        bad += u32::from(g.get(i, 6) != want);
        bad += u32::from(g.get(6, i) != want);
    }
    (bad, 2 * (d as u32 - 16))
}
