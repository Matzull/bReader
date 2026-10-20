//! Perspective grid sampling from the binary image into a module grid.

use breader_core::{BitMatrix, Homography, ImageView, Perspective, Plane, Point};

use crate::alignment::alignment_centers;

use crate::check::{finder_mismatch, timing_mismatch};
use crate::detect::Triplet;

/// Bottom-right finder-center corner. The parallelogram point is corrected
/// for perspective: the finder sizes give the scale change along each axis,
/// hence the vanishing points, and the corner lies on the line from the
/// top-right finder towards the vertical vanishing point and the line from
/// the bottom-left finder towards the horizontal one.
pub(crate) fn fourth_point(t: &Triplet) -> Point {
    let (tl, tr, bl) = (t.tl.pos, t.tr.pos, t.bl.pos);
    let parallelogram = Point::new(tr.x + bl.x - tl.x, tr.y + bl.y - tl.y);
    let mods = [t.tl.module, t.tr.module, t.bl.module];
    let mx = mods.iter().copied().fold(0.0, f32::max);
    let mn = mods.iter().copied().fold(f32::MAX, f32::min);
    // Mild perspective: the sizes are too noisy to improve on the parallelogram.
    if t.parallel || mn <= 0.0 || mx / mn < 1.2 {
        return parallelogram;
    }
    // Direction (unit-free) of the line from `from` towards the vanishing
    // point of the axis `from -> to`, with local scales s0 (at `from`) and s1.
    let toward = |from: Point, to: Point, s0: f32, s1: f32, at: Point| -> Option<Point> {
        if s0 <= 0.0 || s1 <= 0.0 {
            return None;
        }
        let k = (s0 / s1).sqrt() - 1.0;
        let (dx, dy) = (to.x - from.x, to.y - from.y);
        if k.abs() < 0.02 {
            return Some(Point::new(at.x + dx, at.y + dy));
        }
        let _ = at;
        // Vanishing point at fraction (1 + k) / k of the axis from `from`.
        let f = (1.0 + k) / k;
        let v = Point::new(from.x + dx * f, from.y + dy * f);
        Some(v)
    };
    let (Some(vy), Some(vx)) = (
        toward(tl, bl, t.tl.module, t.bl.module, tr),
        toward(tl, tr, t.tl.module, t.tr.module, bl),
    ) else {
        return parallelogram;
    };
    // Lines tr -> vy and bl -> vx.
    let (d1, d2) = ((vy.x - tr.x, vy.y - tr.y), (vx.x - bl.x, vx.y - bl.y));
    let det = d1.0 * d2.1 - d1.1 * d2.0;
    if det.abs() < 1e-6 * (d1.0.hypot(d1.1) * d2.0.hypot(d2.1)).max(1e-12) {
        return parallelogram;
    }
    let (ex, ey) = (bl.x - tr.x, bl.y - tr.y);
    let s = (ex * d2.1 - ey * d2.0) / det;
    let p = Point::new(tr.x + d1.0 * s, tr.y + d1.1 * s);
    let span = tl.distance(tr).max(tl.distance(bl));
    if p.x.is_finite() && p.y.is_finite() && p.distance(parallelogram) < span * 0.6 {
        p
    } else {
        parallelogram
    }
}

