//! Finder pattern search and candidate (finder triplet) selection.

use std::f32::consts::SQRT_2;

use breader_core::{BitMatrix, Plane, Point};

const FINDER_UNITS: [f32; 5] = [1.0, 1.0, 3.0, 1.0, 1.0];
/// Hard cap on stored finder hits per pass.
const ROW_TOL: f32 = 0.65;
const ROW_TOL_PERCENT: u32 = 65;
const MAX_FINDERS: usize = 1024;

/// A confirmed finder pattern, in pass-image coordinates.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Finder {
    pub pos: Point,
    /// Module size estimate in pixels.
    pub module: f32,
    /// Chord lengths through the center (horizontal, vertical, diagonal) in
    /// pixels; the module size follows once the symbol orientation is known.
    chords: [f32; 3],
    pub count: u32,
    /// Light-on-dark symbol: the pattern was found in the inverted plane.
    pub inv: bool,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Triplet {
    pub tl: Finder,
    pub tr: Finder,
    pub bl: Finder,
    /// Estimated (fractional) dimension in modules.
    pub dim: f32,
    pub score: f32,
    /// Dimension agreed on by both timing patterns, 0 when unknown.
    pub dim_hint: usize,
    /// The third finder was inferred from a timing pattern, not detected.
    pub synthetic: bool,
    /// Use the parallelogram for the fourth corner (no perspective correction).
    pub parallel: bool,
    /// Passed only the perspective-tolerant geometry limits; tried after
    /// every strict triplet.
    pub loose: bool,
}

#[derive(Debug, Default)]
pub(crate) struct FinderSearch {
    edges: Vec<u32>,
    row_hits: Vec<RowHit>,
    pub finders: Vec<Finder>,
    pub triplets: Vec<Triplet>,
    order: Vec<usize>,
    recent: Vec<usize>,
    max_module: f32,
    links: Vec<Link>,
    incident: Vec<(usize, f32, u16)>,
}

/// Run lengths must follow `units` within `tol` units (center run twice as lenient).
fn fits(runs: &[f32; 5], units: &[f32; 5], tol: f32) -> bool {
    let total: f32 = runs.iter().sum();
    let usum: f32 = units.iter().sum();
    if total.is_nan() || total <= 0.0 {
        return false;
    }
    let u = total / usum;
    runs.iter().zip(units).all(|(r, w)| {
        let t = if *w > 1.5 { tol * 2.0 } else { tol };
        (r - w * u).abs() <= t * u
    })
}

/// Integer form of [`fits`] for the finder ratio: each run within `tol_pct`
/// percent of a unit of its expected width (the center run twice as lenient).
fn finder_ratio(runs: &[u32; 5], tol_pct: u32) -> bool {
    let total: u32 = runs.iter().sum();
    if total == 0 {
        return false;
    }
    const UNITS: [u32; 5] = [1, 1, 3, 1, 1];
    runs.iter().zip(UNITS).all(|(&r, w)| {
        let t = if w > 1 { tol_pct * 2 } else { tol_pct };
        (7 * r).abs_diff(w * total) * 100 <= t * total
    })
}

fn in_bounds(m: Plane<'_>, x: i32, y: i32) -> bool {
    x >= 0 && y >= 0 && (x as usize) < m.width() && (y as usize) < m.height()
}

/// Counts consecutive pixels of `dark` color starting at (x, y). Returns the
/// count and whether the walk left the image or hit `limit`.
fn count_run(
    m: Plane<'_>,
    x: i32,
    y: i32,
    d: (i32, i32),
    dark: bool,
    limit: u32,
) -> (u32, i32, i32, bool) {
    let (n, stopped) = m.run_len(x, y, d, dark, limit);
    (n, x + d.0 * n as i32, y + d.1 * n as i32, stopped)
}

struct Line {
    runs: [f32; 5],
    /// Center offset from the start pixel times two (steps).
    shift2: i32,
}

/// Measures the five runs around a dark start pixel along direction `d`.
fn line_check(m: Plane<'_>, x: i32, y: i32, d: (i32, i32), limit: u32) -> Option<Line> {
    if !in_bounds(m, x, y) || !m.get(x as usize, y as usize) {
        return None;
    }
    let nd = (-d.0, -d.1);
    let (cp, x1, y1, e) = count_run(m, x, y, d, true, limit);
    if e {
        return None;
    }
    let (lp, x2, y2, e) = count_run(m, x1, y1, d, false, limit);
    if e {
        return None;
    }
    let (op, _, _, e) = count_run(m, x2, y2, d, true, limit);
    if e && op >= limit {
        return None;
    }
    let (cm, x1, y1, e) = count_run(m, x - d.0, y - d.1, nd, true, limit);
    if e {
        return None;
    }
    let (lm, x2, y2, e) = count_run(m, x1, y1, nd, false, limit);
    if e {
        return None;
    }
    let (om, _, _, e) = count_run(m, x2, y2, nd, true, limit);
    if e && om >= limit {
        return None;
    }
    Some(Line {
        runs: [om, lm, cp + cm, lp, op].map(|v| v as f32),
        shift2: cp as i32 - cm as i32,
    })
}

