//! Candidate search: connected components on a pooled binary plane and
//! quadrilateral fitting.

use breader_core::{BitMatrix, Plane, Point};

/// Smallest accepted component: cells and bounding box side.
const MIN_CELLS: u32 = 16;
const MIN_BOX: u32 = 3;
/// Components that are compact, filled blobs (the usual look of a symbol)
/// are fitted before the others when the per-level budget is short.
const GOOD_FLAG: u32 = 1 << 30;
const SHAPE_COMPACT: f32 = 0.7;
const SHAPE_RATIO: f32 = 6.0;
const SHAPE_FILL: f32 = 0.5;
/// Share of a block that must be dark for a pooled cell, per level. The
/// finest level drops speckle (a texture would otherwise bridge a symbol to
/// its surroundings). Coarser levels keep every cell with a dark pixel: a
/// share there would erase the thin clock tracks on the symbol boundary and
/// move the quad by up to one cell.
const POOL_SHARE: [f32; 5] = [0.3, 0.0, 0.0, 0.0, 0.0];

/// Euclidean norm without the libm call of `f32::hypot`.
#[inline]
pub(crate) fn hyp(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

/// Smallest accepted quad side in pixels.
const MIN_SIDE: f32 = 12.0;
/// Largest accepted ratio between the long and the short side.
const MAX_ASPECT: f32 = 20.0;

/// A quadrilateral candidate in pass-image pixels, corners clockwise on
/// screen starting anywhere.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Candidate {
    pub quad: [Point; 4],
    pub score: f32,
}

#[derive(Debug, Clone, Copy)]
struct Run {
    start: u32,
    end: u32,
    row: u32,
}

#[derive(Debug, Clone, Copy)]
struct Stats {
    min_x: u32,
    max_x: u32,
    min_y: u32,
    max_y: u32,
    cells: u32,
    sx: f64,
    sy: f64,
    sxx: f64,
    syy: f64,
    sxy: f64,
}

impl Stats {
    /// (compactness, eigen ratio): both 1 for a filled square, compactness 1
    /// for any filled rectangle.
    fn shape(&self) -> (f32, f32) {
        let n = f64::from(self.cells);
        let (mx, my) = (self.sx / n, self.sy / n);
        let vxx = self.sxx / n - mx * mx + 1.0 / 12.0;
        let vyy = self.syy / n - my * my + 1.0 / 12.0;
        let vxy = self.sxy / n - mx * my;
        let tr = vxx + vyy;
        let det = vxx * vyy - vxy * vxy;
        let disc = (tr * tr / 4.0 - det).max(0.0).sqrt();
        let (l1, l2) = (tr / 2.0 + disc, (tr / 2.0 - disc).max(1e-9));
        (
            (n / (12.0 * det.max(1e-9).sqrt())) as f32,
            (l1 / l2).sqrt() as f32,
        )
    }
}

/// Reusable buffers of the candidate search.
#[derive(Debug, Default)]
pub(crate) struct Detector {
    counts: Counts,
    counts_next: Counts,
    pooled: BitMatrix,
    core: BitMatrix,
    grown: BitMatrix,
    hrows: BitMatrix,
    thin: BitMatrix,
    solid: BitMatrix,
    rowtmp: Vec<u64>,
    runs: Vec<Run>,
    parent: Vec<u32>,
    row_start: Vec<u32>,
    stats: Vec<Stats>,
    shortlist: Vec<(u32, u32)>,
    slot: Vec<u32>,
    ext: Vec<(i32, i32)>,
    ext_off: Vec<(usize, i32)>,
    pts: Vec<(i32, i32)>,
    hull: Vec<(i32, i32)>,
    chain: Vec<(f32, f32)>,
    sides: Vec<Side>,
    quad_scratch: QuadScratch,
    pub candidates: Vec<Candidate>,
}

/// Dark-pixel counts over square blocks: cell `(x, y)` counts the symbol-dark
/// pixels of the `f x f` block of the (optionally inverted) matrix at
/// `(x * f, y * f)`, with `f` the pooling factor of the current level.
#[derive(Debug, Default)]
pub(crate) struct Counts {
    pub width: usize,
    pub height: usize,
    stride: usize,
    data: Vec<u16>,
}

impl Counts {
    fn row(&self, y: usize) -> &[u16] {
        &self.data[y * self.stride..y * self.stride + self.width]
    }

    /// Counts of the 4x4 blocks of `src`.
    pub fn from_blocks4(src: &BitMatrix, inverted: bool, out: &mut Counts) {
        let (w, h) = (src.width(), src.height());
        let wpr = w.div_ceil(64);
        out.width = w.div_ceil(4);
        out.height = h.div_ceil(4);
        out.stride = wpr * 16;
        out.data.clear();
        out.data.resize(out.stride * out.height, 0);
        let tail = w % 64;
        let tail_mask = if tail == 0 {
            u64::MAX
        } else {
            (1u64 << tail) - 1
        };
        let flip = if inverted { u64::MAX } else { 0 };
        for py in 0..out.height {
            let mut rows_v: [Option<&[u64]>; 4] = [None; 4];
            for (k, slot) in rows_v.iter_mut().enumerate() {
                if py * 4 + k < h {
                    *slot = Some(src.row_words(py * 4 + k));
                }
            }
            let dst = &mut out.data[py * out.stride..(py + 1) * out.stride];
            for i in 0..wpr {
                let mask = if i + 1 == wpr { tail_mask } else { u64::MAX };
                // Per-nibble pixel counts of each row (0..=4), summed over the
                // four rows in 8-bit lanes (even and odd nibbles apart).
                let (mut even, mut odd) = (0u64, 0u64);
                for r in rows_v.iter().flatten() {
                    let x = (r[i] ^ flip) & mask;
                    let c = x - ((x >> 1) & 0x5555_5555_5555_5555);
                    let c = (c & 0x3333_3333_3333_3333) + ((c >> 2) & 0x3333_3333_3333_3333);
                    even += c & 0x0F0F_0F0F_0F0F_0F0F;
                    odd += (c >> 4) & 0x0F0F_0F0F_0F0F_0F0F;
                }
                let cells = &mut dst[i * 16..i * 16 + 16];
                for j in 0..8 {
                    cells[2 * j] = ((even >> (8 * j)) & 0xFF) as u16;
                    cells[2 * j + 1] = ((odd >> (8 * j)) & 0xFF) as u16;
                }
            }
        }
    }