/// Module-to-image transform from the finder centers and, when available,
/// the bottom-right alignment pattern center. Without it, the fourth point
/// comes from the parallelogram.
pub(crate) fn transform(
    t: &Triplet,
    dim: usize,
    align: Option<Point>,
    shift: Point,
) -> Option<Mapper> {
    let d = dim as f32;
    let (tl, tr, bl) = (t.tl.pos, t.tr.pos, t.bl.pos);
    let near = |x: f32, y: f32| Point::new(x, y);
    let fourth = fourth_point(t);
    let (from, to) = match align {
        Some(a) => (
            [
                near(3.5, 3.5),
                near(d - 3.5, 3.5),
                near(d - 6.5, d - 6.5),
                near(3.5, d - 3.5),
            ],
            [tl, tr, Point::new(a.x + shift.x, a.y + shift.y), bl],
        ),
        None => (
            [
                near(3.5, 3.5),
                near(d - 3.5, 3.5),
                near(d - 3.5, d - 3.5),
                near(3.5, d - 3.5),
            ],
            [
                tl,
                tr,
                Point::new(fourth.x + shift.x, fourth.y + shift.y),
                bl,
            ],
        ),
    };
    let h = Mapper::Corners(Perspective::quad_to_quad(from, to)?);
    let m = module_size(&h, dim)?;
    let ref_m = (t.tl.module + t.tr.module + t.bl.module) / 3.0;
    (m > ref_m * 0.35 && m < ref_m * 2.8).then_some(h)
}

/// Module-to-image mapping used for sampling.
#[derive(Debug, Clone)]
pub(crate) enum Mapper {
    Corners(Perspective),
    Fitted(Homography),
    /// A mapping with its module coordinates corrected along each axis.
    Warped(Box<Warped>),
}

impl Mapper {
    pub fn map(&self, p: Point) -> Option<Point> {
        match self {
            Self::Corners(h) => h.map(p),
            Self::Fitted(h) => h.map(p),
            Self::Warped(w) => w.map(p),
        }
    }
}

/// Per-boundary offsets (in modules) of the true column and row boundaries
/// relative to `base`, measured on the finder and timing patterns: a real
/// print or a creased sheet does not keep a perfectly uniform module pitch,
/// which a homography cannot follow.
#[derive(Debug, Clone)]
pub(crate) struct Warped {
    base: Mapper,
    du: Vec<f32>,
    dv: Vec<f32>,
}

impl Warped {
    fn at(d: &[f32], t: f32) -> f32 {
        let last = d.len() - 1;
        let t = t.clamp(0.0, last as f32);
        let i = (t as usize).min(last - 1);
        let f = t - i as f32;
        d[i] * (1.0 - f) + d[i + 1] * f
    }

    fn map(&self, p: Point) -> Option<Point> {
        self.base.map(Point::new(
            p.x + Self::at(&self.du, p.x),
            p.y + Self::at(&self.dv, p.y),
        ))
    }
}

/// Offset in modules (within half a module along `axis`) of the transition
/// expected at module coordinates `at`, where the module before is
/// `dark_before`.
fn locate(
    bin: Plane<'_>,
    h: &Mapper,
    at: Point,
    axis: Point,
    dark_before: bool,
    step: f32,
) -> Option<f32> {
    let pixel = |p: Point| {
        (p.x >= 0.0 && p.y >= 0.0 && (p.x as usize) < bin.width() && (p.y as usize) < bin.height())
            .then(|| bin.get(p.x as usize, p.y as usize))
    };
    let mut best: Option<f32> = None;
    let mut prev: Option<(f32, bool)> = None;
    let mut s = -0.5f32;
    while s <= 0.5 + 1e-6 {
        let v = pixel(h.map(Point::new(at.x + axis.x * s, at.y + axis.y * s))?)?;
        if let Some((ps, pv)) = prev
            && pv == dark_before
            && v != dark_before
        {
            let m = (ps + s) / 2.0;
            if best.is_none_or(|b| m.abs() < b.abs()) {
                best = Some(m);
            }
        }
        prev = Some((s, v));
        s += step;
    }
    best.filter(|s| s.abs() <= 0.42)
}

