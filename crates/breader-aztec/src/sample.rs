//! From a bullseye candidate to a module grid: core fit from the ring
//! crossings, orientation probe, reference-grid anchoring and sampling.

use std::f32::consts::TAU;

use breader_core::{BitMatrix, Homography, Perspective, Plane, Point};

use crate::spec::{self, CORE_SIZE, Layout};

/// Rays cast from the center to find ring boundaries.
const RAYS: usize = 32;
/// Rotation steps of the initial similarity search, over a quarter turn.
const ANGLE_STEPS: usize = 30;
/// Fewest ring crossings a fit needs: half the rays of a compact bullseye.
const MIN_CROSSINGS: usize = 48;
/// Largest rms distance of the crossings from their rings, in modules.
const MAX_RING_RMS: f32 = 0.2;
/// Least share of the crossings that must lie on their rings.
const MIN_INLIER_PERCENT: usize = 92;
/// Iterations of the homography refinement.
const ICP_ITERATIONS: usize = 6;
/// Weight of an anchor found on the reference grid against a ring crossing.
const ANCHOR_WEIGHT: usize = 8;
/// Modules between reference grid lines.
const GRID_PITCH: i32 = 16;
/// Largest share of modules that may fall outside the image.
const MAX_OUTSIDE_PERCENT: usize = 3;

/// A boundary between bullseye rings crossed by a ray.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Crossing {
    pub p: Point,
    /// 1 for the boundary between the center module and the first ring.
    pub j: u8,
}

/// Module-to-image map of the core and its scale.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CoreFit {
    pub h: Homography,
    /// Pixels per module near the center.
    pub module: f32,
    /// Ring boundaries the fit used: 4 reach a compact bullseye, 6 a
    /// full-range one.
    pub rings: u8,
}

fn pixel(plane: Plane<'_>, p: Point) -> Option<bool> {
    (p.x >= 0.0 && p.y >= 0.0 && (p.x as usize) < plane.width() && (p.y as usize) < plane.height())
        .then(|| plane.get(p.x as usize, p.y as usize))
}

/// Pixels per module of `h` at the module-space origin.
pub(crate) fn module_size(h: &Homography) -> Option<f32> {
    let o = h.map(Point::new(0.0, 0.0))?;
    let x = h.map(Point::new(1.0, 0.0))?;
    let y = h.map(Point::new(0.0, 1.0))?;
    Some((o.distance(x) + o.distance(y)) / 2.0)
}

fn orientation_preserving(h: &Homography) -> bool {
    let (Some(o), Some(x), Some(y)) = (
        h.map(Point::new(0.0, 0.0)),
        h.map(Point::new(1.0, 0.0)),
        h.map(Point::new(0.0, 1.0)),
    ) else {
        return false;
    };
    (x.x - o.x) * (y.y - o.y) - (x.y - o.y) * (y.x - o.x) > 0.0
}

/// Casts rays from `center` and records the ring boundaries `2..=jmax`
/// along each ray whose boundaries are evenly spaced.
pub(crate) fn cast_rays(
    plane: Plane<'_>,
    center: Point,
    module: f32,
    jmax: u8,
    out: &mut Vec<Crossing>,
) {
    out.clear();
    if !pixel(plane, center).unwrap_or(false) {
        return;
    }
    let step = (module * 0.2).clamp(0.2, 0.5);
    let reach = (f32::from(jmax) + 1.0) * module * 1.8;
    let mut valid = 0;
    // The rays are visited in an order that spreads the first eight over the
    // circle, so that a false candidate fails early.
    for m in 0..RAYS {
        if m == 8 && valid < 6 {
            out.clear();
            return;
        }
        let k = (m % 8) * (RAYS / 8) + m / 8;
        let a = TAU * (k as f32 + 0.5) / RAYS as f32;
        let (sin, cos) = a.sin_cos();
        let mut found = [0.0f32; 8];
        let mut n = 0usize;
        let mut dark = true;
        let mut t = step;
        while t <= reach && n < usize::from(jmax) {
            let Some(v) = pixel(plane, Point::new(center.x + cos * t, center.y + sin * t)) else {
                break;
            };
            if v != dark {
                found[n] = t - step / 2.0;
                // Boundary j lies at (j - 1/2) modules from the center.
                if n >= 2 {
                    let (w, wref) = (found[n] / (n as f32 + 0.5), found[1] / 1.5);
                    if w < 0.7 * wref || w > 1.35 * wref {
                        break;
                    }
                }
                n += 1;
                dark = v;
            }
            t += step;
        }
        if n < 3 {
            continue;
        }
        valid += 1;
        let wref = found[1] / 1.5;
        for (i, &f) in found.iter().enumerate().take(n).skip(1) {
            if i >= 4 {
                let w = f / (i as f32 + 0.5);
                if w < 0.75 * wref || w > 1.3 * wref {
                    break;
                }
            }
            out.push(Crossing {
                p: Point::new(center.x + cos * f, center.y + sin * f),
                j: i as u8 + 1,
            });
        }
    }
}