fn confirm_finder(m: Plane<'_>, hx: f32, y: usize, h_total: f32) -> Option<(Point, [f32; 3])> {
    let limit = (h_total * 1.5) as u32 + 2;
    let (mut cx, mut cy) = (hx, y as f32 + 0.5);
    let mut chords = [h_total, 0.0, 0.0];
    // Alternate vertical and horizontal checks so both chords pass through
    // the refined center.
    for round in 0..2 {
        let v = line_check(m, cx.floor() as i32, cy.floor() as i32, (0, 1), limit)?;
        if !fits(&v.runs, &FINDER_UNITS, ROW_TOL + 0.1) {
            return None;
        }
        cy = cy.floor() + v.shift2 as f32 / 2.0;
        chords[1] = v.runs.iter().sum();
        let h = line_check(m, cx.floor() as i32, cy.floor() as i32, (1, 0), limit)?;
        if !fits(&h.runs, &FINDER_UNITS, ROW_TOL + 0.1) {
            return None;
        }
        cx = cx.floor() + h.shift2 as f32 / 2.0;
        chords[0] = h.runs.iter().sum();
        let ratio = chords[1] / chords[0];
        if round == 1 && !(0.6..=1.7).contains(&ratio) {
            return None;
        }
    }
    let d = line_check(m, cx.floor() as i32, cy.floor() as i32, (1, 1), limit)?;
    if !fits(&d.runs, &FINDER_UNITS, ROW_TOL + 0.3) {
        return None;
    }
    chords[2] = d.runs.iter().sum::<f32>() * SQRT_2;
    Some((Point::new(cx, cy), chords))
}

/// Rows per parallel work item of the finder search.
#[cfg(feature = "rayon")]
const PAR_BAND_ROWS: usize = 32;
/// Matrices below this many pixels are searched on the calling thread.
#[cfg(feature = "rayon")]
const PAR_MIN_PIXELS: usize = 1 << 20;

/// A run sequence of row `y` in the 1:1:3:1:1 ratio.
#[derive(Debug, Clone, Copy)]
struct RowHit {
    y: usize,
    inv: bool,
    hx: f32,
    h_total: f32,
}

/// Appends the finder-like run sequences of row `y` to `out`, dark-on-light
/// ones before light-on-dark ones.
fn row_hits(m: &BitMatrix, y: usize, bounds: &mut Vec<u32>, out: &mut Vec<RowHit>) {
    let width = m.width();
    // Run boundaries: 0, every color change, width.
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
    // Run k spans bounds[k]..bounds[k + 1]; run 0 is dark when pixel 0 is.
    // A dark-on-light finder starts on a dark run, a light-on-dark one on
    // a light run; both are tested against the same run boundaries.
    let first_dark = usize::from(!m.get(0, y));
    for inv in [false, true] {
        let first = first_dark ^ usize::from(inv);
        for b in bounds.windows(6).skip(first).step_by(2) {
            let runs = [
                b[1] - b[0],
                b[2] - b[1],
                b[3] - b[2],
                b[4] - b[3],
                b[5] - b[4],
            ];
            if finder_ratio(&runs, ROW_TOL_PERCENT) {
                out.push(RowHit {
                    y,
                    inv,
                    hx: (b[2] + b[3]) as f32 / 2.0,
                    h_total: (b[5] - b[0]) as f32,
                });
            }
        }
    }
}

impl FinderSearch {
    /// Scans every `row_step`-th row of the binary image for finder patterns.
    pub fn find_finders(&mut self, m: &BitMatrix, row_step: usize) {
        self.finders.clear();
        self.recent.clear();
        self.max_module = 0.0;
        let step = row_step.max(1);
        let first = step / 2;
        #[cfg(feature = "rayon")]
        if m.width() * m.height() >= PAR_MIN_PIXELS
            && rayon::current_num_threads() > 1
            && !breader_core::parallel::is_sequential()
        {
            self.find_finders_banded(m, first, step);
            return;
        }
        let mut bounds = std::mem::take(&mut self.edges);
        let mut hits = std::mem::take(&mut self.row_hits);
        for y in (first..m.height()).step_by(step) {
            self.age_recent(y);
            hits.clear();
            row_hits(m, y, &mut bounds, &mut hits);
            self.apply_hits(m, &hits);
        }
        self.edges = bounds;
        self.row_hits = hits;
    }