/// Corrects `h` with the column and row boundaries measured on the finders
/// and the timing patterns. `None` when too few boundaries are found.
pub(crate) fn timing_warp(bin: Plane<'_>, h: &Mapper, dim: usize, smooth: usize) -> Option<Mapper> {
    let module = module_size(h, dim)?;
    if module < 2.0 {
        return None;
    }
    let step = (0.2 / module).clamp(0.02, 0.1);
    // Boundary index, line coordinate and the shade before it, per axis.
    let mut du = vec![(0.0f32, 0u32); dim + 1];
    let mut dv = vec![(0.0f32, 0u32); dim + 1];
    let finder_edges: [(usize, bool); 5] =
        [(1, true), (2, false), (5, true), (6, false), (7, true)];
    let mut lines: Vec<(bool, usize, f32, bool)> = Vec::new();
    for horizontal in [true, false] {
        for (c, dark) in finder_edges {
            lines.push((horizontal, c, 3.5, dark));
        }
        for c in 8..=dim - 8 {
            lines.push((horizontal, c, 6.5, (c - 1) % 2 == 0));
        }
    }
    // Right finder along the top rows; bottom finder along the left columns.
    for (c, dark) in [
        (0usize, false),
        (1, true),
        (2, false),
        (5, true),
        (6, false),
    ] {
        lines.push((true, dim - 7 + c, 3.5, dark));
        lines.push((false, dim - 7 + c, 3.5, dark));
    }
    for (horizontal, c, line, dark_before) in lines {
        let (at, axis) = if horizontal {
            (Point::new(c as f32, line), Point::new(1.0, 0.0))
        } else {
            (Point::new(line, c as f32), Point::new(0.0, 1.0))
        };
        if let Some(s) = locate(bin, h, at, axis, dark_before, step) {
            let acc = if horizontal { &mut du } else { &mut dv };
            acc[c].0 += s;
            acc[c].1 += 1;
        }
    }
    let finish = |acc: &[(f32, u32)]| -> Option<Vec<f32>> {
        let known: Vec<(usize, f32)> = (1..dim)
            .filter(|&i| acc[i].1 > 0)
            .map(|i| (i, acc[i].0 / acc[i].1 as f32))
            .collect();
        if known.len() * 2 < dim {
            return None;
        }
        let mut out = vec![0.0f32; dim + 1];
        for (i, o) in out.iter_mut().enumerate() {
            let below = known.iter().rev().find(|k| k.0 <= i);
            let above = known.iter().find(|k| k.0 >= i);
            *o = match (below, above) {
                (Some(b), Some(a)) if a.0 != b.0 => {
                    let f = (i - b.0) as f32 / (a.0 - b.0) as f32;
                    b.1 * (1.0 - f) + a.1 * f
                }
                (Some(b), _) => b.1,
                (_, Some(a)) => a.1,
                _ => 0.0,
            };
        }
        let src = out.clone();
        for (i, o) in out.iter_mut().enumerate() {
            let lo = i.saturating_sub(smooth);
            let hi = (i + smooth).min(src.len() - 1);
            *o = src[lo..=hi].iter().sum::<f32>() / (hi - lo + 1) as f32;
        }
        Some(out)
    };
    let (mut du, mut dv) = (finish(&du)?, finish(&dv)?);
    // A mapping that is already warped keeps its corrections and adds the rest.
    let base = match h {
        Mapper::Warped(w) => {
            for (a, b) in du.iter_mut().zip(&w.du) {
                *a += b;
            }
            for (a, b) in dv.iter_mut().zip(&w.dv) {
                *a += b;
            }
            w.base.clone()
        }
        _ => h.clone(),
    };
    Some(Mapper::Warped(Box::new(Warped { base, du, dv })))
}

/// Alignment pattern centers (row, column indices) that do not overlap a finder.
pub(crate) fn pattern_cells(version: u8) -> impl Iterator<Item = (u8, u8)> {
    let c = alignment_centers(version);
    let last = c.len().saturating_sub(1);
    (0..c.len()).flat_map(move |i| {
        (0..c.len())
            .filter(move |&j| !((i == 0 && (j == 0 || j == last)) || (i == last && j == 0)))
            .map(move |j| (c[i], c[j]))
    })
}