    /// Counts of the 2x2 groups of `self` (blocks twice as large).
    pub fn halve(&self, out: &mut Counts) {
        out.width = self.width.div_ceil(2);
        out.height = self.height.div_ceil(2);
        out.stride = out.width;
        out.data.clear();
        out.data.resize(out.stride * out.height, 0);
        for y in 0..out.height {
            let top = self.row(y * 2);
            let bottom = (y * 2 + 1 < self.height).then(|| self.row(y * 2 + 1));
            let dst = &mut out.data[y * out.stride..(y + 1) * out.stride];
            for (x, d) in dst.iter_mut().enumerate() {
                let i = 2 * x;
                let mut v = top[i];
                if i + 1 < top.len() {
                    v += top[i + 1];
                }
                if let Some(b) = bottom {
                    v += b[i];
                    if i + 1 < b.len() {
                        v += b[i + 1];
                    }
                }
                *d = v;
            }
        }
    }

    /// Sets the cells whose count reaches `min`.
    pub fn threshold(&self, min: u16, out: &mut BitMatrix) {
        out.resize(self.width, self.height);
        for y in 0..self.height {
            let dst = out.row_words_mut(y);
            for (wi, chunk) in self.row(y).chunks(64).enumerate() {
                let mut bits = 0u64;
                for (gi, group) in chunk.chunks(8).enumerate() {
                    let mut flags = [0u8; 8];
                    for (f, &c) in flags.iter_mut().zip(group) {
                        *f = u8::from(c >= min);
                    }
                    // Collects the low bit of each byte into one byte.
                    let packed =
                        u64::from_le_bytes(flags).wrapping_mul(0x0102_0408_1020_4080) >> 56;
                    bits |= packed << (8 * gi);
                }
                dst[wi] = bits;
            }
        }
    }
}

/// `out[x] = src[x - k]` (shift towards higher columns) for `k` below 64.
fn shift_up(src: &[u64], k: usize, out: &mut [u64]) {
    for i in 0..src.len() {
        let lo = src[i] << k;
        let carry = if i > 0 && k > 0 {
            src[i - 1] >> (64 - k)
        } else {
            0
        };
        out[i] = lo | carry;
    }
}

/// `out[x] = src[x + k]` for `k` below 64.
fn shift_down(src: &[u64], k: usize, out: &mut [u64]) {
    for i in 0..src.len() {
        let hi = src[i] >> k;
        let carry = if i + 1 < src.len() && k > 0 {
            src[i + 1] << (64 - k)
        } else {
            0
        };
        out[i] = hi | carry;
    }
}

/// Erodes (`and`) or dilates (`!and`) the plane by `r` cells in both
/// directions with a square element. Cells beyond the border count as light.
fn morph(
    src: &BitMatrix,
    r: usize,
    and: bool,
    out: &mut BitMatrix,
    tmp: &mut Vec<u64>,
    hrows: &mut BitMatrix,
) {
    let (w, h) = (src.width(), src.height());
    out.resize(w, h);
    hrows.resize(w, h);
    let wpr = src.row_words(0).len().max(1);
    tmp.clear();
    tmp.resize(2 * wpr, 0);
    let (a, b) = tmp.split_at_mut(wpr);
    let tail = w % 64;
    let tail_mask = if tail == 0 {
        u64::MAX
    } else {
        (1u64 << tail) - 1
    };
    for y in 0..h {
        let row = src.row_words(y);
        let dst = hrows.row_words_mut(y);
        dst.copy_from_slice(row);
        for d in 1..=r {
            for dir in 0..2 {
                if dir == 0 {
                    shift_up(row, d, a);
                } else {
                    shift_down(row, d, a);
                }
                b[..wpr].copy_from_slice(&a[..wpr]);
                for i in 0..wpr {
                    dst[i] = if and { dst[i] & b[i] } else { dst[i] | b[i] };
                }
            }
        }
        if let Some(last) = dst.last_mut() {
            *last &= tail_mask;
        }
    }
    for y in 0..h {
        let dst = out.row_words_mut(y);
        let mut first = true;
        for k in 0..=2 * r {
            let yy = (y + k).checked_sub(r).filter(|&v| v < h);
            match yy {
                Some(v) => {
                    let row = hrows.row_words(v);
                    if first {
                        dst.copy_from_slice(row);
                        first = false;
                    } else {
                        for i in 0..wpr {
                            dst[i] = if and {
                                dst[i] & row[i]
                            } else {
                                dst[i] | row[i]
                            };
                        }
                    }
                }
                None if and => {
                    dst.fill(0);
                    first = false;
                    break;
                }
                None => {}
            }
        }
        if first {
            dst.fill(0);
        }
    }
}