    /// Run extraction is independent per row; only the clustering of hits
    /// into finders depends on earlier rows, so rows are searched in
    /// parallel bands and the hits fed to the clustering in row order.
    #[cfg(feature = "rayon")]
    fn find_finders_banded(&mut self, m: &BitMatrix, first: usize, step: usize) {
        use rayon::prelude::*;
        let rows: Vec<usize> = (first..m.height()).step_by(step).collect();
        let per_band: Vec<Vec<RowHit>> = rows
            .par_chunks(PAR_BAND_ROWS)
            .map_init(Vec::new, |bounds, band| {
                let mut hits = Vec::new();
                for &y in band {
                    row_hits(m, y, bounds, &mut hits);
                }
                hits
            })
            .collect();
        let mut hits = per_band.iter().flatten().peekable();
        let mut row = Vec::new();
        for &y in &rows {
            self.age_recent(y);
            row.clear();
            while let Some(h) = hits.next_if(|h| h.y == y) {
                row.push(*h);
            }
            self.apply_hits(m, &row);
        }
    }

    /// Finders far above the current row can no longer be matched.
    fn age_recent(&mut self, y: usize) {
        let (fs, reach) = (&self.finders, 3.5 * self.max_module + 2.0);
        self.recent.retain(|&i| fs[i].pos.y + reach >= y as f32);
    }

    fn apply_hits(&mut self, m: &BitMatrix, hits: &[RowHit]) {
        for h in hits {
            self.add_hit(Plane::new(m, h.inv), h.hx, h.y, h.h_total);
        }
    }

    fn add_hit(&mut self, plane: Plane<'_>, hx: f32, y: usize, h_total: f32) {
        let inv = plane.inverted();
        let yf = y as f32 + 0.5;
        for &i in &self.recent {
            let f = &mut self.finders[i];
            if f.inv == inv
                && (hx - f.pos.x).abs() < f.module * 2.5
                && (yf - f.pos.y).abs() < f.module * 2.5
            {
                f.count += 1;
                return;
            }
        }
        if self.finders.len() >= MAX_FINDERS
            && self.finders.iter().filter(|f| f.inv == inv).count() >= MAX_FINDERS
        {
            return;
        }
        let Some((pos, chords)) = confirm_finder(plane, hx, y, h_total) else {
            return;
        };
        let reach = chords.iter().copied().fold(f32::MAX, f32::min) / 7.0 * 3.5;
        for &i in &self.recent {
            let f = &mut self.finders[i];
            if f.inv == inv && f.pos.distance(pos) < reach.max(f.module * 3.5) {
                let n = f.count as f32;
                f.pos = Point::new(
                    (f.pos.x * n + pos.x) / (n + 1.0),
                    (f.pos.y * n + pos.y) / (n + 1.0),
                );
                for (c, new) in f.chords.iter_mut().zip(chords) {
                    *c = (*c * n + new) / (n + 1.0);
                }
                f.module = f.chords.iter().copied().fold(f32::MAX, f32::min) / 7.0;
                f.count += 1;
                self.max_module = self.max_module.max(f.module);
                return;
            }
        }
        let module = chords.iter().copied().fold(f32::MAX, f32::min) / 7.0;
        self.max_module = self.max_module.max(module);
        self.recent.push(self.finders.len());
        self.finders.push(Finder {
            pos,
            module,
            chords,
            count: 1,
            inv,
        });
    }