/// Approximate module size in pixels at the symbol center.
pub(crate) fn module_size(h: &Mapper, dim: usize) -> Option<f32> {
    let c = dim as f32 / 2.0;
    let p = h.map(Point::new(c, c))?;
    let px = h.map(Point::new(c + 1.0, c))?;
    let py = h.map(Point::new(c, c + 1.0))?;
    Some((p.distance(px) + p.distance(py)) / 2.0)
}

/// Module axes in pixels (affine estimate from the finder centers).
pub(crate) fn module_axes(t: &Triplet, dim: usize) -> (Point, Point) {
    let span = (dim as f32 - 7.0).max(1.0);
    let (tl, tr, bl) = (t.tl.pos, t.tr.pos, t.bl.pos);
    (
        Point::new((tr.x - tl.x) / span, (tr.y - tl.y) / span),
        Point::new((bl.x - tl.x) / span, (bl.y - tl.y) / span),
    )
}

/// Predicted bottom-right alignment pattern center.
pub(crate) fn predict_alignment(t: &Triplet, dim: usize) -> Point {
    let d = dim as f32;
    let corners = [
        Point::new(3.5, 3.5),
        Point::new(d - 3.5, 3.5),
        Point::new(d - 3.5, d - 3.5),
        Point::new(3.5, d - 3.5),
    ];
    let to = [t.tl.pos, t.tr.pos, fourth_point(t), t.bl.pos];
    if let Some(h) = Perspective::quad_to_quad(corners, to)
        && let Some(p) = h.map(Point::new(d - 6.5, d - 6.5))
    {
        return p;
    }
    let (ex, ey) = module_axes(t, dim);
    Point::new(
        t.tr.pos.x + t.bl.pos.x - t.tl.pos.x - 3.0 * (ex.x + ey.x),
        t.tr.pos.y + t.bl.pos.y - t.tl.pos.y - 3.0 * (ex.y + ey.y),
    )
}

/// Largest fraction of timing-pattern modules that may disagree.
const TIMING_TOLERANCE: (u32, u32) = (7, 20);
/// Most finder/separator modules (of 192) that may disagree.
const FINDER_TOLERANCE: u32 = 48;

/// Reads one module: outer `None` when the mapping fails, inner `None` when
/// the module lies outside the image. Large modules take the majority of
/// five sub-samples.
pub(crate) fn read_module(
    bin: Plane<'_>,
    h: &Mapper,
    c: usize,
    r: usize,
    multi: bool,
) -> Option<Option<bool>> {
    const OFFSETS: [(f32, f32); 4] = [(-0.25, -0.25), (0.25, -0.25), (-0.25, 0.25), (0.25, 0.25)];
    let (w, ht) = (bin.width() as f32, bin.height() as f32);
    let pixel = |p: Point| -> Option<bool> {
        (p.x >= 0.0 && p.y >= 0.0 && p.x < w && p.y < ht)
            .then(|| bin.get(p.x as usize, p.y as usize))
    };
    let (mx, my) = (c as f32 + 0.5, r as f32 + 0.5);
    let p = h.map(Point::new(mx, my))?;
    let Some(center) = pixel(p) else {
        return Some(None);
    };
    if !multi {
        return Some(Some(center));
    }
    let mut votes = u32::from(center);
    for (ox, oy) in OFFSETS {
        if let Some(q) = h.map(Point::new(mx + ox, my + oy))
            && pixel(q) == Some(true)
        {
            votes += 1;
        }
    }
    Some(Some(votes >= 3))
}