fn similarity(center: Point, scale: f32, theta: f32) -> Option<Homography> {
    let (s, c) = theta.sin_cos();
    let pairs: Vec<(Point, Point)> = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (-1.0, 1.0)]
        .iter()
        .map(|&(u, v): &(f32, f32)| {
            (
                Point::new(u, v),
                Point::new(
                    center.x + scale * (u * c - v * s),
                    center.y + scale * (u * s + v * c),
                ),
            )
        })
        .collect();
    Homography::fit(&pairs)
}

/// Fits the module-to-image map to the ring crossings: a similarity from a
/// search over rotation, then iterated projective fits where every crossing
/// is matched to the point of its square ring on the same ray. Crossings
/// whose direction in the module frame lies closer to a diagonal than the
/// ratio `corner_skip` stay out of the fit (blur rounds the corners of the
/// rings, so with 0.8 only the straight parts of the edges count; 1 or more
/// uses all).
pub(crate) fn fit_core(
    center: Point,
    crossings: &[Crossing],
    module: f32,
    rings: u8,
    corner_skip: f32,
) -> Option<CoreFit> {
    // The perspective terms of a fit to the rings alone are weak; when they
    // run wild beyond the core, an affine map is the safer answer.
    fit_with(center, crossings, module, rings, corner_skip, false)
        .or_else(|| fit_with(center, crossings, module, rings, corner_skip, true))
}

/// Least-squares affine map through the pairs, as a homography.
fn affine_fit(pairs: &[(Point, Point)]) -> Option<Homography> {
    // Normal equations of x' = a u + b v + c (and the same for y').
    let mut ata = [[0.0f64; 3]; 3];
    let mut atx = [0.0f64; 3];
    let mut aty = [0.0f64; 3];
    for (m, p) in pairs {
        let row = [f64::from(m.x), f64::from(m.y), 1.0];
        for i in 0..3 {
            for j in 0..3 {
                ata[i][j] += row[i] * row[j];
            }
            atx[i] += row[i] * f64::from(p.x);
            aty[i] += row[i] * f64::from(p.y);
        }
    }
    let solve = |rhs: [f64; 3]| -> Option<[f64; 3]> {
        let mut a = [[0.0f64; 4]; 3];
        for i in 0..3 {
            a[i][..3].copy_from_slice(&ata[i]);
            a[i][3] = rhs[i];
        }
        for col in 0..3 {
            let piv = (col..3).max_by(|&x, &y| a[x][col].abs().total_cmp(&a[y][col].abs()))?;
            if a[piv][col].abs() < 1e-9 {
                return None;
            }
            a.swap(col, piv);
            for r in 0..3 {
                if r != col {
                    let f = a[r][col] / a[col][col];
                    let pivot_row = a[col];
                    for (dst, src) in a[r].iter_mut().zip(pivot_row) {
                        *dst -= f * src;
                    }
                }
            }
        }
        Some([a[0][3] / a[0][0], a[1][3] / a[1][1], a[2][3] / a[2][2]])
    };
    let (cx, cy) = (solve(atx)?, solve(aty)?);
    let at = |u: f32, v: f32| {
        let (u, v) = (f64::from(u), f64::from(v));
        Point::new(
            (cx[0] * u + cx[1] * v + cx[2]) as f32,
            (cy[0] * u + cy[1] * v + cy[2]) as f32,
        )
    };
    let corners: Vec<(Point, Point)> =
        [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0), (-1.0, 0.0)]
            .iter()
            .map(|&(u, v)| (Point::new(u, v), at(u, v)))
            .collect();
    Homography::fit(&corners)
}