fn next_bit(words: &[u64], from: usize, want: bool) -> usize {
    let mut wi = from / 64;
    if wi >= words.len() {
        return words.len() * 64;
    }
    let pick = |w: u64| if want { w } else { !w };
    let mut w = pick(words[wi]) & (!0u64 << (from % 64));
    loop {
        if w != 0 {
            return wi * 64 + w.trailing_zeros() as usize;
        }
        wi += 1;
        if wi >= words.len() {
            return wi * 64;
        }
        w = pick(words[wi]);
    }
}

fn find(parent: &mut [u32], mut i: u32) -> u32 {
    while parent[i as usize] != i {
        let p = parent[i as usize];
        parent[i as usize] = parent[p as usize];
        i = p;
    }
    i
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[hi as usize] = lo;
    }
}

/// Leftmost and rightmost symbol-dark pixels of row `y` in `[x0, x1)`.
fn row_extent(plane: Plane<'_>, y: usize, x0: usize, x1: usize) -> Option<(usize, usize)> {
    let words = plane.raw().row_words(y);
    let flip = if plane.inverted() { u64::MAX } else { 0 };
    let x1 = x1.min(plane.width());
    if x0 >= x1 {
        return None;
    }
    let bit = |wi: usize| -> u64 {
        let mut w = words[wi] ^ flip;
        let lo = wi * 64;
        if lo < x0 {
            w &= !0u64 << (x0 - lo);
        }
        if lo + 64 > x1 {
            let keep = x1 - lo;
            if keep < 64 {
                w &= (1u64 << keep) - 1;
            }
        }
        w
    };
    let (w0, w1) = (x0 / 64, (x1 - 1) / 64);
    let mut first = None;
    for wi in w0..=w1 {
        let w = bit(wi);
        if w != 0 {
            first = Some(wi * 64 + w.trailing_zeros() as usize);
            break;
        }
    }
    let first = first?;
    let mut last = first;
    for wi in (w0..=w1).rev() {
        let w = bit(wi);
        if w != 0 {
            last = wi * 64 + 63 - w.leading_zeros() as usize;
            break;
        }
    }
    Some((first, last))
}

impl Detector {
    /// Finds quad candidates of one polarity at the given pooling levels
    /// (4 times a power of two), best first, at most `max` overall. Levels
    /// below `from` are pooled but not searched (a later stage of a search
    /// that was split).
    pub fn find(
        &mut self,
        raw: &BitMatrix,
        inverted: bool,
        from: usize,
        levels: usize,
        components: usize,
        max: usize,
    ) {
        self.candidates.clear();
        if raw.width() < 8 || raw.height() < 8 {
            return;
        }
        Counts::from_blocks4(raw, inverted, &mut self.counts);
        let plane = Plane::new(raw, inverted);
        let mut factor = 4;
        for level in 0..levels {
            if level > 0 {
                self.counts.halve(&mut self.counts_next);
                std::mem::swap(&mut self.counts, &mut self.counts_next);
                factor *= 2;
            }
            if level < from {
                continue;
            }
            let area = (factor * factor) as u16;
            let min = ((POOL_SHARE[level.min(4)] * f32::from(area)).ceil() as u16).max(1);
            self.counts.threshold(min, &mut self.pooled);
            self.counts.threshold(area, &mut self.solid);
            if self.pooled.width() < 3 || self.pooled.height() < 3 {
                break;
            }
            self.thin_out();
            self.label();
            self.select(components);
            self.fit_all(plane, factor);
        }
        self.finish(max);
    }

    /// Orders by score, drops near duplicates (the same symbol found at
    /// several levels) and keeps `max`.
    fn finish(&mut self, max: usize) {
        self.candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
        let mut kept = 0;
        for i in 0..self.candidates.len() {
            let c = self.candidates[i];
            let dup = self.candidates[..kept]
                .iter()
                .any(|k| similar(&k.quad, &c.quad));
            if !dup {
                self.candidates[kept] = c;
                kept += 1;
            }
        }
        self.candidates.truncate(kept.min(max));
    }

    /// Removes thick dark regions (shadows, filled areas): a Data Matrix is
    /// at most a few modules thick everywhere except in rare module clusters.
    fn thin_out(&mut self) {
        // Solid regions wider than 11 cells (shadows, filled areas) are cut
        // out, with a margin of one cell, so that they cannot merge a symbol
        // with its surroundings.
        const RADIUS: usize = 5;
        let h = self.pooled.height();
        let mut tmp = std::mem::take(&mut self.rowtmp);
        morph(
            &self.solid,
            RADIUS,
            true,
            &mut self.core,
            &mut tmp,
            &mut self.hrows,
        );
        morph(
            &self.core,
            RADIUS + 1,
            false,
            &mut self.grown,
            &mut tmp,
            &mut self.hrows,
        );
        self.thin.resize(self.pooled.width(), h);
        for y in 0..h {
            let g = self.grown.row_words(y);
            let p = self.pooled.row_words(y);
            let t = self.thin.row_words_mut(y);
            for i in 0..p.len() {
                t[i] = p[i] & !g[i];
            }
        }
        self.rowtmp = tmp;
    }