/// Cheap test of a dimension hypothesis: the timing patterns between the
/// finders must alternate under the mapping.
pub(crate) fn timing_plausible(bin: Plane<'_>, h: &Mapper, dim: usize) -> bool {
    let multi = module_size(h, dim).is_some_and(|m| m >= 3.0);
    let mut bad = 0u32;
    for i in 8..dim - 8 {
        let want = i % 2 == 0;
        for (c, r) in [(i, 6), (6, i)] {
            match read_module(bin, h, c, r, multi) {
                None => return false,
                Some(v) => bad += u32::from(v.unwrap_or(false) != want),
            }
        }
    }
    let total = 2 * (dim as u32 - 16);
    bad * TIMING_TOLERANCE.1 <= total * TIMING_TOLERANCE.0
}

/// Cheap test of a mapping: the finder patterns and their separators must
/// appear where the mapping puts them.
pub(crate) fn finders_plausible(bin: Plane<'_>, h: &Mapper, dim: usize) -> bool {
    let multi = module_size(h, dim).is_some_and(|m| m >= 3.0);
    let mut bad = 0u32;
    // Origin of each 8x8 block (separator included) and of its 7x7 finder.
    for (bx, by, fx, fy) in [
        (0, 0, 0, 0),
        (dim - 8, 0, dim - 7, 0),
        (0, dim - 8, 0, dim - 7),
    ] {
        for r in by..by + 8 {
            for c in bx..bx + 8 {
                let want = (fx..fx + 7).contains(&c)
                    && (fy..fy + 7).contains(&r)
                    && (c - fx).abs_diff(3).max((r - fy).abs_diff(3)) != 2;
                match read_module(bin, h, c, r, multi) {
                    None => return false,
                    Some(v) => bad += u32::from(v.unwrap_or(false) != want),
                }
            }
        }
    }
    bad <= FINDER_TOLERANCE
}

/// Samples `dim` x `dim` modules into `grid`. Returns false when too much of
/// the symbol lies outside the image, or when the timing patterns and finder
/// corners of the grid are implausible (checked before the data area is read).
pub(crate) fn sample_grid(
    bin: Plane<'_>,
    h: &Mapper,
    dim: usize,
    module_px: f32,
    grid: &mut BitMatrix,
) -> bool {
    grid.resize(dim, dim);
    let multi = module_px >= 3.0;
    let read = |c: usize, r: usize| read_module(bin, h, c, r, multi);
    let mut outside = 0usize;
    let mut put = |grid: &mut BitMatrix, c: usize, r: usize| -> bool {
        match read(c, r) {
            None => false,
            Some(None) => {
                outside += 1;
                true
            }
            Some(Some(dark)) => {
                if dark {
                    grid.set(c, r, true);
                }
                true
            }
        }
    };
    let early = |c: usize, r: usize| {
        r == 6 || c == 6 || (c < 8 && (r < 8 || r >= dim - 8)) || (r < 8 && c >= dim - 8)
    };
    for i in 0..dim {
        if !put(grid, i, 6) || !put(grid, 6, i) {
            return false;
        }
    }
    let (bad, total) = timing_mismatch(grid);
    if bad * TIMING_TOLERANCE.1 > total * TIMING_TOLERANCE.0 {
        return false;
    }
    for (x0, y0) in [(0, 0), (dim - 8, 0), (0, dim - 8)] {
        for r in y0..y0 + 8 {
            for c in x0..x0 + 8 {
                if c != 6 && r != 6 && !put(grid, c, r) {
                    return false;
                }
            }
        }
    }
    if finder_mismatch(grid) > FINDER_TOLERANCE {
        return false;
    }
    for r in 0..dim {
        for c in 0..dim {
            if !early(c, r) && !put(grid, c, r) {
                return false;
            }
        }
    }
    outside * 20 <= dim * dim
}

/// Cells of the 9x9 finder template that are dark: the 7x7 pattern without
/// its inner light ring, in a light border (`WANT[j][i]`, i along x).
const FINDER_WANT: [[bool; 9]; 9] = {
    let mut t = [[false; 9]; 9];
    let mut j = 0;
    while j < 9 {
        let mut i = 0;
        while i < 9 {
            let (ai, aj) = ((i as i32 - 4).abs(), (j as i32 - 4).abs());
            let r = if ai > aj { ai } else { aj };
            t[j][i] = r <= 3 && r != 2;
            i += 1;
        }
        j += 1;
    }
    t
};