    /// Builds candidate triplets from the strongest `max_finders` finders,
    /// best score first. Finder pairs are kept only when the timing pattern
    /// between them alternates, so triplets are not formed across symbols.
    pub fn select_triplets(&mut self, m: Plane<'_>, max_finders: usize, row_step: usize) {
        let inv = m.inverted();
        self.triplets.clear();
        let strength = |f: &Finder| f.count as f32 * row_step as f32 / (7.0 * f.module.max(0.5));
        self.order.clear();
        self.order
            .extend((0..self.finders.len()).filter(|&i| self.finders[i].inv == inv));
        let fs = &self.finders;
        self.order
            .sort_by(|&a, &b| strength(&fs[b]).total_cmp(&strength(&fs[a])));
        self.order.truncate(max_finders);
        let n = self.order.len();
        self.links.clear();
        // Few finders: strong perspective is worth the extra false pairs.
        let relaxed = n <= RELAXED_MAX_FINDERS;
        for i in 0..n {
            for j in i + 1..n {
                let (a, b) = (&self.finders[self.order[i]], &self.finders[self.order[j]]);
                if let Some(l) = link(m, a, b, i, j, relaxed)
                    && (l.score >= 0.75 || n <= UNVERIFIED_LINK_MAX_FINDERS)
                {
                    self.links.push(l);
                }
            }
        }
        // Per finder: its strongest links, as the corner of a triplet.
        self.incident.clear();
        for corner in 0..n {
            self.incident.clear();
            self.incident.extend(
                self.links
                    .iter()
                    .filter(|l| l.a == corner || l.b == corner)
                    .map(|l| (if l.a == corner { l.b } else { l.a }, l.score, l.dim)),
            );
            self.incident.sort_by(|x, y| y.1.total_cmp(&x.1));
            self.incident.truncate(MAX_LINKS_PER_FINDER);
            for p in 0..self.incident.len() {
                for q in p + 1..self.incident.len() {
                    let (ip, sp, dp) = self.incident[p];
                    let (iq, sq, dq) = self.incident[q];
                    let (c, a, b) = (
                        &self.finders[self.order[corner]],
                        &self.finders[self.order[ip]],
                        &self.finders[self.order[iq]],
                    );
                    let st = strength(a) + strength(b) + strength(c);
                    if let Some(t) = make_triplet(c, a, b, st, (sp, sq), (dp, dq), relaxed) {
                        self.triplets.push(t);
                    }
                }
            }
        }
        self.add_recovered_triplets(n);
        self.triplets
            .sort_by(|a, b| a.loose.cmp(&b.loose).then(a.score.total_cmp(&b.score)));
    }

    /// A verified timing pattern between two finders locates the third one
    /// even when it is too damaged to be detected; such triplets rank after
    /// real ones.
    fn add_recovered_triplets(&mut self, n: usize) {
        for li in 0..self.links.len() {
            let l = self.links[li];
            if l.score < 0.75 || l.dim == 0 || l.side == 0 || l.a >= n || l.b >= n {
                continue;
            }
            let (fa, fb) = (self.finders[self.order[l.a]], self.finders[self.order[l.b]]);
            let (dx, dy) = (fb.pos.x - fa.pos.x, fb.pos.y - fa.pos.y);
            let side = f32::from(l.side);
            // Perpendicular of the a->b direction on the symbol side.
            let (px, py) = (-dy * side, dx * side);
            for (corner, other) in [(fa, fb), (fb, fa)] {
                let c = Point::new(corner.pos.x + px, corner.pos.y + py);
                let near = self.finders.iter().any(|f| {
                    f.inv == corner.inv && f.pos.distance(c) < 2.0 * corner.module.max(1.0)
                });
                if near {
                    continue;
                }
                let synth = Finder {
                    pos: c,
                    count: 1,
                    ..corner
                };
                if let Some(mut t) = make_triplet(
                    &corner,
                    &other,
                    &synth,
                    0.0,
                    (l.score, l.score),
                    (l.dim, l.dim),
                    n <= RELAXED_MAX_FINDERS,
                ) {
                    t.score += 1.5;
                    t.synthetic = true;
                    self.triplets.push(t);
                }
            }
        }
    }
}

/// Finder pair whose connecting timing pattern was verified.
#[derive(Debug, Clone, Copy)]
struct Link {
    a: usize,
    b: usize,
    /// Fraction of matching timing modules (0.5 when too small to check).
    score: f32,
    /// Dimension that matched best, 0 when unknown.
    dim: u16,
    /// Which side of the line the symbol body lies on (+1 / -1), 0 unknown.
    side: i8,
}

const RELAXED_MAX_FINDERS: usize = 60;
/// Limits of the perspective tier: leg ratio, |cos| of the corner angle,
/// module ratio between finders, and the relative disagreement of the
/// per-side dimension estimates.
const LOOSE_LEG: f32 = 3.0;
const LOOSE_COS: f32 = 0.8;
const LOOSE_MODULE: f32 = 3.2;
const LOOSE_SIDE_DIFF: f32 = 0.25;
const MAX_LINKS_PER_FINDER: usize = 8;
/// With few finders, pairs without a verified timing pattern stay candidates
/// (strong perspective or blur can break the check).
const UNVERIFIED_LINK_MAX_FINDERS: usize = 40;

fn pixel_dark(m: Plane<'_>, p: Point) -> bool {
    p.x >= 0.0
        && p.y >= 0.0
        && (p.x as usize) < m.width()
        && (p.y as usize) < m.height()
        && m.get(p.x as usize, p.y as usize)
}

