//! Bullseye search: seven equal runs around a dark center on rows of the
//! binary image, confirmed on the column and both diagonals.

use breader_core::{BitMatrix, Plane, Point};

/// Runs around the center module: three light/dark/light on each side, all
/// one module wide.
const RUNS: usize = 7;
/// Each run within this percentage of the mean run (rows).
const ROW_TOL_PERCENT: u32 = 50;
/// Same for the lines through the center.
const LINE_TOL_PERCENT: u32 = 80;
/// Same on the diagonals, where the binarized edges carry more jitter.
const DIAG_TOL_PERCENT: u32 = 95;
/// Narrowest bullseye (the seven runs) worth looking at, in pixels.
const MIN_WIDTH: u32 = 14;
/// Candidates kept per search.
const MAX_CANDIDATES: usize = 256;
/// Row hits confirmed per search; the central row of a full-range symbol
/// alone offers one per module of its reference grid line.
const MAX_CONFIRMS: usize = 16_000;

/// A confirmed bullseye, in binary-plane coordinates.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Candidate {
    pub pos: Point,
    /// Module size estimate in pixels (mean over the horizontal and
    /// vertical lines; a rotated symbol reads up to 1.41 times larger).
    pub module: f32,
    /// Light-on-dark symbol: the center is light in the raw matrix.
    pub inv: bool,
    pub hits: u32,
}

/// Seven equal runs on a row: where their center is and how wide they are.
#[derive(Debug, Clone, Copy)]
struct RowHit {
    inv: bool,
    hx: f32,
    total: u32,
}

#[derive(Debug, Default)]
pub(crate) struct BullseyeSearch {
    bounds: Vec<u32>,
    hits: Vec<RowHit>,
    /// Hits of the previous and of the current row that failed to confirm.
    rejected_prev: Vec<RowHit>,
    rejected_cur: Vec<RowHit>,
    /// Position in `rejected_prev` of the hit being looked up (hits of a row
    /// come in increasing order).
    cursor: usize,
    pub candidates: Vec<Candidate>,
    /// Row hits sent to the line checks.
    pub confirms: usize,
}

/// The seven runs are equal within `tol_pct` percent of their mean, or
/// within one pixel of it (narrow runs jitter by a pixel when binarized).
pub(crate) fn bullseye_ratio(runs: &[u32; RUNS], tol_pct: u32) -> bool {
    let total: u32 = runs.iter().sum();
    if total == 0 {
        return false;
    }
    runs.iter().all(|&r| {
        let d = (7 * r).abs_diff(total);
        d * 100 <= tol_pct * total || d <= 7
    })
}

/// Run boundaries of row `y`: 0, every color change, the width.
fn row_bounds(m: &BitMatrix, y: usize, bounds: &mut Vec<u32>) {
    let width = m.width();
    bounds.clear();
    bounds.push(0);
    let words = m.row_words(y);
    let mut prev = words.first().map_or(0, |w| w & 1);
    for (wi, &wd) in words.iter().enumerate() {
        let mut t = wd ^ ((wd << 1) | prev);
        prev = wd >> 63;
        while t != 0 {
            let pos = wi * 64 + t.trailing_zeros() as usize;
            if pos >= width {
                break;
            }
            bounds.push(pos as u32);
            t &= t - 1;
        }
    }
    bounds.push(width as u32);
}