fn fit_with(
    center: Point,
    crossings: &[Crossing],
    module: f32,
    rings: u8,
    corner_skip: f32,
    affine: bool,
) -> Option<CoreFit> {
    if crossings.len() < MIN_CROSSINGS {
        return None;
    }
    let ring = |j: u8| f32::from(j) - 0.5;
    let mut best: Option<(f32, f32, f32)> = None;
    for i in 0..ANGLE_STEPS {
        let theta = i as f32 * std::f32::consts::FRAC_PI_2 / ANGLE_STEPS as f32;
        let (s, c) = theta.sin_cos();
        let (mut qr, mut rr) = (0.0f32, 0.0f32);
        let q: Vec<f32> = crossings
            .iter()
            .map(|x| {
                let (dx, dy) = (x.p.x - center.x, x.p.y - center.y);
                let q = (dx * c + dy * s).abs().max((-dx * s + dy * c).abs());
                qr += q * ring(x.j);
                rr += ring(x.j) * ring(x.j);
                q
            })
            .collect();
        let scale = qr / rr;
        let err: f32 = crossings
            .iter()
            .zip(&q)
            .map(|(x, q)| (q - scale * ring(x.j)).powi(2))
            .sum();
        if best.is_none_or(|b| err < b.0) {
            best = Some((err, theta, scale));
        }
    }
    let (_, theta, scale) = best?;
    if scale < 1.0 || scale > module * 2.0 {
        return None;
    }
    let mut h = similarity(center, scale, theta)?;
    let mut keep = vec![true; crossings.len()];
    let mut pairs: Vec<(Point, Point)> = Vec::with_capacity(crossings.len() + 4);
    for iter in 0..ICP_ITERATIONS {
        let inv = h.inverse()?;
        pairs.clear();
        for (x, &k) in crossings.iter().zip(&keep) {
            if !k {
                continue;
            }
            let Some(m) = inv.map(x.p) else { continue };
            let norm = m.x.abs().max(m.y.abs());
            if norm < 1e-3 || m.x.abs().min(m.y.abs()) > corner_skip * norm {
                continue;
            }
            // The nearest point of the ring: independent of where the ray
            // started.
            let rho = ring(x.j);
            let target = if m.x.abs() >= m.y.abs() {
                Point::new(rho.copysign(m.x), m.y)
            } else {
                Point::new(m.x, rho.copysign(m.y))
            };
            pairs.push((target, x.p));
        }
        h = if affine {
            affine_fit(&pairs)?
        } else {
            Homography::fit(&pairs)?
        };
        if iter >= 1 {
            let mpx = module_size(&h)?;
            let inv = h.inverse()?;
            for (x, k) in crossings.iter().zip(keep.iter_mut()) {
                *k = inv.map(x.p).is_some_and(|m| {
                    (m.x.abs().max(m.y.abs()) - ring(x.j)).abs() * mpx < 0.4 * mpx.max(1.0)
                });
            }
            if keep.iter().filter(|&&k| k).count() * 2 < crossings.len() {
                return None;
            }
        }
    }
    let module = module_size(&h)?;
    // A bullseye leaves nearly every crossing on its square ring.
    let inv = h.inverse()?;
    let (mut inliers, mut sq) = (0usize, 0.0f32);
    for x in crossings {
        if let Some(m) = inv.map(x.p) {
            let e = (m.x.abs().max(m.y.abs()) - ring(x.j)).abs();
            if e < 0.35 {
                inliers += 1;
                sq += e * e;
            }
        }
    }
    if inliers * 100 < crossings.len() * MIN_INLIER_PERCENT
        || sq > MAX_RING_RMS * MAX_RING_RMS * inliers as f32
    {
        return None;
    }
    (orientation_preserving(&h) && module > 0.8 && mild_perspective(&h)).then_some(CoreFit {
        h,
        module,
        rings,
    })
}

/// Pixels per module of `h` around the module-space point `p`.
fn local_scale(h: &Homography, p: Point) -> Option<f32> {
    let o = h.map(p)?;
    let x = h.map(Point::new(p.x + 1.0, p.y))?;
    let y = h.map(Point::new(p.x, p.y + 1.0))?;
    Some((o.distance(x) + o.distance(y)) / 2.0)
}