/// Cheap pre-check of the first modules of a timing line next to a finder
/// (finder edge, separator, first timing modules) at fixed offsets.
fn timing_end_ok(m: Plane<'_>, from: Point, u: Point, perp: Point, module: f32) -> bool {
    const EXPECT: [(f32, bool); 6] = [
        (0.0, true),
        (3.0, true),
        (4.0, false),
        (5.0, true),
        (6.0, false),
        (7.0, true),
    ];
    let mut misses = 0;
    for (off, dark) in EXPECT {
        let mut votes = 0;
        for jitter in [2.7f32, 3.0, 3.3] {
            let p = Point::new(
                from.x + (u.x * off + perp.x * jitter) * module,
                from.y + (u.y * off + perp.y * jitter) * module,
            );
            votes += usize::from(pixel_dark(m, p));
        }
        if (votes >= 2) != dark {
            misses += 1;
            if misses > 2 {
                return false;
            }
        }
    }
    true
}

/// Verifies the timing pattern between two finders by walking the line along
/// it (three modules inside the symbol edge) and checking the run structure:
/// finder edge, separator, alternating unit modules, separator, finder edge.
/// The unit run count gives the dimension independently of the module size
/// estimate, and tolerates the foreshortening of perspective.
fn link(m: Plane<'_>, a: &Finder, b: &Finder, ia: usize, ib: usize, relaxed: bool) -> Option<Link> {
    let dist = a.pos.distance(b.pos);
    if dist <= 0.0 {
        return None;
    }
    // The line between two finders of one symbol is a symbol axis.
    let (ux, uy) = ((b.pos.x - a.pos.x) / dist, (b.pos.y - a.pos.y) / dist);
    let (ma, mb) = (module_along(a, ux, uy), module_along(b, ux, uy));
    if ma <= 0.0 || mb <= 0.0 || ma.max(mb) / ma.min(mb) > if relaxed { 3.2 } else { 1.9 } {
        return None;
    }
    let module = (ma + mb) / 2.0;
    let est = dist / module + 7.0;
    if !(19.0..=185.0).contains(&est) {
        return None;
    }
    let unchecked = Link {
        a: ia,
        b: ib,
        score: 0.5,
        dim: 0,
        side: 0,
    };
    if module < 1.3 {
        return None;
    }
    const MAX_STEPS: usize = 1024;
    let u = Point::new(ux, uy);
    let steps = ((dist / (0.25 * module).max(0.6)).ceil() as usize).clamp(8, MAX_STEPS - 1);
    let mut best = Link {
        score: 0.0,
        ..unchecked
    };
    let step_px = dist / steps as f32;
    let mut colors = [false; MAX_STEPS];
    let mut widths = [0.0f32; 256];
    for side in [-1.0f32, 1.0] {
        let perp = Point::new(-u.y * side, u.x * side);
        if !timing_end_ok(m, a.pos, u, perp, ma)
            || !timing_end_ok(m, b.pos, Point::new(-u.x, -u.y), perp, mb)
        {
            continue;
        }
        let p0 = Point::new(a.pos.x + perp.x * 3.0 * ma, a.pos.y + perp.y * 3.0 * ma);
        let p1 = Point::new(b.pos.x + perp.x * 3.0 * mb, b.pos.y + perp.y * 3.0 * mb);
        let inside = |p: Point| {
            p.x >= 0.0 && p.y >= 0.0 && p.x < m.width() as f32 && p.y < m.height() as f32
        };
        let (dx, dy) = ((p1.x - p0.x) / steps as f32, (p1.y - p0.y) / steps as f32);
        if inside(p0) && inside(p1) {
            // Both ends inside: so is every sample between them (the clamp
            // only absorbs rounding at the far end).
            let (xmax, ymax) = (m.width() - 1, m.height() - 1);
            for (k, c) in colors[..=steps].iter_mut().enumerate() {
                let kf = k as f32;
                *c = m.get(
                    ((p0.x + dx * kf) as usize).min(xmax),
                    ((p0.y + dy * kf) as usize).min(ymax),
                );
            }
        } else {
            for (k, c) in colors[..=steps].iter_mut().enumerate() {
                let kf = k as f32;
                *c = pixel_dark(m, Point::new(p0.x + dx * kf, p0.y + dy * kf));
            }
        }
        // Majority filter against single-sample glitches.
        let raw = colors;
        for k in 1..steps {
            colors[k] =
                usize::from(raw[k - 1]) + usize::from(raw[k]) + usize::from(raw[k + 1]) >= 2;
        }
        // Run lengths between the first and last dark runs.
        let mut n_int = 0usize;
        let mut len = 1usize;
        let mut seen_first = false;
        let mut overflow = false;
        for k in 1..=steps {
            if colors[k] == colors[k - 1] && k < steps {
                len += 1;
                continue;
            }
            if colors[k] == colors[k - 1] {
                len += 1;
            }
            // A run ended at k (or the line ended).
            let run_dark = colors[k - 1];
            if !seen_first {
                if !run_dark {
                    // Must start inside the finder edge.
                    overflow = true;
                    break;
                }
                seen_first = true;
            } else if n_int < widths.len() {
                // Width relative to the module size expected at this spot
                // (interpolated between the two ends: perspective).
                let mid = (k as f32 - len as f32 / 2.0) / steps as f32;
                let expected = (ma + (mb - ma) * mid) / step_px;
                widths[n_int] = len as f32 / expected.max(0.1);
                n_int += 1;
            } else {
                overflow = true;
                break;
            }
            len = 1;
        }
        // The final run is the last finder edge (dark): drop it again.
        if overflow || n_int < 6 || !colors[steps] {
            continue;
        }
        let interior = &mut widths[..n_int - 1];
        let n_unit = interior.len();
        interior.sort_unstable_by(f32::total_cmp);
        let median = interior[n_unit / 2];
        let ok = interior
            .iter()
            .filter(|w| **w >= 0.5 * median && **w <= 1.9 * median)
            .count();
        let d_run = n_unit + 14;
        let valid = d_run % 4 == 1 && (21..=177).contains(&d_run);
        let slack = if relaxed {
            0.5 * est + 6.0
        } else {
            0.25 * est + 4.0
        };
        let consistent = (d_run as f32 - est).abs() <= slack;
        let mut score = ok as f32 / n_unit as f32;
        if !consistent {
            score *= 0.5;
        }
        if !valid {
            score *= 0.85;
        }
        if score > best.score {
            best = Link {
                score,
                dim: if valid && consistent { d_run as u16 } else { 0 },
                side: side as i8,
                ..unchecked
            };
        }
    }
    Some(best)
}