    fn label(&mut self) {
        let (pw, ph) = (self.thin.width(), self.thin.height());
        self.runs.clear();
        self.parent.clear();
        self.row_start.clear();
        let mut prev = 0usize..0usize;
        for y in 0..ph {
            let cur_begin = self.runs.len();
            self.row_start.push(cur_begin as u32);
            let words = self.thin.row_words(y);
            let mut x = 0usize;
            while x < pw {
                let start = next_bit(words, x, true);
                if start >= pw {
                    break;
                }
                let end = next_bit(words, start, false).min(pw);
                let idx = self.runs.len() as u32;
                self.runs.push(Run {
                    start: start as u32,
                    end: end as u32,
                    row: y as u32,
                });
                self.parent.push(idx);
                x = end;
            }
            let cur = cur_begin..self.runs.len();
            let mut j = prev.start;
            for k in cur.clone() {
                let (a, b) = (self.runs[k].start, self.runs[k].end);
                while j < prev.end && self.runs[j].end < a {
                    j += 1;
                }
                let mut m = j;
                while m < prev.end && self.runs[m].start <= b {
                    union(&mut self.parent, k as u32, m as u32);
                    m += 1;
                }
            }
            prev = cur;
        }
        self.row_start.push(self.runs.len() as u32);
        self.stats.clear();
        self.stats.resize(
            self.runs.len(),
            Stats {
                min_x: u32::MAX,
                max_x: 0,
                min_y: u32::MAX,
                max_y: 0,
                cells: 0,
                sx: 0.0,
                sy: 0.0,
                sxx: 0.0,
                syy: 0.0,
                sxy: 0.0,
            },
        );
        for i in 0..self.runs.len() {
            let r = find(&mut self.parent, i as u32) as usize;
            let run = self.runs[i];
            let s = &mut self.stats[r];
            s.min_x = s.min_x.min(run.start);
            s.max_x = s.max_x.max(run.end - 1);
            s.min_y = s.min_y.min(run.row);
            s.max_y = s.max_y.max(run.row);
            s.cells += run.end - run.start;
            // Moments of the cells `[start, end)` of this row.
            let (a, b) = (f64::from(run.start), f64::from(run.end));
            let l = b - a;
            let y = f64::from(run.row);
            let sum_x = (a + b - 1.0) * l / 2.0;
            let sum_x2 =
                (b - 1.0) * b * (2.0 * b - 1.0) / 6.0 - (a - 1.0) * a * (2.0 * a - 1.0) / 6.0;
            s.sx += sum_x;
            s.sy += y * l;
            s.sxx += sum_x2;
            s.syy += y * y * l;
            s.sxy += y * sum_x;
        }
    }

    fn select(&mut self, limit: usize) {
        self.shortlist.clear();
        for (i, s) in self.stats.iter().enumerate() {
            if self.parent[i] != i as u32 || s.cells < MIN_CELLS {
                continue;
            }
            let bw = s.max_x - s.min_x + 1;
            let bh = s.max_y - s.min_y + 1;
            if bw < MIN_BOX || bh < MIN_BOX {
                continue;
            }
            let (compact, ratio) = s.shape();
            let fill = s.cells as f32 / (bw * bh) as f32;
            let good = compact >= SHAPE_COMPACT && ratio <= SHAPE_RATIO && fill >= SHAPE_FILL;
            self.shortlist
                .push((s.cells + if good { GOOD_FLAG } else { 0 }, i as u32));
        }
        // Half of the budget goes to compact, filled blobs, the rest to the
        // largest others (merged symbols, clusters).
        self.shortlist
            .sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let good = self
            .shortlist
            .iter()
            .take_while(|e| e.0 >= GOOD_FLAG)
            .count();
        let rest = self.shortlist.len() - good;
        let take_rest = rest.min(limit - good.min(limit.div_ceil(2)));
        let take_good = good.min(limit - take_rest);
        self.shortlist.truncate(good + take_rest);
        self.shortlist.drain(take_good..good);
        for e in &mut self.shortlist[..take_good] {
            e.0 -= GOOD_FLAG;
        }
    }

    fn fit_all(&mut self, plane: Plane<'_>, factor: usize) {
        let n = self.shortlist.len();
        if n == 0 {
            return;
        }
        self.slot.clear();
        self.slot.resize(self.runs.len(), 0);
        self.ext.clear();
        self.ext_off.clear();
        for (ci, &(_, root)) in self.shortlist.iter().enumerate() {
            self.slot[root as usize] = ci as u32 + 1;
            let s = self.stats[root as usize];
            let y0 = s.min_y as usize * factor;
            let y1 = ((s.max_y as usize + 1) * factor).min(plane.height());
            self.ext_off.push((self.ext.len(), y0 as i32));
            for _ in y0..y1 {
                self.ext.push((i32::MAX, -1));
            }
        }
        for i in 0..self.runs.len() {
            let root = find(&mut self.parent, i as u32) as usize;
            let slot = self.slot[root];
            if slot == 0 {
                continue;
            }
            let (off, y0) = self.ext_off[slot as usize - 1];
            let run = self.runs[i];
            let x0 = run.start as usize * factor;
            let x1 = run.end as usize * factor;
            for y in
                run.row as usize * factor..((run.row as usize + 1) * factor).min(plane.height())
            {
                if let Some((a, b)) = row_extent(plane, y, x0, x1) {
                    let e = &mut self.ext[off + y - y0 as usize];
                    e.0 = e.0.min(a as i32);
                    e.1 = e.1.max(b as i32);
                }
            }
        }
        for ci in 0..n {
            let cells = self.stats[self.shortlist[ci].1 as usize].cells;
            let (off, y0) = self.ext_off[ci];
            let end = self.ext_off.get(ci + 1).map_or(self.ext.len(), |o| o.0);
            let rows = &self.ext[off..end];
            if rows.iter().filter(|e| e.1 >= 0).count() < 2 {
                continue;
            }
            self.pts.clear();
            row_extreme_points(rows, y0, &mut self.pts);
            convex_hull_by_rows(&self.pts, &mut self.hull);
            fit_quad(
                &self.hull,
                cells as f32 * (factor * factor) as f32,
                &mut self.chain,
                &mut self.sides,
                &mut self.candidates,
                &mut self.quad_scratch,
            );
        }
    }
}