/// Whether the scale across the core stays within what a symbol seen at an
/// angle shows: a wilder fit is a pattern that merely resembles rings.
fn mild_perspective(h: &Homography) -> bool {
    let Some(center) = local_scale(h, Point::new(0.0, 0.0)) else {
        return false;
    };
    [(-13.0, -13.0), (13.0, -13.0), (13.0, 13.0), (-13.0, 13.0)]
        .iter()
        .all(|&(u, v)| {
            local_scale(h, Point::new(u, v)).is_some_and(|s| s > 0.6 * center && s < 1.7 * center)
        })
}

/// Module-to-image mapping for sampling: one homography, or a homography
/// plus the residuals measured at the reference grid intersections.
#[derive(Debug, Clone)]
pub(crate) enum Mapper {
    Global(Homography),
    Warped(Box<Warped>),
}

/// A homography corrected by the offsets (in pixels) of the anchors found on
/// the reference grid from where it puts them, interpolated between the grid
/// lines and held constant beyond the outermost ones: it follows a warped
/// sheet without extrapolating the anchors' noise to the symbol edge.
#[derive(Debug, Clone)]
pub(crate) struct Warped {
    global: Homography,
    /// Grid lines at multiples of the pitch from `-m` to `m`.
    m: i32,
    /// Residuals at the intersections, row after row.
    residual: Vec<Point>,
}

impl Mapper {
    /// Image position of the module at offset (`u`, `v`) from the center.
    pub fn map(&self, p: Point) -> Option<Point> {
        match self {
            Self::Global(h) => h.map(p),
            Self::Warped(w) => {
                let g = w.global.map(p)?;
                let pitch = GRID_PITCH as f32;
                let m = w.m as f32;
                let (u, v) = ((p.x / pitch).clamp(-m, m), (p.y / pitch).clamp(-m, m));
                let (i, j) = (
                    (u.floor() as i32).min(w.m - 1),
                    (v.floor() as i32).min(w.m - 1),
                );
                let (fu, fv) = (u - i as f32, v - j as f32);
                let n = 2 * w.m + 1;
                let at = |i: i32, j: i32| w.residual[((j + w.m) * n + (i + w.m)) as usize];
                let mix = |a: Point, b: Point, t: f32| {
                    Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
                };
                let top = mix(at(i, j), at(i + 1, j), fu);
                let bottom = mix(at(i, j + 1), at(i + 1, j + 1), fu);
                let r = mix(top, bottom, fv);
                Some(Point::new(g.x + r.x, g.y + r.y))
            }
        }
    }

    pub fn module(&self) -> Option<f32> {
        match self {
            Self::Global(h) => module_size(h),
            Self::Warped(w) => module_size(&w.global),
        }
    }
}

/// Value of the module whose center maps to `p`: the center pixel for small
/// modules, the majority of five samples for larger ones.
fn read_module(plane: Plane<'_>, p: Point, module: f32) -> Option<bool> {
    if module < 3.0 {
        return pixel(plane, p);
    }
    let d = module * 0.25;
    let mut dark = 0;
    for (ox, oy) in [(0.0, 0.0), (d, 0.0), (-d, 0.0), (0.0, d), (0.0, -d)] {
        dark += usize::from(pixel(plane, Point::new(p.x + ox, p.y + oy))?);
    }
    Some(dark >= 3)
}

/// Samples a `size` x `size` grid centered on the symbol center. `false` when
/// too many modules fall outside the image.
pub(crate) fn sample_grid(
    plane: Plane<'_>,
    mapper: &Mapper,
    size: usize,
    out: &mut BitMatrix,
) -> bool {
    sample_grid_within(plane, mapper, size, out, MAX_OUTSIDE_PERCENT)
}

/// Like [`sample_grid`] with a different limit on the modules that may fall
/// outside the image (they read as light).
pub(crate) fn sample_grid_within(
    plane: Plane<'_>,
    mapper: &Mapper,
    size: usize,
    out: &mut BitMatrix,
    max_outside_percent: usize,
) -> bool {
    let Some(module) = mapper.module() else {
        return false;
    };
    out.resize(size, size);
    let c = (size / 2) as f32;
    let mut outside = 0usize;
    for y in 0..size {
        for x in 0..size {
            let q = mapper.map(Point::new(x as f32 - c, y as f32 - c));
            match q.and_then(|q| read_module(plane, q, module)) {
                Some(true) => out.set(x, y, true),
                Some(false) => {}
                None => outside += 1,
            }
        }
    }
    outside * 100 <= size * size * max_outside_percent
}