/// Sub-module refinement of a finder center: maximizes agreement of the 9x9
/// finder template (7x7 pattern in a light one-module border) with the
/// binary image, along the module axes `ex`, `ey` (pixels per module).
pub(crate) fn refine_finder(bin: Plane<'_>, c: Point, ex: Point, ey: Point) -> Point {
    // Offsets of the template cells along the axes, formed once (the same
    // products, added in the same order, as a cell-by-cell evaluation).
    let mut di = [[0.0f32; 9]; 2];
    let mut dj = [[0.0f32; 9]; 2];
    for k in 0..9 {
        let d = (k as i32 - 4) as f32;
        di[0][k] = d * ex.x;
        di[1][k] = d * ex.y;
        dj[0][k] = d * ey.x;
        dj[1][k] = d * ey.y;
    }
    let (w, h) = (bin.width(), bin.height());
    // Half-extent of the template in pixels, with a pixel to spare: a
    // template whose center is this far inside the image samples only
    // pixels inside it, and needs no per-cell bounds test.
    let rx = 4.0 * (ex.x.abs() + ey.x.abs()) + 1.0;
    let ry = 4.0 * (ex.y.abs() + ey.y.abs()) + 1.0;
    // Agreement count; gives up (returning 0) once `floor` can no longer be beaten.
    let score = |p: Point, floor: i32| -> i32 {
        let inside =
            p.x - rx >= 0.0 && p.y - ry >= 0.0 && p.x + rx < w as f32 && p.y + ry < h as f32;
        let mut s = 0;
        let mut left = 81;
        for (j, want_row) in FINDER_WANT.iter().enumerate() {
            if inside {
                // Positions of the row first, as plain array arithmetic; every
                // coordinate is inside the image, so truncating to i32 is the
                // same as truncating to usize.
                let mut xs = [0i32; 9];
                let mut ys = [0i32; 9];
                for i in 0..9 {
                    xs[i] = (p.x + di[0][i] + dj[0][j]) as i32;
                    ys[i] = (p.y + di[1][i] + dj[1][j]) as i32;
                }
                for i in 0..9 {
                    s += i32::from(bin.get(xs[i] as usize, ys[i] as usize) == want_row[i]);
                }
            } else {
                for (i, &want) in want_row.iter().enumerate() {
                    let qx = p.x + di[0][i] + dj[0][j];
                    let qy = p.y + di[1][i] + dj[1][j];
                    let got = qx >= 0.0
                        && qy >= 0.0
                        && (qx as usize) < w
                        && (qy as usize) < h
                        && bin.get(qx as usize, qy as usize);
                    s += i32::from(got == want);
                }
            }
            left -= 9;
            if s + left <= floor {
                return 0;
            }
        }
        s
    };
    let mut best = (score(c, -1), c);
    for (span, steps) in [(0.7f32, 4i32), (0.18, 3)] {
        let center = best.1;
        let step = span / steps as f32;
        for oj in -steps..=steps {
            for oi in -steps..=steps {
                let (a, b) = (oi as f32 * step, oj as f32 * step);
                let p = Point::new(
                    center.x + a * ex.x + b * ey.x,
                    center.y + a * ex.y + b * ey.y,
                );
                let sc = score(p, best.0);
                if sc > best.0 {
                    best = (sc, p);
                }
            }
        }
    }
    best.1
}

/// Luma image and its scale relative to the binary plane, for sampling
/// modules where the binarization lost them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Gray<'a> {
    pub img: ImageView<'a>,
    /// Plane coordinates times this give image coordinates.
    pub scale: (f32, f32),
}