fn module_at(f: &Finder, theta: f32) -> f32 {
    module_along(f, theta.cos(), theta.sin())
}

/// Module size from the finder's chords seen along the unit direction (ux, uy).
fn module_along(f: &Finder, ux: f32, uy: f32) -> f32 {
    let fh = ux.abs().max(uy.abs());
    let fd = (ux + uy).abs().max((uy - ux).abs()) * std::f32::consts::FRAC_1_SQRT_2;
    (f.chords[0] * fh + f.chords[1] * fh + f.chords[2] * fd) / 21.0
}

fn make_triplet(
    corner: &Finder,
    p: &Finder,
    q: &Finder,
    strength: f32,
    (sp, sq): (f32, f32),
    (dp, dq): (u16, u16),
    relaxed: bool,
) -> Option<Triplet> {
    let (v1, v2) = (
        (p.pos.x - corner.pos.x, p.pos.y - corner.pos.y),
        (q.pos.x - corner.pos.x, q.pos.y - corner.pos.y),
    );
    let (l1, l2) = (v1.0.hypot(v1.1), v2.0.hypot(v2.1));
    if l1 <= 0.0 || l2 <= 0.0 {
        return None;
    }
    let leg_ratio = l1.max(l2) / l1.min(l2);
    let cos = (v1.0 * v2.0 + v1.1 * v2.1) / (l1 * l2);
    // Both timing patterns agreeing on the dimension tolerates strong perspective.
    let hinted = relaxed && dp == dq && dp != 0;
    let (max_leg, max_cos, max_mod) = if hinted || relaxed {
        (LOOSE_LEG, LOOSE_COS, LOOSE_MODULE)
    } else {
        (1.6, 0.5, 1.6)
    };
    if leg_ratio > max_leg || cos.abs() > max_cos {
        return None;
    }
    let cross = v1.0 * v2.1 - v1.1 * v2.0;
    let ((tr, str_, dtr), (bl, sbl, dbl)) = if cross > 0.0 {
        ((p, sp, dp), (q, sq, dq))
    } else {
        ((q, sq, dq), (p, sp, dp))
    };
    let theta = (tr.pos.y - corner.pos.y).atan2(tr.pos.x - corner.pos.x);
    let mut tl = *corner;
    let (mut tr, mut bl) = (*tr, *bl);
    for f in [&mut tl, &mut tr, &mut bl] {
        f.module = module_at(f, theta);
    }
    let mods = [tl.module, tr.module, bl.module];
    let mx = mods.iter().copied().fold(0.0, f32::max);
    let mn = mods.iter().copied().fold(f32::MAX, f32::min);
    if mn <= 0.0 || mx / mn > max_mod {
        return None;
    }
    let d1 = tl.pos.distance(tr.pos) / ((tl.module + tr.module) / 2.0) + 7.0;
    let d2 = tl.pos.distance(bl.pos) / ((tl.module + bl.module) / 2.0) + 7.0;
    let mut dim = if hinted {
        f32::from(dp)
    } else {
        (d1 + d2) / 2.0
    };
    let mut loose = false;
    if !hinted {
        let strict = leg_ratio <= 1.6
            && cos.abs() <= 0.5
            && mx / mn <= 1.6
            && (19.0..=185.0).contains(&dim)
            && (d1 - d2).abs() <= 0.2 * dim;
        if !strict {
            if !relaxed {
                return None;
            }
            let est = perspective_dim(&tl, &tr, &bl, v1, v2, cross > 0.0)?;
            dim = est;
            loose = true;
        }
    }
    let score =
        cos.abs() * 2.0 + (leg_ratio.ln()).abs() + (mx / mn).ln() + (d1 - d2).abs() / dim * 3.0
            - 0.3 * strength.min(3.0)
            - 2.0 * (str_ + sbl);
    Some(Triplet {
        tl,
        tr,
        bl,
        dim,
        score,
        dim_hint: if dtr == dbl && dtr != 0 {
            usize::from(dtr)
        } else {
            0
        },
        synthetic: false,
        parallel: false,
        loose,
    })
}