/// The grid rotated `rot` quarter turns, then transposed when `mirror`:
/// the orientations a symbol can be read in.
pub(crate) fn orient(grid: &BitMatrix, rot: usize, mirror: bool) -> BitMatrix {
    let n = grid.width();
    let mut cur = grid.clone();
    for _ in 0..rot {
        let mut next = BitMatrix::new(n, n);
        for y in 0..n {
            for x in 0..n {
                if cur.get(y, n - 1 - x) {
                    next.set(x, y, true);
                }
            }
        }
        cur = next;
    }
    if mirror {
        crate::decode::transpose(&cur)
    } else {
        cur
    }
}

/// Whether the modules on the reference grid lines of a full-range grid
/// alternate as they should (at least 80 %); trivially true without lines.
pub(crate) fn grid_lines_plausible(grid: &BitMatrix, layout: Layout) -> bool {
    if layout.compact {
        return true;
    }
    let size = grid.width();
    let c = size / 2;
    let (mut total, mut ok) = (0usize, 0usize);
    for y in 0..size {
        for x in 0..size {
            if x.abs_diff(c).max(y.abs_diff(c)) <= 7 || !spec::on_grid_line(size, x, y) {
                continue;
            }
            let other = if x.abs_diff(c).is_multiple_of(GRID_PITCH as usize) {
                y.abs_diff(c)
            } else {
                x.abs_diff(c)
            };
            total += 1;
            ok += usize::from(grid.get(x, y) == other.is_multiple_of(2));
        }
    }
    total == 0 || ok * 5 >= total * 4
}

/// Template of the reference grid around the intersection at `nominal`:
/// the alternating modules of both lines, dark at even offsets, as far as
/// they lie inside a symbol that reaches `half` modules from its center.
fn grid_template(nominal: Point, half: f32) -> Vec<(f32, f32, bool)> {
    let mut out = vec![(0.0, 0.0, true)];
    for a in 1..=5 {
        let dark = a % 2 == 0;
        let a = a as f32;
        for (dx, dy) in [(a, 0.0), (-a, 0.0), (0.0, a), (0.0, -a)] {
            if (nominal.x + dx).abs() <= half && (nominal.y + dy).abs() <= half {
                out.push((dx, dy, dark));
            }
        }
    }
    out
}

/// Matches of the template placed at module position `at` through `h`.
fn template_score(
    plane: Plane<'_>,
    h: &Homography,
    at: Point,
    template: &[(f32, f32, bool)],
) -> usize {
    template
        .iter()
        .filter(|&&(dx, dy, dark)| {
            h.map(Point::new(at.x + dx, at.y + dy))
                .and_then(|p| pixel(plane, p))
                == Some(dark)
        })
        .count()
}

/// Fewest template points worth a decision.
const MIN_TEMPLATE_POINTS: usize = 18;
/// The template matches equally two modules away along a line, so the search
/// stays within this many modules of the prediction.
const ANCHOR_REACH: f32 = 0.9;

/// Position, in modules, of the grid intersection near `nominal`, if the
/// template fits at least 80 % of its points there. Every placement of the
/// template within about half a module of the true one fits equally well, so
/// the answer is the middle of the placements that fit best. `half` is the
/// distance from the symbol center to its edge, in modules.
fn find_anchor(plane: Plane<'_>, h: &Homography, nominal: Point, half: f32) -> Option<Point> {
    let template = grid_template(nominal, half);
    if template.len() < MIN_TEMPLATE_POINTS {
        return None;
    }
    let score_at = |p: Point| template_score(plane, h, p, &template);
    let mut coarse: Option<(usize, f32, Point)> = None;
    for iy in -3..=3 {
        for ix in -3..=3 {
            let p = Point::new(nominal.x + ix as f32 * 0.3, nominal.y + iy as f32 * 0.3);
            let (s, off) = (score_at(p), p.distance(nominal));
            if coarse.is_none_or(|b| s > b.0 || (s == b.0 && off < b.1)) {
                coarse = Some((s, off, p));
            }
        }
    }
    let (_, _, start) = coarse?;
    let mut scored: Vec<(usize, Point)> = Vec::with_capacity(81);
    for iy in -6..=6 {
        for ix in -6..=6 {
            let p = Point::new(start.x + ix as f32 * 0.125, start.y + iy as f32 * 0.125);
            scored.push((score_at(p), p));
        }
    }
    let best = scored.iter().map(|s| s.0).max()?;
    if best * 5 < template.len() * 4 {
        return None;
    }
    // The placements that fit best form one blob around the true position
    // (and, two modules away along a line, copies of it): take the middle of
    // the blob nearest to the coarse optimum.
    let at = |ix: i32, iy: i32| scored[((iy + 6) * 13 + ix + 6) as usize];
    let seed = (-6..=6)
        .flat_map(|iy| (-6..=6).map(move |ix| (ix, iy)))
        .filter(|&(ix, iy)| at(ix, iy).0 == best)
        .min_by_key(|&(ix, iy)| ix * ix + iy * iy)?;
    let mut blob = vec![seed];
    let mut next = 0;
    while next < blob.len() {
        let (ix, iy) = blob[next];
        next += 1;
        for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let q = (ix + dx, iy + dy);
            if (-6..=6).contains(&q.0)
                && (-6..=6).contains(&q.1)
                && at(q.0, q.1).0 == best
                && !blob.contains(&q)
            {
                blob.push(q);
            }
        }
    }
    let n = blob.len() as f32;
    let p = Point::new(
        blob.iter().map(|&(ix, iy)| at(ix, iy).1.x).sum::<f32>() / n,
        blob.iter().map(|&(ix, iy)| at(ix, iy).1.y).sum::<f32>() / n,
    );
    (p.distance(nominal) <= ANCHOR_REACH).then_some(p)
}