/// Appends the bullseye-like run sequences of row `y` to `hits`; with
/// `both` the light-centered ones too.
fn row_hits(m: &BitMatrix, y: usize, both: bool, bounds: &mut Vec<u32>, hits: &mut Vec<RowHit>) {
    row_bounds(m, y, bounds);
    if bounds.len() < RUNS + 1 {
        return;
    }
    // Run k is dark in the raw matrix when `dark0 ^ (k & 1)`.
    let dark0 = m.get(0, y);
    for (s, b) in bounds.windows(RUNS + 1).enumerate() {
        // The center run is s + 3: dark for a dark-on-light bullseye.
        let center_dark = dark0 ^ ((s + 3) & 1 == 1);
        if !center_dark && !both {
            continue;
        }
        let (r2, r3, r4) = (b[3] - b[2], b[4] - b[3], b[5] - b[4]);
        if r2 > 3 * r3 || r3 > 3 * r2 || r4 > 3 * r3 || r3 > 3 * r4 {
            continue;
        }
        let total = b[RUNS] - b[0];
        if total < MIN_WIDTH {
            continue;
        }
        // The dark rings beyond the window are at least about a module wide.
        if s == 0 || s + RUNS + 1 >= bounds.len() {
            continue;
        }
        let (before, after) = (b[0] - bounds[s - 1], bounds[s + RUNS + 1] - b[RUNS]);
        if 14 * before.min(after) < total {
            continue;
        }
        let runs: [u32; RUNS] = std::array::from_fn(|i| b[i + 1] - b[i]);
        if bullseye_ratio(&runs, ROW_TOL_PERCENT) {
            hits.push(RowHit {
                inv: !center_dark,
                hx: (b[3] + b[4]) as f32 / 2.0,
                total,
            });
        }
    }
}

fn in_bounds(m: Plane<'_>, x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && (x as usize) < m.width() && (y as usize) < m.height()
}

/// Length of the run of `dark` pixels from (x, y) along `d`, and whether the
/// walk ended at the image border or at `limit`.
fn count_run(
    m: Plane<'_>,
    mut x: i32,
    mut y: i32,
    d: (i32, i32),
    dark: bool,
    limit: u32,
) -> (u32, bool) {
    let mut n = 0;
    loop {
        if !in_bounds(m, x, y) || n >= limit {
            return (n, true);
        }
        if m.get(x as usize, y as usize) != dark {
            return (n, false);
        }
        n += 1;
        x += d.0;
        y += d.1;
    }
}

/// The seven runs through the dark pixel (x, y) along `d`, and how far the
/// center of the middle run lies from (x, y) along `d`, times two. `None`
/// when a run reaches the image border or `limit`.
fn line_runs(
    m: Plane<'_>,
    x: i32,
    y: i32,
    d: (i32, i32),
    limit: u32,
) -> Option<([u32; RUNS], i32)> {
    if !in_bounds(m, x, y) || !m.get(x as usize, y as usize) {
        return None;
    }
    let walk = |mut cx: i32, mut cy: i32, d: (i32, i32)| -> Option<[u32; 4]> {
        let mut out = [0u32; 4];
        let mut dark = true;
        for slot in &mut out {
            let (n, stopped) = count_run(m, cx, cy, d, dark, limit);
            if stopped {
                return None;
            }
            *slot = n;
            cx += d.0 * n as i32;
            cy += d.1 * n as i32;
            dark = !dark;
        }
        Some(out)
    };
    let p = walk(x, y, d)?;
    let q = walk(x - d.0, y - d.1, (-d.0, -d.1))?;
    // q[0] counts the center run before the start pixel.
    let runs = [q[3], q[2], q[1], p[0] + q[0], p[1], p[2], p[3]];
    Some((runs, p[0] as i32 - q[0] as i32))
}

impl BullseyeSearch {
    /// Scans every `row_step`-th row of the binary matrix; `both` also
    /// looks for light-on-dark bullseyes.
    pub fn find(&mut self, m: &BitMatrix, row_step: usize, both: bool) {
        self.candidates.clear();
        self.rejected_prev.clear();
        self.confirms = 0;
        let step = row_step.max(1);
        let mut bounds = std::mem::take(&mut self.bounds);
        let mut hits = std::mem::take(&mut self.hits);
        for y in (step / 2..m.height()).step_by(step) {
            hits.clear();
            row_hits(m, y, both, &mut bounds, &mut hits);
            std::mem::swap(&mut self.rejected_prev, &mut self.rejected_cur);
            self.rejected_cur.clear();
            self.cursor = 0;
            for h in &hits {
                self.apply(m, y, *h);
            }
        }
        self.bounds = bounds;
        self.hits = hits;
        self.candidates
            .sort_by(|a, b| b.hits.cmp(&a.hits).then(a.module.total_cmp(&b.module)));
    }

