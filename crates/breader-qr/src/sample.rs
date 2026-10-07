//! Perspective grid sampling from the binary image into a module grid.

use breader_core::{BitMatrix, Perspective, Point};

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

/// Projective map fitted by least squares to point pairs.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Homography {
    m: [[f64; 3]; 3],
}

impl Homography {
    /// Fits `src -> dst` (module to image) with at least four pairs.
    pub fn fit(pairs: &[(Point, Point)]) -> Option<Self> {
        let n = pairs.len();
        if n < 4 {
            return None;
        }
        let norm = |pts: &mut dyn Iterator<Item = Point>| -> (f64, f64, f64) {
            let v: Vec<(f64, f64)> = pts.map(|p| (f64::from(p.x), f64::from(p.y))).collect();
            let (cx, cy) = (
                v.iter().map(|p| p.0).sum::<f64>() / n as f64,
                v.iter().map(|p| p.1).sum::<f64>() / n as f64,
            );
            let md = v.iter().map(|p| (p.0 - cx).hypot(p.1 - cy)).sum::<f64>() / n as f64;
            (cx, cy, if md > 1e-9 { md } else { 1.0 })
        };
        let (mcx, mcy, mmd) = norm(&mut pairs.iter().map(|p| p.0));
        let (icx, icy, imd) = norm(&mut pairs.iter().map(|p| p.1));
        let mut ata = [[0.0f64; 9]; 8];
        for (s, d) in pairs {
            let u = (f64::from(s.x) - mcx) / mmd;
            let v = (f64::from(s.y) - mcy) / mmd;
            let x = (f64::from(d.x) - icx) / imd;
            let y = (f64::from(d.y) - icy) / imd;
            let rows = [
                ([u, v, 1.0, 0.0, 0.0, 0.0, -x * u, -x * v], x),
                ([0.0, 0.0, 0.0, u, v, 1.0, -y * u, -y * v], y),
            ];
            for (r, rhs) in rows {
                for i in 0..8 {
                    for j in 0..8 {
                        ata[i][j] += r[i] * r[j];
                    }
                    ata[i][8] += r[i] * rhs;
                }
            }
        }
        for col in 0..8 {
            let piv = (col..8).max_by(|&a, &b| ata[a][col].abs().total_cmp(&ata[b][col].abs()))?;
            if ata[piv][col].abs() < 1e-12 {
                return None;
            }
            ata.swap(col, piv);
            for r in 0..8 {
                if r != col {
                    let f = ata[r][col] / ata[col][col];
                    let pivot_row = ata[col];
                    for (dst, src) in ata[r][col..].iter_mut().zip(&pivot_row[col..]) {
                        *dst -= f * src;
                    }
                }
            }
        }
        let h: Vec<f64> = (0..8).map(|i| ata[i][8] / ata[i][i]).collect();
        let hn = [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]];
        // H = Timg^-1 * Hn * Tmod
        let tmod = [
            [1.0 / mmd, 0.0, -mcx / mmd],
            [0.0, 1.0 / mmd, -mcy / mmd],
            [0.0, 0.0, 1.0],
        ];
        let timg_inv = [[imd, 0.0, icx], [0.0, imd, icy], [0.0, 0.0, 1.0]];
        let mul = |a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]| {
            let mut o = [[0.0; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
                }
            }
            o
        };
        let m = mul(&timg_inv, &mul(&hn, &tmod));
        m.iter()
            .flatten()
            .all(|v| v.is_finite())
            .then_some(Self { m })
    }

    pub fn map(&self, p: Point) -> Option<Point> {
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        let m = &self.m;
        let w = m[2][0] * x + m[2][1] * y + m[2][2];
        if w.abs() < 1e-12 {
            return None;
        }
        let u = (m[0][0] * x + m[0][1] * y + m[0][2]) / w;
        let v = (m[1][0] * x + m[1][1] * y + m[1][2]) / w;
        (u.is_finite() && v.is_finite()).then(|| Point::new(u as f32, v as f32))
    }
}

/// Module-to-image mapping used for sampling.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mapper {
    Corners(Perspective),
    Fitted(Homography),
}

impl Mapper {
    pub fn map(&self, p: Point) -> Option<Point> {
        match self {
            Self::Corners(h) => h.map(p),
            Self::Fitted(h) => h.map(p),
        }
    }
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
fn read_module(
    bin: &BitMatrix,
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
pub(crate) fn timing_plausible(bin: &BitMatrix, h: &Mapper, dim: usize) -> bool {
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
pub(crate) fn finders_plausible(bin: &BitMatrix, h: &Mapper, dim: usize) -> bool {
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
    bin: &BitMatrix,
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

/// Sub-module refinement of a finder center: maximizes agreement of the 9x9
/// finder template (7x7 pattern in a light one-module border) with the
/// binary image, along the module axes `ex`, `ey` (pixels per module).
pub(crate) fn refine_finder(bin: &BitMatrix, c: Point, ex: Point, ey: Point) -> Point {
    // Agreement count; gives up (returning 0) once `floor` can no longer be beaten.
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