/// Re-anchors a full-range symbol on its reference grid. Intersections are
/// searched ring by ring outwards; each ring that is found refines the global
/// fit used to predict the next. Returns the global fit corrected by the
/// anchors found, or the fit alone when there is none.
pub(crate) fn anchor_grid(plane: Plane<'_>, fit: &CoreFit, size: usize) -> Mapper {
    let m = (size / 2) as i32 / GRID_PITCH;
    if m == 0 {
        return Mapper::Global(fit.h);
    }
    // The core as fitted and refined stays in the least-squares fits.
    let mut pairs: Vec<(Point, Point)> = Vec::new();
    for v in -3..=3 {
        for u in -3..=3 {
            let p = Point::new(u as f32 * 2.0, v as f32 * 2.0);
            if let Some(q) = fit.h.map(p) {
                pairs.push((p, q));
            }
        }
    }
    let mut h = fit.h;
    let mut found: Vec<(i32, i32, Point)> = Vec::new();
    for ring in 1..=m {
        let mut added = false;
        for j in -ring..=ring {
            for i in -ring..=ring {
                if i.abs().max(j.abs()) != ring {
                    continue;
                }
                let at = Point::new((i * GRID_PITCH) as f32, (j * GRID_PITCH) as f32);
                let Some(p) = find_anchor(plane, &h, at, (size / 2) as f32) else {
                    continue;
                };
                let Some(img) = h.map(p) else { continue };
                found.push((i, j, img));
                for _ in 0..ANCHOR_WEIGHT {
                    pairs.push((at, img));
                }
                added = true;
            }
        }
        if added
            && let Some(refit) = Homography::fit(&pairs)
            && orientation_preserving(&refit)
        {
            h = refit;
        }
    }
    if found.is_empty() {
        return Mapper::Global(h);
    }
    let n = 2 * m + 1;
    let mut residual = vec![Point::default(); (n * n) as usize];
    for &(i, j, img) in &found {
        let at = Point::new((i * GRID_PITCH) as f32, (j * GRID_PITCH) as f32);
        if let Some(g) = h.map(at) {
            residual[((j + m) * n + (i + m)) as usize] = Point::new(img.x - g.x, img.y - g.y);
        }
    }
    Mapper::Warped(Box::new(Warped {
        global: h,
        m,
        residual,
    }))
}

/// Maps a point of the oriented grid (offsets from the center) back to the
/// sampled frame.
pub(crate) fn unorient(mut p: Point, rot: usize, mirror: bool) -> Point {
    if mirror {
        p = Point::new(p.y, p.x);
    }
    for _ in 0..rot {
        p = Point::new(p.y, -p.x);
    }
    p
}

/// Expected modules of a core: the bullseye rings and, in each of the eight
/// orientations, the orientation marks (offsets in the sampled frame).
struct CoreModel {
    bullseye: Vec<(f32, f32, bool)>,
    marks: Vec<[(f32, f32, bool); 12]>,
}