impl Gray<'_> {
    /// Interpolated luma at a point of the binary plane.
    pub(crate) fn value_at(&self, p: Point) -> Option<f32> {
        self.at(p)
    }

    fn at(&self, p: Point) -> Option<f32> {
        let (x, y) = (p.x * self.scale.0 - 0.5, p.y * self.scale.1 - 0.5);
        let (w, h) = (self.img.width() as f32, self.img.height() as f32);
        if x < -0.5 || y < -0.5 || x > w - 0.5 || y > h - 0.5 {
            return None;
        }
        let (x, y) = (x.clamp(0.0, w - 1.0), y.clamp(0.0, h - 1.0));
        let (x0, y0) = (x as usize, y as usize);
        let (x1, y1) = (
            (x0 + 1).min(self.img.width() - 1),
            (y0 + 1).min(self.img.height() - 1),
        );
        let (fx, fy) = (x - x0 as f32, y - y0 as f32);
        let g = |x: usize, y: usize| f32::from(self.img.get(x, y));
        Some(
            (g(x0, y0) * (1.0 - fx) + g(x1, y0) * fx) * (1.0 - fy)
                + (g(x0, y1) * (1.0 - fx) + g(x1, y1) * fx) * fy,
        )
    }
}

/// Half-width in modules of the window whose mean is the local threshold.
const LOCAL_RADIUS: usize = 3;