/// Module size along the unit direction (ux, uy) of a symbol side. The
/// horizontal chord measures the side's module when the side is horizontal,
/// the vertical chord when it is vertical, so foreshortening along one image
/// axis shows up in one side only.
fn module_side(f: &Finder, ux: f32, uy: f32) -> f32 {
    let w = (f.chords[0] * ux * ux + f.chords[1] * uy * uy) * ux.abs().max(uy.abs());
    w / 7.0
}

/// Dimension estimated per side with the module size along that side, for
/// triplets outside the strict isotropic limits. A homography changes the
/// module size along the two sides differently, so the sides must agree on
/// the dimension within `LOOSE_SIDE_DIFF`, a bound far tighter than the
/// isotropic estimate would need. `None` when they do not.
fn perspective_dim(
    tl: &Finder,
    tr: &Finder,
    bl: &Finder,
    v1: (f32, f32),
    v2: (f32, f32),
    p_is_tr: bool,
) -> Option<f32> {
    let (vt, vb) = if p_is_tr { (v1, v2) } else { (v2, v1) };
    let side = |end: &Finder, v: (f32, f32)| {
        let l = v.0.hypot(v.1);
        let (ux, uy) = (v.0 / l, v.1 / l);
        let m = (module_side(tl, ux, uy) + module_side(end, ux, uy)) / 2.0;
        (l, m)
    };
    let (lt, mt) = side(tr, vt);
    let (lb, mb) = side(bl, vb);
    if mt <= 0.0 || mb <= 0.0 {
        return None;
    }
    let (da, db) = (lt / mt + 7.0, lb / mb + 7.0);
    let dim = (da + db) / 2.0;
    ((19.0..=185.0).contains(&dim) && (da - db).abs() <= LOOSE_SIDE_DIFF * dim).then_some(dim)
}

/// Valid dimensions near the fractional estimate, closest first.
pub(crate) fn candidate_dims(dim: f32) -> ([usize; 3], usize) {
    let k = ((dim - 1.0) / 4.0).round();
    let base = (k * 4.0 + 1.0).clamp(21.0, 177.0) as usize;
    let other = if dim > base as f32 {
        [base + 4, base.wrapping_sub(4)]
    } else {
        [base.wrapping_sub(4), base + 4]
    };
    let mut out = [base, 0, 0];
    let mut n = 1;
    for d in other {
        if (21..=177).contains(&d) {
            out[n] = d;
            n += 1;
        }
    }
    (out, n)
}

/// Mean unit of the inner light-dark-light runs when they look like the
/// center of an alignment pattern; the outer dark runs only need to exist
/// because neighbouring data modules may extend them.
fn alignment_unit(runs: &[f32; 5], module: f32) -> bool {
    let u = (runs[1] + runs[2] + runs[3]) / 3.0;
    u >= module * 0.6
        && u <= module * 1.7
        && runs[1..4].iter().all(|r| (r - u).abs() <= 0.5 * u)
        && runs[0] >= 0.5 * u
        && runs[4] >= 0.5 * u
}

/// Checks the 5x5 module pattern around `c` (dark ring, light ring, dark
/// center) along the module axes `ex`, `ey` (pixels per module).
fn verify_alignment(m: Plane<'_>, c: Point, ex: Point, ey: Point) -> usize {
    let mut bad = 0;
    for j in -2i32..=2 {
        for i in -2i32..=2 {
            let x = c.x + i as f32 * ex.x + j as f32 * ey.x;
            let y = c.y + i as f32 * ex.y + j as f32 * ey.y;
            let want = i.abs().max(j.abs()) != 1;
            let got = x >= 0.0
                && y >= 0.0
                && (x as usize) < m.width()
                && (y as usize) < m.height()
                && m.get(x as usize, y as usize);
            bad += usize::from(got != want);
        }
    }
    bad
}