impl CoreModel {
    fn new(full: bool) -> Self {
        let (rb, rm): (i32, i32) = if full { (6, 7) } else { (4, 5) };
        let mut bullseye = Vec::new();
        for dy in -rb..=rb {
            for dx in -rb..=rb {
                let d = dx.abs().max(dy.abs()) as usize;
                bullseye.push((dx as f32, dy as f32, spec::bullseye_dark(d)));
            }
        }
        let mut marks = Vec::new();
        for mirror in [false, true] {
            for rot in 0..4 {
                marks.push(spec::orientation_marks(rm).map(|(dx, dy, dark)| {
                    let p = unorient(Point::new(dx as f32, dy as f32), rot, mirror);
                    (p.x, p.y, dark)
                }));
            }
        }
        Self { bullseye, marks }
    }

    /// Share of the five samples of the module at (`dx`, `dy`) that have the
    /// expected color.
    fn agree(plane: Plane<'_>, h: &Homography, m: (f32, f32, bool)) -> f32 {
        const OFFSETS: [(f32, f32); 5] =
            [(0.0, 0.0), (0.3, 0.0), (-0.3, 0.0), (0.0, 0.3), (0.0, -0.3)];
        let hits = OFFSETS
            .iter()
            .filter(|(ox, oy)| {
                h.map(Point::new(m.0 + ox, m.1 + oy))
                    .and_then(|p| pixel(plane, p))
                    == Some(m.2)
            })
            .count();
        hits as f32 / 5.0
    }

    fn score(&self, plane: Plane<'_>, h: &Homography) -> f32 {
        let ring: f32 = self
            .bullseye
            .iter()
            .map(|&m| Self::agree(plane, h, m))
            .sum();
        let marks = self
            .marks
            .iter()
            .map(|variant| {
                variant
                    .iter()
                    .map(|&m| Self::agree(plane, h, m))
                    .sum::<f32>()
            })
            .fold(0.0, f32::max);
        ring + 4.0 * marks
    }
}

/// Moves the corners of the core square by a few tenths of a module while
/// the bullseye and the orientation marks match better: lens distortion and
/// blur let a homography fitted to the rings drift off at the outer corners
/// of the core. Returns the original map when nothing improves.
pub(crate) fn refine_core(plane: Plane<'_>, fit: &CoreFit, full: bool) -> CoreFit {
    let model = CoreModel::new(full);
    let half = if full { 7.0 } else { 5.0 };
    let corners = [(-half, -half), (half, -half), (half, half), (-half, half)];
    let Some(base): Option<Vec<Point>> = corners
        .iter()
        .map(|&(u, v)| fit.h.map(Point::new(u, v)))
        .collect()
    else {
        return *fit;
    };
    let build = |delta: &[f32; 8]| -> Option<Homography> {
        let moved: Vec<Point> = base
            .iter()
            .enumerate()
            .map(|(i, p)| {
                Point::new(
                    p.x + delta[2 * i] * fit.module,
                    p.y + delta[2 * i + 1] * fit.module,
                )
            })
            .collect();
        let from = corners.map(|(u, v)| Point::new(u, v));
        let persp = Perspective::quad_to_quad(from, [moved[0], moved[1], moved[2], moved[3]])?;
        let mut pairs = Vec::new();
        for v in [-half, 0.0, half] {
            for u in [-half, 0.0, half] {
                let p = Point::new(u, v);
                pairs.push((p, persp.map(p)?));
            }
        }
        let h = Homography::fit(&pairs)?;
        orientation_preserving(&h).then_some(h)
    };
    let mut delta = [0.0f32; 8];
    let mut best = model.score(plane, &fit.h);
    let mut best_h = fit.h;
    for step in [0.4f32, 0.2, 0.1] {
        for _ in 0..3 {
            let mut improved = false;
            for i in 0..8 {
                for sign in [1.0f32, -1.0] {
                    let mut trial = delta;
                    trial[i] += sign * step;
                    let Some(h) = build(&trial) else { continue };
                    let s = model.score(plane, &h);
                    if s > best {
                        best = s;
                        best_h = h;
                        delta = trial;
                        improved = true;
                        break;
                    }
                }
            }
            if !improved {
                break;
            }
        }
    }
    CoreFit {
        h: best_h,
        module: module_size(&best_h).unwrap_or(fit.module),
        rings: fit.rings,
    }
}