fn similar(a: &[Point; 4], b: &[Point; 4]) -> bool {
    let d = |p: Point, q: Point| {
        let (dx, dy) = (p.x - q.x, p.y - q.y);
        dx * dx + dy * dy
    };
    let side2 = d(a[0], a[1]).min(d(a[1], a[2]));
    let lim = 0.06 * 0.06 * side2;
    (0..4).any(|s| (0..4).all(|i| d(a[i], b[(i + s) % 4]) < lim))
}

/// Extreme points of the pixel rows of a component, ordered by `(y, x)`:
/// for each pixel-corner level the leftmost and rightmost corner of the
/// rows above and below it. `rows[k]` is the `(first, last)` dark pixel of
/// row `y0 + k`, `last < 0` for an empty row.
fn row_extreme_points(rows: &[(i32, i32)], y0: i32, out: &mut Vec<(i32, i32)>) {
    let valid = |k: usize| rows.get(k).filter(|e| e.1 >= 0).copied();
    for l in 0..=rows.len() {
        let above = l.checked_sub(1).and_then(valid);
        let below = valid(l);
        let (lo, hi) = match (above, below) {
            (None, None) => continue,
            (Some(a), None) | (None, Some(a)) => (a.0, a.1 + 1),
            (Some(a), Some(b)) => (a.0.min(b.0), a.1.max(b.1) + 1),
        };
        let y = y0 + l as i32;
        out.push((lo, y));
        out.push((hi, y));
    }
}