    /// Whether the previous row had the same run sequence at the same place.
    fn seen_above(&mut self, h: &RowHit) -> bool {
        let tol = 0.5 + h.total as f32 / 14.0;
        while self
            .rejected_prev
            .get(self.cursor)
            .is_some_and(|r| r.hx < h.hx - tol)
        {
            self.cursor += 1;
        }
        self.rejected_prev[self.cursor..]
            .iter()
            .take_while(|r| r.hx <= h.hx + tol)
            .any(|r| r.inv == h.inv && r.total.abs_diff(h.total) <= 1 + h.total / 10)
    }

    fn apply(&mut self, m: &BitMatrix, y: usize, h: RowHit) {
        if self.confirms >= MAX_CONFIRMS {
            return;
        }
        let yf = y as f32 + 0.5;
        let near = |p: Point, mo: f32| (h.hx - p.x).abs() < mo * 1.5 && (yf - p.y).abs() < mo * 1.5;
        if let Some(c) = self
            .candidates
            .iter_mut()
            .find(|c| c.inv == h.inv && near(c.pos, c.module))
        {
            c.hits += 1;
            return;
        }
        // The same run sequence in the row above: a stripe, not a bullseye.
        if self.seen_above(&h) {
            self.rejected_cur.push(h);
            return;
        }
        self.confirms += 1;
        match confirm(Plane::new(m, h.inv), h.hx, y, h.total) {
            Some((pos, module)) if self.candidates.len() < MAX_CANDIDATES => {
                self.candidates.push(Candidate {
                    pos,
                    module,
                    inv: h.inv,
                    hits: 1,
                });
            }
            Some(_) => {}
            None => self.rejected_cur.push(h),
        }
    }
}

/// Checks the column and both diagonals through a row hit; returns the
/// refined center and the module estimate.
fn confirm(m: Plane<'_>, hx: f32, y: usize, h_total: u32) -> Option<(Point, f32)> {
    let limit = h_total + h_total / 2 + 2;
    let (mut cx, mut cy) = (hx, y as f32 + 0.5);
    let mut totals = [h_total as f32, 0.0];
    for _ in 0..2 {
        let (v, shift2) = line_runs(m, cx.floor() as i32, cy.floor() as i32, (0, 1), limit)?;
        if !bullseye_ratio(&v, LINE_TOL_PERCENT) {
            return None;
        }
        cy = cy.floor() + 0.5 + shift2 as f32 / 2.0;
        totals[1] = v.iter().sum::<u32>() as f32;
        let (h, shift2) = line_runs(m, cx.floor() as i32, cy.floor() as i32, (1, 0), limit)?;
        if !bullseye_ratio(&h, LINE_TOL_PERCENT) {
            return None;
        }
        cx = cx.floor() + 0.5 + shift2 as f32 / 2.0;
        totals[0] = h.iter().sum::<u32>() as f32;
    }
    let ratio = totals[1] / totals[0];
    if !(0.55..=1.8).contains(&ratio) {
        return None;
    }
    let mean = (totals[0] + totals[1]) / 2.0;
    for d in [(1, 1), (1, -1)] {
        let (r, _) = line_runs(m, cx.floor() as i32, cy.floor() as i32, d, limit)?;
        if !bullseye_ratio(&r, DIAG_TOL_PERCENT) {
            return None;
        }
        let t = r.iter().sum::<u32>() as f32 / mean;
        if !(0.4..=1.3).contains(&t) {
            return None;
        }
    }
    Some((Point::new(cx, cy), mean / RUNS as f32))
}

#[cfg(test)]
mod tests;