/// How sharply the modules of a `size` x `size` symbol sampled through `h`
/// read: a module whose five samples agree sits on a module, one whose
/// samples disagree straddles an edge.
fn crispness(plane: Plane<'_>, h: &Homography, size: usize) -> i32 {
    const OFFSETS: [(f32, f32); 5] = [(0.0, 0.0), (0.3, 0.0), (-0.3, 0.0), (0.0, 0.3), (0.0, -0.3)];
    let c = (size / 2) as f32;
    let mut total = 0;
    for y in 0..size {
        for x in 0..size {
            let (u, v) = (x as f32 - c, y as f32 - c);
            let mut dark = 0;
            for (ox, oy) in OFFSETS {
                if h.map(Point::new(u + ox, v + oy))
                    .and_then(|p| pixel(plane, p))
                    == Some(true)
                {
                    dark += 1;
                }
            }
            total += (2 * dark - 5i32).abs();
        }
    }
    total
}

/// Moves the corners of the symbol by fractions of a module while its modules
/// read more crisply: a last resort for a symbol that is tilted more than
/// the rings alone can tell. `None` when nothing improves.
pub(crate) fn refine_symbol(plane: Plane<'_>, fit: &CoreFit, size: usize) -> Option<CoreFit> {
    let half = (size / 2) as f32;
    let corners = [(-half, -half), (half, -half), (half, half), (-half, half)];
    let base: Vec<Point> = corners
        .iter()
        .map(|&(u, v)| fit.h.map(Point::new(u, v)))
        .collect::<Option<_>>()?;
    let build = |delta: &[f32; 8]| -> Option<Homography> {
        let moved: Vec<Point> = base
            .iter()
            .enumerate()
            .map(|(i, p)| {
                Point::new(
                    p.x + delta[2 * i] * fit.module,
                    p.y + delta[2 * i + 1] * fit.module,
                )
            })
            .collect();
        let from = corners.map(|(u, v)| Point::new(u, v));
        let persp = Perspective::quad_to_quad(from, [moved[0], moved[1], moved[2], moved[3]])?;
        let mut pairs = Vec::new();
        for v in [-half, 0.0, half] {
            for u in [-half, 0.0, half] {
                let p = Point::new(u, v);
                pairs.push((p, persp.map(p)?));
            }
        }
        let h = Homography::fit(&pairs)?;
        orientation_preserving(&h).then_some(h)
    };
    let mut delta = [0.0f32; 8];
    let start = crispness(plane, &fit.h, size);
    let mut best = start;
    let mut best_h = fit.h;
    for step in [0.5f32, 0.25, 0.12] {
        for _ in 0..3 {
            let mut improved = false;
            for i in 0..8 {
                for sign in [1.0f32, -1.0] {
                    let mut trial = delta;
                    trial[i] += sign * step;
                    let Some(h) = build(&trial) else { continue };
                    let score = crispness(plane, &h, size);
                    if score > best {
                        best = score;
                        best_h = h;
                        delta = trial;
                        improved = true;
                        break;
                    }
                }
            }
            if !improved {
                break;
            }
        }
    }
    (best > start).then(|| CoreFit {
        h: best_h,
        module: module_size(&best_h).unwrap_or(fit.module),
        rings: fit.rings,
    })
}

/// Share of the bullseye modules (compact: rings up to 4, full-range: up to
/// 6) of a sampled core that are as expected.
pub(crate) fn bullseye_match(core: &BitMatrix, full: bool) -> f32 {
    let c = (core.width() / 2) as i32;
    let rb: i32 = if full { 6 } else { 4 };
    let mut ok = 0;
    for dy in -rb..=rb {
        for dx in -rb..=rb {
            let d = dx.abs().max(dy.abs()) as usize;
            ok += usize::from(
                core.get((c + dx) as usize, (c + dy) as usize) == spec::bullseye_dark(d),
            );
        }
    }
    ok as f32 / ((2 * rb + 1) * (2 * rb + 1)) as f32
}

/// Samples the central 15 x 15 modules through `h`.
pub(crate) fn sample_core(plane: Plane<'_>, fit: &CoreFit, out: &mut BitMatrix) -> bool {
    // A symbol without quiet zone may end at the image border, which cuts
    // the outer modules of the core off.
    sample_grid_within(plane, &Mapper::Global(fit.h), CORE_SIZE, out, 100)
}

#[cfg(test)]
mod tests;