/// Convex hull of points already ordered by `(y, x)`, in the same
/// orientation as `convex_hull`.
fn convex_hull_by_rows(pts: &[(i32, i32)], hull: &mut Vec<(i32, i32)>) {
    hull.clear();
    let cr =
        |o: (i32, i32), a: (i32, i32), b: (i32, i32)| cross((o.1, o.0), (a.1, a.0), (b.1, b.0));
    if pts.len() < 3 {
        hull.extend_from_slice(pts);
        return;
    }
    for &p in pts {
        while hull.len() >= 2 && cr(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in pts.iter().rev().skip(1) {
        while hull.len() >= lower && cr(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
    hull.reverse();
    // Start at the lexicographically smallest `(x, y)` like `convex_hull`.
    if let Some(first) = (0..hull.len()).min_by_key(|&i| hull[i]) {
        hull.rotate_left(first);
    }
}

fn cross(o: (i32, i32), a: (i32, i32), b: (i32, i32)) -> i64 {
    i64::from(a.0 - o.0) * i64::from(b.1 - o.1) - i64::from(a.1 - o.1) * i64::from(b.0 - o.0)
}

/// Andrew's monotone chain; the result runs counter-clockwise in a y-up
/// frame, that is clockwise on screen.
#[cfg(test)]
fn convex_hull(pts: &mut Vec<(i32, i32)>, hull: &mut Vec<(i32, i32)>) {
    pts.sort_unstable();
    pts.dedup();
    hull.clear();
    if pts.len() < 3 {
        hull.extend_from_slice(pts);
        return;
    }
    for &p in pts.iter() {
        while hull.len() >= 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0 {
            hull.pop();
        }
        hull.push(p);
    }
    let lower = hull.len() + 1;
    for &p in pts.iter().rev().skip(1) {
        while hull.len() >= lower && cross(hull[hull.len() - 2], hull[hull.len() - 1], p) <= 0 {
            hull.pop();
        }
        hull.push(p);
    }
    hull.pop();
}

fn area_f(p: &[(f32, f32)]) -> f32 {
    let mut a = 0.0;
    for i in 0..p.len() {
        let (x0, y0) = p[i];
        let (x1, y1) = p[(i + 1) % p.len()];
        a += x0 * y1 - x1 * y0;
    }
    a / 2.0
}

/// Buffers of the quad fit.
#[derive(Debug, Default)]
struct QuadScratch {
    poly: Vec<(f32, f32)>,
    keep: Vec<bool>,
    stack: Vec<(usize, usize)>,
    on: Vec<(f32, f32)>,
}

/// Douglas-Peucker simplification of a closed polygon.
fn simplify(
    pts: &[(f32, f32)],
    eps: f32,
    out: &mut Vec<(f32, f32)>,
    keep: &mut Vec<bool>,
    stack: &mut Vec<(usize, usize)>,
) {
    let n = pts.len();
    out.clear();
    if n < 4 {
        out.extend_from_slice(pts);
        return;
    }
    keep.clear();
    keep.resize(n, false);
    let far = (1..n)
        .max_by(|&a, &b| {
            let da = hyp(pts[a].0 - pts[0].0, pts[a].1 - pts[0].1);
            let db = hyp(pts[b].0 - pts[0].0, pts[b].1 - pts[0].1);
            da.total_cmp(&db)
        })
        .unwrap_or(1);
    keep[0] = true;
    keep[far] = true;
    stack.clear();
    stack.push((0, far));
    stack.push((far, n));
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (pts[a], pts[b % n]);
        let (dx, dy) = (pb.0 - pa.0, pb.1 - pa.1);
        let len = hyp(dx, dy).max(1e-6);
        let mut worst = (0.0f32, a);
        for (i, p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = ((p.0 - pa.0) * dy - (p.1 - pa.1) * dx).abs() / len;
            if d > worst.0 {
                worst = (d, i);
            }
        }
        if worst.0 > eps {
            keep[worst.1] = true;
            stack.push((a, worst.1));
            stack.push((worst.1, b));
        }
    }
    out.extend(
        pts.iter()
            .zip(keep.iter())
            .filter(|(_, k)| **k)
            .map(|(p, _)| *p),
    );
}

/// Total least squares line through the points: (centroid, unit direction).
fn fit_line(pts: &[(f32, f32)]) -> Option<((f32, f32), (f32, f32))> {
    if pts.len() < 2 {
        return None;
    }
    let n = pts.len() as f32;
    let (mx, my) = pts.iter().fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
    let (mx, my) = (mx / n, my / n);
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for p in pts {
        let (dx, dy) = (p.0 - mx, p.1 - my);
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (s, c) = theta.sin_cos();
    Some(((mx, my), (c, s)))
}

/// A hull side as `n . p = c` with outward unit normal angle `phi`, and the
/// hull edge length lying on it.
#[derive(Debug, Clone, Copy)]
struct Side {
    phi: f32,
    sin: f32,
    cos: f32,
    c: f32,
    cover: f32,
}

/// Maps an angle within a few turns of zero into `[0, tau)` (no libm call).
fn normalize_angle(a: f32) -> f32 {
    let mut r = a;
    while r < 0.0 {
        r += std::f32::consts::TAU;
    }
    while r >= std::f32::consts::TAU {
        r -= std::f32::consts::TAU;
    }
    r
}

/// Sine and cosine of the sweep angles of `enclosing_quad`.
fn sweep_table() -> &'static [(f32, f32); SWEEP] {
    static TABLE: std::sync::OnceLock<[(f32, f32); SWEEP]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [(0.0, 0.0); SWEEP];
        for (k, e) in t.iter_mut().enumerate() {
            *e = (k as f32 * std::f32::consts::TAU / SWEEP as f32).sin_cos();
        }
        t
    })
}

/// Directions of the 1 degree style sweep of `enclosing_quad`.
const SWEEP: usize = 72;

/// Quad bounded by the four hull sides that are best supported by hull edges
/// (a side is a line carrying hull edges: the tips of a clock track, a solid
/// finder edge), with turns between consecutive sides of 55 to 125 degrees.
/// A hull edge cutting a corner is ruled out by the turn limits, so a missing
/// corner (a light module, a fragmented clock track) still gives the true
/// corner as the intersection of its two sides.
fn enclosing_quad(
    pts: &[(f32, f32)],
    poly: &[(f32, f32)],
    sides: &mut Vec<Side>,
    on: &mut Vec<(f32, f32)>,
) -> Option<[(f32, f32); 4]> {
    let h = pts.len();
    let hp = poly.len();
    let tol = |len: f32| 1.2 + 0.01 * len;
    sides.clear();
    let perimeter: f32 = (0..h)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % h]);
            hyp(b.0 - a.0, b.1 - a.1)
        })
        .sum();
    let t = tol(perimeter / 4.0);
    // Coverage of the support line at every angle: hull edge length lying
    // within the tolerance of the line.
    let cover_at = |s: f32, c: f32| {
        let off = poly
            .iter()
            .map(|p| p.0 * c + p.1 * s)
            .fold(f32::MIN, f32::max);
        let mut cover = 0.0;
        for j in 0..hp {
            let (u, v) = (poly[j], poly[(j + 1) % hp]);
            if off - (u.0 * c + u.1 * s) <= t && off - (v.0 * c + v.1 * s) <= t {
                cover += hyp(v.0 - u.0, v.1 - u.1);
            }
        }
        (cover, off)
    };
    // Candidate directions: a 1 degree sweep plus the direction of each hull
    // edge; keep the best-covered, mutually distinct ones.
    for (k, &(s, c)) in sweep_table().iter().enumerate() {
        let phi = k as f32 * std::f32::consts::TAU / SWEEP as f32;
        let (cover, off) = cover_at(s, c);
        sides.push(Side {
            phi,
            sin: s,
            cos: c,
            c: off,
            cover,
        });
    }
    for i in 0..hp {
        let (a, b) = (poly[i], poly[(i + 1) % hp]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let len = hyp(dx, dy);
        if len < 1e-6 {
            continue;
        }
        // Outward normal of the edge: the direction rotated by -90 degrees.
        let (s, c) = (-dx / len, dy / len);
        let phi = normalize_angle(s.atan2(c));
        let (cover, off) = cover_at(s, c);
        sides.push(Side {
            phi,
            sin: s,
            cos: c,
            c: off,
            cover,
        });
    }
    sides.sort_unstable_by(|a, b| b.cover.total_cmp(&a.cover));
    let mut kept = 0;
    for i in 0..sides.len() {
        let cand = sides[i];
        let dup = sides[..kept].iter().any(|q| {
            let d = normalize_angle(q.phi - cand.phi + std::f32::consts::PI) - std::f32::consts::PI;
            d.abs() < 4.0f32.to_radians()
        });
        if !dup && cand.cover > 0.0 {
            sides[kept] = cand;
            kept += 1;
        }
    }
    sides.truncate(kept);
    sides.sort_unstable_by(|a, b| b.cover.total_cmp(&a.cover));
    sides.truncate(12);
    sides.sort_unstable_by(|a, b| a.phi.total_cmp(&b.phi));
    let n = sides.len();
    if n < 4 {
        return None;
    }
    let turn_ok = |a: f32, b: f32| {
        let d = normalize_angle(b - a);
        (55.0f32.to_radians()..=125.0f32.to_radians()).contains(&d)
    };
    let mut best: Option<(f32, [usize; 4])> = None;
    for a in 0..n {
        for b in a + 1..n {
            if !turn_ok(sides[a].phi, sides[b].phi) {
                continue;
            }
            for c in b + 1..n {
                if !turn_ok(sides[b].phi, sides[c].phi) {
                    continue;
                }
                for d in c + 1..n {
                    if !turn_ok(sides[c].phi, sides[d].phi) || !turn_ok(sides[d].phi, sides[a].phi)
                    {
                        continue;
                    }
                    let score = sides[a].cover + sides[b].cover + sides[c].cover + sides[d].cover;
                    if best.is_none_or(|x| score > x.0) {
                        best = Some((score, [a, b, c, d]));
                    }
                }
            }
        }
    }
    let (_, pick) = best?;
    let mut phi = [0.0f32; 4];
    let mut c = [0.0f32; 4];
    for (k, &i) in pick.iter().enumerate() {
        let side = sides[i];
        // Refit through the hull vertices on the side.
        let (s, co) = (side.sin, side.cos);
        let t = tol(perimeter / 4.0);
        on.clear();
        on.extend(
            pts.iter()
                .copied()
                .filter(|p| (p.0 * co + p.1 * s - side.c).abs() <= t),
        );
        phi[k] = side.phi;
        c[k] = side.c;
        if on.len() >= 3
            && let Some((m, d)) = fit_line(on)
        {
            let mut normal = (d.1, -d.0);
            let mut ang = normal.1.atan2(normal.0);
            if (ang - side.phi).cos() < 0.0 {
                normal = (-normal.0, -normal.1);
                ang = normal.1.atan2(normal.0);
            }
            phi[k] = ang;
            c[k] = m.0 * normal.0 + m.1 * normal.1;
        }
    }
    quad_from_lines(&phi, &c)
}