/// Columns `p` in `(x0, x1)` where pixel `p` differs from pixel `p - 1`.
fn window_edges(m: &BitMatrix, y: usize, x0: usize, x1: usize, out: &mut Vec<u32>) {
    let words = m.row_words(y);
    let first = x0 / 64;
    let last = (x1.saturating_sub(1)) / 64;
    let mut prev = if first == 0 {
        words.first().map_or(0, |w| w & 1)
    } else {
        words[first - 1] >> 63
    };
    for (wi, &wd) in words.iter().enumerate().take(last + 1).skip(first) {
        let mut t = wd ^ ((wd << 1) | prev);
        prev = wd >> 63;
        while t != 0 {
            let pos = wi * 64 + t.trailing_zeros() as usize;
            t &= t - 1;
            if pos > x0 && pos < x1 {
                out.push(pos as u32);
            }
        }
    }
}

/// Looks for an alignment pattern center near `pred` (within `radius`
/// pixels). `ex`/`ey` are the module axes in pixels.
pub(crate) fn find_alignment(
    m: Plane<'_>,
    scratch: &mut Vec<u32>,
    pred: Point,
    (ex, ey): (Point, Point),
    radius: f32,
) -> Option<Point> {
    let module = (ex.x.hypot(ex.y) + ey.x.hypot(ey.y)) / 2.0;
    let margin = module * 3.5;
    let x0 = ((pred.x - radius - margin).floor().max(0.0)) as usize;
    let x1 = ((pred.x + radius + margin).ceil().max(0.0) as usize).min(m.width());
    let y0 = ((pred.y - radius).floor().max(0.0)) as usize;
    let y1 = ((pred.y + radius).ceil().max(0.0) as usize).min(m.height());
    if x1 <= x0 + 5 || y1 <= y0 {
        return None;
    }
    // (sum x, sum y, hits)
    let mut cands = [(0.0f32, 0.0f32, 0u32); 32];
    let mut nc = 0;
    // The center cell is a full module tall: a few rows per module suffice.
    let row_step = ((module / 3.0) as usize).max(1);
    for y in (y0..y1).step_by(row_step) {
        scratch.clear();
        window_edges(m.raw(), y, x0, x1, scratch);
        let first_dark = m.get(x0, y);
        let n_runs = scratch.len() + 1;
        let bound = |k: usize| -> u32 {
            if k == 0 {
                x0 as u32
            } else if k <= scratch.len() {
                scratch[k - 1]
            } else {
                x1 as u32
            }
        };
        let mut i = usize::from(!first_dark);
        while i + 4 < n_runs {
            let b = [
                bound(i),
                bound(i + 1),
                bound(i + 2),
                bound(i + 3),
                bound(i + 4),
                bound(i + 5),
            ];
            i += 2;
            let runs = [
                (b[1] - b[0]) as f32,
                (b[2] - b[1]) as f32,
                (b[3] - b[2]) as f32,
                (b[4] - b[3]) as f32,
                (b[5] - b[4]) as f32,
            ];
            if !alignment_unit(&runs, module) {
                continue;
            }
            let pos = Point::new((b[2] + b[3]) as f32 / 2.0, y as f32 + 0.5);
            if pos.distance(pred) > radius + module {
                continue;
            }
            let slot = cands[..nc].iter().position(|c| {
                let n = c.2 as f32;
                Point::new(c.0 / n, c.1 / n).distance(pos) < module * 1.5
            });
            match slot {
                Some(s) => {
                    cands[s].0 += pos.x;
                    cands[s].1 += pos.y;
                    cands[s].2 += 1;
                }
                None if nc < cands.len() => {
                    cands[nc] = (pos.x, pos.y, 1);
                    nc += 1;
                }
                None => {}
            }
        }
    }
    let mut best: Option<(f32, Point)> = None;
    for c in &cands[..nc] {
        let n = c.2 as f32;
        let p = Point::new(c.0 / n, c.1 / n);
        let bad = verify_alignment(m, p, ex, ey);
        if bad > 3 || p.distance(pred) > radius {
            continue;
        }
        let score = bad as f32 * module * 2.0 + p.distance(pred);
        if best.is_none_or(|b| score < b.0) {
            best = Some((score, p));
        }
    }
    best.map(|b| b.1)
}