/// Samples the grid from the luma image: each module's value (the mean over
/// its center and four nearby points) is compared with the mean of the
/// modules around it, which tolerates uneven lighting and a binarization
/// threshold that does not suit a symbol at a couple of pixels per module.
pub(crate) fn sample_grid_local(
    gray: Gray<'_>,
    inverted: bool,
    h: &Mapper,
    dim: usize,
    grid: &mut BitMatrix,
) -> bool {
    const OFFSETS: [(f32, f32); 5] = [
        (0.0, 0.0),
        (-0.2, -0.2),
        (0.2, -0.2),
        (-0.2, 0.2),
        (0.2, 0.2),
    ];
    let mut vals = vec![0.0f32; dim * dim];
    for r in 0..dim {
        for c in 0..dim {
            let mut sum = 0.0;
            for (ox, oy) in OFFSETS {
                let Some(p) = h.map(Point::new(c as f32 + 0.5 + ox, r as f32 + 0.5 + oy)) else {
                    return false;
                };
                let Some(v) = gray.at(p) else {
                    return false;
                };
                sum += v;
            }
            vals[r * dim + c] = sum / OFFSETS.len() as f32;
        }
    }
    // Summed-area table for the window means.
    let n = dim + 1;
    let mut sat = vec![0.0f64; n * n];
    for r in 0..dim {
        for c in 0..dim {
            sat[(r + 1) * n + c + 1] =
                f64::from(vals[r * dim + c]) + sat[r * n + c + 1] + sat[(r + 1) * n + c]
                    - sat[r * n + c];
        }
    }
    grid.resize(dim, dim);
    for r in 0..dim {
        for c in 0..dim {
            let (r0, r1) = (
                r.saturating_sub(LOCAL_RADIUS),
                (r + LOCAL_RADIUS + 1).min(dim),
            );
            let (c0, c1) = (
                c.saturating_sub(LOCAL_RADIUS),
                (c + LOCAL_RADIUS + 1).min(dim),
            );
            let area = ((r1 - r0) * (c1 - c0)) as f64;
            let mean =
                (sat[r1 * n + c1] - sat[r0 * n + c1] - sat[r1 * n + c0] + sat[r0 * n + c0]) / area;
            let dark = (f64::from(vals[r * dim + c]) < mean) != inverted;
            if dark {
                grid.set(c, r, true);
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use breader_core::{BitMatrix, Plane, Point};

    use super::refine_finder;

    /// The cell-by-cell evaluation that `refine_finder` replaced.
    fn refine_finder_reference(bin: Plane<'_>, c: Point, ex: Point, ey: Point) -> Point {
        let score = |p: Point, floor: i32| -> i32 {
            let mut s = 0;
            let mut left = 81;
            for j in -4i32..=4 {
                for i in -4i32..=4 {
                    let r = i.abs().max(j.abs());
                    let want = r <= 3 && r != 2;
                    let q = Point::new(
                        p.x + i as f32 * ex.x + j as f32 * ey.x,
                        p.y + i as f32 * ex.y + j as f32 * ey.y,
                    );
                    let got = q.x >= 0.0
                        && q.y >= 0.0
                        && (q.x as usize) < bin.width()
                        && (q.y as usize) < bin.height()
                        && bin.get(q.x as usize, q.y as usize);
                    s += i32::from(got == want);
                    left -= 1;
                }
                if s + left <= floor {
                    return 0;
                }
            }
            s
        };
        let mut best = (score(c, -1), c);
        for (span, steps) in [(0.7f32, 4i32), (0.18, 3)] {
            let center = best.1;
            let step = span / steps as f32;
            for oj in -steps..=steps {
                for oi in -steps..=steps {
                    let (a, b) = (oi as f32 * step, oj as f32 * step);
                    let p = Point::new(
                        center.x + a * ex.x + b * ey.x,
                        center.y + a * ex.y + b * ey.y,
                    );
                    let sc = score(p, best.0);
                    if sc > best.0 {
                        best = (sc, p);
                    }
                }
            }
        }
        best.1
    }

    fn xorshift(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    fn uniform(state: &mut u64, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * ((xorshift(state) >> 40) as f32 / (1u64 << 24) as f32)
    }

    /// Same answer as the cell-by-cell evaluation: on noise (so that early
    /// exits and ties happen), with finder-like blocks, tilted axes and
    /// centers inside the image, at its border and outside it.
    #[test]
    fn refine_finder_matches_reference() {
        let mut st = 0x9E37_79B9_7F4A_7C15u64;
        for round in 0..300 {
            let (w, h) = (
                40 + (xorshift(&mut st) % 50) as usize,
                40 + (xorshift(&mut st) % 50) as usize,
            );
            let mut bin = BitMatrix::new(w, h);
            // Noise of a varying density, then a finder pattern stamped at a
            // random place on half of the rounds.
            let density = 1 + xorshift(&mut st) % 3;
            for y in 0..h {
                for x in 0..w {
                    bin.set(x, y, xorshift(&mut st) % 4 < density);
                }
            }
            let module = uniform(&mut st, 1.2, 4.5);
            let theta = uniform(&mut st, 0.0, std::f32::consts::TAU);
            let (sn, cs) = theta.sin_cos();
            let ex = Point::new(module * cs, module * sn);
            let ey = Point::new(-module * sn, module * cs);
            let c = Point::new(
                uniform(&mut st, -3.0, w as f32 + 3.0),
                uniform(&mut st, -3.0, h as f32 + 3.0),
            );
            if round % 2 == 0 {
                for y in 0..h {
                    for x in 0..w {
                        let (dx, dy) = (x as f32 - c.x, y as f32 - c.y);
                        let u = (dx * cs + dy * sn) / module;
                        let v = (-dx * sn + dy * cs) / module;
                        let r = u.abs().max(v.abs());
                        if r < 4.0 {
                            bin.set(x, y, r <= 3.5 && !(2.5..=3.5).contains(&r) || r < 1.5);
                        }
                    }
                }
            }
            for inverted in [false, true] {
                let plane = Plane::new(&bin, inverted);
                let a = refine_finder(plane, c, ex, ey);
                let b = refine_finder_reference(plane, c, ex, ey);
                assert_eq!(
                    (a.x.to_bits(), a.y.to_bits()),
                    (b.x.to_bits(), b.y.to_bits()),
                    "round {round}"
                );
            }
        }
    }
}