/// Corners of the quad bounded by the lines `n(phi_k) . p = c_k`; corner k
/// joins sides k and k + 1.
fn quad_from_lines(phi: &[f32; 4], c: &[f32; 4]) -> Option<[(f32, f32); 4]> {
    let mut out = [(0.0, 0.0); 4];
    for (k, o) in out.iter_mut().enumerate() {
        let (a, b) = (k, (k + 1) % 4);
        let (sa, ca) = phi[a].sin_cos();
        let (sb, cb) = phi[b].sin_cos();
        let det = ca * sb - sa * cb;
        if det.abs() < 0.2 {
            return None;
        }
        *o = ((c[a] * sb - c[b] * sa) / det, (c[b] * ca - c[a] * cb) / det);
    }
    Some(out)
}

/// Four corners of a quad.
type Corners = [(f32, f32); 4];

/// Side lengths of a convex quad; `None` when it is not convex or too
/// small or elongated.
fn quad_sides(corners: &[(f32, f32); 4]) -> Option<(f32, f32)> {
    let mut min_side = f32::MAX;
    let mut max_side = 0.0f32;
    for j in 0..4 {
        let (a, b, c) = (corners[j], corners[(j + 1) % 4], corners[(j + 2) % 4]);
        let cr = (b.0 - a.0) * (c.1 - b.1) - (b.1 - a.1) * (c.0 - b.0);
        if cr <= 0.0 {
            return None;
        }
        let s = hyp(b.0 - a.0, b.1 - a.1);
        min_side = min_side.min(s);
        max_side = max_side.max(s);
    }
    (min_side >= MIN_SIDE && max_side <= MAX_ASPECT * min_side).then_some((min_side, max_side))
}

/// Extent of the hull edges lying on a side line, along the line.
fn side_extent(pts: &[(f32, f32)], side: &Side, t: f32) -> Option<(f32, f32)> {
    let h = pts.len();
    let (s, c) = (side.sin, side.cos);
    let mut range: Option<(f32, f32)> = None;
    for j in 0..h {
        let (u, v) = (pts[j], pts[(j + 1) % h]);
        if (side.c - (u.0 * c + u.1 * s)).abs() <= t && (side.c - (v.0 * c + v.1 * s)).abs() <= t {
            for p in [u, v] {
                let along = -p.0 * s + p.1 * c;
                range = Some(range.map_or((along, along), |r| (r.0.min(along), r.1.max(along))));
            }
        }
    }
    range
}

/// Quads completed from two solid hull sides meeting at a corner (the L
/// finder): the far ends of the two sides and their parallelogram corner.
/// Independent of the clock tracks, which fragment on large modules.
fn l_quads(
    pts: &[(f32, f32)],
    sides: &[Side],
    perimeter: f32,
    out: &mut [Option<[(f32, f32); 4]>; 2],
) {
    let t = 1.2 + 0.01 * perimeter / 4.0;
    let mut best: [(f32, Option<Corners>); 2] = [(0.0, None), (0.0, None)];
    for (ia, a) in sides.iter().enumerate() {
        for b in &sides[ia + 1..] {
            let d = normalize_angle(b.phi - a.phi).to_degrees();
            if !((55.0..=125.0).contains(&d) || (235.0..=305.0).contains(&d)) {
                continue;
            }
            if a.cover < 0.1 * perimeter || b.cover < 0.1 * perimeter {
                continue;
            }
            let ((sa, ca), (sb, cb)) = ((a.sin, a.cos), (b.sin, b.cos));
            let det = ca * sb - sa * cb;
            if det.abs() < 0.2 {
                continue;
            }
            let corner = ((a.c * sb - b.c * sa) / det, (b.c * ca - a.c * cb) / det);
            let (Some(ea), Some(eb)) = (side_extent(pts, a, t), side_extent(pts, b, t)) else {
                continue;
            };
            let along_a = -corner.0 * sa + corner.1 * ca;
            let along_b = -corner.0 * sb + corner.1 * cb;
            let (la, lb) = (ea.1 - ea.0, eb.1 - eb.0);
            let (na, nb) = (
                (along_a - ea.0).abs().min((along_a - ea.1).abs()),
                (along_b - eb.0).abs().min((along_b - eb.1).abs()),
            );
            if la < 10.0 || lb < 10.0 || na > 0.12 * la + 3.0 || nb > 0.12 * lb + 3.0 {
                continue;
            }
            let far_a = if (along_a - ea.0).abs() < (along_a - ea.1).abs() {
                ea.1
            } else {
                ea.0
            };
            let far_b = if (along_b - eb.0).abs() < (along_b - eb.1).abs() {
                eb.1
            } else {
                eb.0
            };
            let pa = (
                corner.0 - sa * (far_a - along_a),
                corner.1 + ca * (far_a - along_a),
            );
            let pb = (
                corner.0 - sb * (far_b - along_b),
                corner.1 + cb * (far_b - along_b),
            );
            let opp = (pa.0 + pb.0 - corner.0, pa.1 + pb.1 - corner.1);
            let mut quad = [corner, pa, opp, pb];
            if area_f(&quad) < 0.0 {
                quad = [corner, pb, opp, pa];
            }
            let score = a.cover + b.cover;
            // Keep the two best, distinct by their corner.
            let dup = best.iter().any(|(_, q)| {
                q.is_some_and(|q| hyp(q[0].0 - corner.0, q[0].1 - corner.1) < 0.1 * (la + lb))
            });
            if dup {
                continue;
            }
            if score > best[0].0 {
                best[1] = best[0];
                best[0] = (score, Some(quad));
            } else if score > best[1].0 {
                best[1] = (score, Some(quad));
            }
        }
    }
    out[0] = best[0].1;
    out[1] = best[1].1;
}

/// Appends the quad candidates of one hull.
fn fit_quad(
    hull: &[(i32, i32)],
    area_px: f32,
    chain: &mut Vec<(f32, f32)>,
    sides: &mut Vec<Side>,
    out: &mut Vec<Candidate>,
    scratch: &mut QuadScratch,
) {
    let h = hull.len();
    if h < 4 {
        return;
    }
    chain.clear();
    chain.extend(hull.iter().map(|&(x, y)| (x as f32, y as f32)));
    let ha = (0..h)
        .map(|i| {
            let (a, b) = (hull[i], hull[(i + 1) % h]);
            i64::from(a.0) * i64::from(b.1) - i64::from(b.0) * i64::from(a.1)
        })
        .sum::<i64>() as f32
        / 2.0;
    let density = area_px / ha.max(1.0);
    simplify(
        chain,
        1.0,
        &mut scratch.poly,
        &mut scratch.keep,
        &mut scratch.stack,
    );
    let perimeter: f32 = (0..h)
        .map(|i| {
            let (a, b) = (chain[i], chain[(i + 1) % h]);
            hyp(b.0 - a.0, b.1 - a.1)
        })
        .sum();
    let main = enclosing_quad(chain, &scratch.poly, sides, &mut scratch.on);
    if let Some(corners) = main {
        let qa = area_f(&corners);
        if qa > 0.0 && quad_sides(&corners).is_some() {
            let convexity = ha / qa;
            if (0.65..=1.2).contains(&convexity) && density >= 0.4 {
                out.push(Candidate {
                    quad: corners.map(|(x, y)| Point::new(x, y)),
                    score: convexity.min(1.0) + 0.2 * density.min(1.2),
                });
            }
        }
    }
    let mut lq = [None; 2];
    l_quads(chain, sides, perimeter, &mut lq);
    for corners in lq.into_iter().flatten() {
        if area_f(&corners) > 0.0 && quad_sides(&corners).is_some() {
            out.push(Candidate {
                quad: corners.map(|(x, y)| Point::new(x, y)),
                score: 0.9 + 0.1 * density.min(1.2),
            });
        }
    }
}

#[cfg(test)]
mod tests;
