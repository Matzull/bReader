//! Orientation from the L finder and clock tracks, homography refinement
//! and module sampling.

use breader_core::{BitMatrix, Perspective, Plane, Point};

use crate::spec::{self, SymbolSize};

/// Fraction of dark samples along a side for it to count as part of the L.
const SOLID_MIN: f32 = 0.78;
/// The same test on the unrefined quad, with coarse sampling, accepts a bit less.
const PRECHECK_SOLID_MIN: f32 = SOLID_MIN - 0.05;
/// Sides at or above this fraction cannot be clock tracks.
const CLOCK_SOLID_MAX: f32 = 0.95;
/// Minimum share of clock runs with a regular length.
const REGULAR_MIN: f32 = 0.7;
/// Smallest module (pixels) at which the geometry searches (corner search, broad size hypotheses, track warp) run: below it a pattern is read from one or two pixels and any pattern fits.
const MIN_SEARCH_MODULE: f32 = 2.0;
/// Largest ratio between a size and the module count of a clock track for the size to be searched.
const EST_FACTOR: f32 = 2.2;
/// Largest relative size mismatch accepted when snapping to the size table.
const SNAP_MAX_COST: f32 = 0.22;

/// Row-major 3x3 homography in f64, module coordinates to pixels.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Mapper {
    m: [[f64; 3]; 3],
}

impl Mapper {
    pub fn map(&self, u: f64, v: f64) -> Option<(f64, f64)> {
        let m = &self.m;
        let w = m[2][0] * u + m[2][1] * v + m[2][2];
        if w.abs() < 1e-12 {
            return None;
        }
        let x = (m[0][0] * u + m[0][1] * v + m[0][2]) / w;
        let y = (m[1][0] * u + m[1][1] * v + m[1][2]) / w;
        (x.is_finite() && y.is_finite()).then_some((x, y))
    }

    pub fn map_point(&self, u: f32, v: f32) -> Option<Point> {
        self.map(f64::from(u), f64::from(v))
            .map(|(x, y)| Point::new(x as f32, y as f32))
    }

    /// Weighted least-squares fit of module -> pixel pairs `(u, v, x, y, weight)`.
    pub fn fit(pairs: &[(f64, f64, f64, f64, f64)]) -> Option<Self> {
        if pairs.len() < 4 {
            return None;
        }
        let n = pairs.len() as f64;
        let (mut cu, mut cv, mut cx, mut cy) = (0.0, 0.0, 0.0, 0.0);
        for p in pairs {
            cu += p.0;
            cv += p.1;
            cx += p.2;
            cy += p.3;
        }
        let (cu, cv, cx, cy) = (cu / n, cv / n, cx / n, cy / n);
        let (mut su, mut sx) = (0.0, 0.0);
        for p in pairs {
            su += (p.0 - cu).hypot(p.1 - cv);
            sx += (p.2 - cx).hypot(p.3 - cy);
        }
        let (su, sx) = (su / n, sx / n);
        if su < 1e-9 || sx < 1e-9 {
            return None;
        }
        let mut ata = [[0.0f64; 9]; 8];
        for p in pairs {
            let (u, v) = ((p.0 - cu) / su, (p.1 - cv) / su);
            let (x, y) = ((p.2 - cx) / sx, (p.3 - cy) / sx);
            let w = p.4;
            let r1 = [u, v, 1.0, 0.0, 0.0, 0.0, -x * u, -x * v, x];
            let r2 = [0.0, 0.0, 0.0, u, v, 1.0, -y * u, -y * v, y];
            for r in [r1, r2] {
                for i in 0..8 {
                    for j in 0..9 {
                        ata[i][j] += w * r[i] * r[j];
                    }
                }
            }
        }
        for col in 0..8 {
            let mut piv = col;
            for r in col + 1..8 {
                if ata[r][col].abs() > ata[piv][col].abs() {
                    piv = r;
                }
            }
            if ata[piv][col].abs() < 1e-12 {
                return None;
            }
            ata.swap(col, piv);
            let prow = ata[col];
            for (r, row) in ata.iter_mut().enumerate() {
                if r != col {
                    let f = row[col] / prow[col];
                    for (x, p) in row[col..].iter_mut().zip(&prow[col..]) {
                        *x -= f * p;
                    }
                }
            }
        }
        let h: Vec<f64> = (0..8).map(|i| ata[i][8] / ata[i][i]).collect();
        let hn = [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]];
        // H = Ti^-1 * Hn * Tm
        let tm = [
            [1.0 / su, 0.0, -cu / su],
            [0.0, 1.0 / su, -cv / su],
            [0.0, 0.0, 1.0],
        ];
        let ti = [[sx, 0.0, cx], [0.0, sx, cy], [0.0, 0.0, 1.0]];
        let mul = |a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]| {
            let mut r = [[0.0; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    r[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
                }
            }
            r
        };
        let m = mul(&ti, &mul(&hn, &tm));
        m.iter()
            .flatten()
            .all(|v| v.is_finite())
            .then_some(Self { m })
    }

    /// Mapper sending the module rectangle corners to the pixel corners
    /// (top-left, top-right, bottom-right, bottom-left).
    pub fn from_corners(c: &[Point; 4], rows: usize, cols: usize) -> Option<Self> {
        let (w, h) = (cols as f64, rows as f64);
        let uv = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)];
        let pairs: Vec<(f64, f64, f64, f64, f64)> = uv
            .iter()
            .zip(c)
            .map(|(&(u, v), p)| (u, v, f64::from(p.x), f64::from(p.y), 1.0))
            .collect();
        Self::fit(&pairs)
    }

    /// Local module size in pixels around the symbol center.
    pub fn module_px(&self, rows: usize, cols: usize) -> f32 {
        let (u, v) = (cols as f64 / 2.0, rows as f64 / 2.0);
        let (Some(p0), Some(p1), Some(p2)) =
            (self.map(u, v), self.map(u + 1.0, v), self.map(u, v + 1.0))
        else {
            return 0.0;
        };
        (((p1.0 - p0.0).hypot(p1.1 - p0.1) + (p2.0 - p0.0).hypot(p2.1 - p0.1)) / 2.0) as f32
    }
}

/// Reusable buffers of the sampling stage.
#[derive(Debug, Default)]
pub(crate) struct SampleScratch {
    runs: Vec<(bool, u32)>,
    lens: Vec<u32>,
    mods: Vec<f32>,
    pairs: Vec<(f64, f64, f64, f64, f64)>,
    transitions: Vec<(f64, f64, f64, f64)>,
    edge: Vec<(f32, f32)>,
    inliers: Vec<(f32, f32)>,
    /// Raw clock samples of the last `clock_runs` call.
    samples: Vec<bool>,
    /// Clock samples of the top and right track of the last layout.
    top_samples: Vec<bool>,
    right_samples: Vec<bool>,
    /// Clock track points of the corner search.
    pattern: Vec<(f32, f32, bool)>,
    /// Runs along a clock track of `track_boundaries` (colour, start, end).
    track_runs: Vec<(bool, f64, f64)>,
}

/// Orientation and size evidence of a quad.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Layout {
    /// Top-left, top-right, bottom-right, bottom-left in the reading
    /// orientation (L finder at the bottom-left).
    pub corners: [Point; 4],
    /// Modules counted along the right clock track.
    pub rows_est: usize,
    /// Modules counted along the top clock track.
    pub cols_est: usize,
    /// Rows and columns implied by the side lengths and the module size.
    pub rows_geo: f32,
    pub cols_geo: f32,
}

fn dist(a: Point, b: Point) -> f32 {
    crate::detect::hyp(a.x - b.x, a.y - b.y)
}

fn plane_at(plane: Plane<'_>, p: Point) -> bool {
    // Truncation equals floor for non-negative coordinates.
    if p.x < 0.0 || p.y < 0.0 {
        return false;
    }
    // `i32` casts are single instructions; `usize` casts need extra checks.
    let (x, y) = (p.x as i32 as usize, p.y as i32 as usize);
    x < plane.width() && y < plane.height() && plane.get(x, y)
}

/// Dark run lengths walking inwards from each side; the lower quartile is
/// the module size.
fn module_estimate(plane: Plane<'_>, q: &[Point; 4], sc: &mut SampleScratch) -> Option<f32> {
    sc.mods.clear();
    let center = Point::new(
        (q[0].x + q[1].x + q[2].x + q[3].x) / 4.0,
        (q[0].y + q[1].y + q[2].y + q[3].y) / 4.0,
    );
    let min_side = (0..4)
        .map(|k| dist(q[k], q[(k + 1) % 4]))
        .fold(f32::MAX, f32::min);
    let limit = 0.25 * min_side;
    for k in 0..4 {
        let (a, b) = (q[k], q[(k + 1) % 4]);
        let len = dist(a, b).max(1e-3);
        let mut n = ((b.y - a.y) / len, -(b.x - a.x) / len);
        if n.0 * (center.x - a.x) + n.1 * (center.y - a.y) < 0.0 {
            n = (-n.0, -n.1);
        }
        for i in 0..20 {
            let t = 0.1 + 0.8 * (i as f32 + 0.5) / 20.0;
            let base = Point::new(a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t);
            let at = |s: f32| Point::new(base.x + n.0 * s, base.y + n.1 * s);
            if !plane_at(plane, at(0.9)) {
                continue;
            }
            let mut s = 0.9;
            let mut step = 0.25;
            while s < limit && plane_at(plane, at(s + step)) {
                s += step;
                step = step.max(0.05 * s);
            }
            sc.mods.push(s + step);
        }
    }
    if sc.mods.len() < 6 {
        return None;
    }
    sc.mods.sort_by(f32::total_cmp);
    sc.mods.retain(|&r| r >= 2.0);
    if sc.mods.len() < 6 {
        return None;
    }
    Some(sc.mods[sc.mods.len() / 5])
}

fn unit_corner(k: usize) -> (f32, f32) {
    [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)][k % 4]
}

/// Point at parameter `t` along the side from corner `f` to corner `g` of
/// the unit square, moved inwards by `eps` of the perpendicular extent.
fn side_point(h: &Perspective, f: usize, g: usize, t: f32, eps: f32) -> Option<Point> {
    let (a, b) = (unit_corner(f), unit_corner(g));
    let (mut u, mut v) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
    if (a.1 - b.1).abs() < 1e-6 {
        v = if a.1 < 0.5 { eps } else { 1.0 - eps };
    } else {
        u = if a.0 < 0.5 { eps } else { 1.0 - eps };
    }
    h.map(Point::new(u, v))
}

fn dark_fraction(plane: Plane<'_>, h: &Perspective, k: usize, eps: f32, n: usize) -> f32 {
    let mut dark = 0;
    let mut total = 0;
    for i in 0..n {
        let t = 0.05 + 0.9 * (i as f32 + 0.5) / n as f32;
        if let Some(p) = side_point(h, k, k + 1, t, eps) {
            total += 1;
            dark += usize::from(plane_at(plane, p));
        }
    }
    if total == 0 {
        0.0
    } else {
        dark as f32 / total as f32
    }
}

/// Run-length encodes the samples along a clock track, merges runs that are
/// too short to be modules and returns (run count, regular share, starts dark).
fn clock_runs(
    plane: Plane<'_>,
    h: &Perspective,
    f: usize,
    g: usize,
    eps: f32,
    samples: usize,
    sc: &mut SampleScratch,
) -> Option<(usize, f32, bool)> {
    sc.runs.clear();
    sc.samples.clear();
    for i in 0..samples {
        let t = (i as f32 + 0.5) / samples as f32;
        let p = side_point(h, f, g, t, eps)?;
        let v = plane_at(plane, p);
        sc.samples.push(v);
        match sc.runs.last_mut() {
            Some(r) if r.0 == v => r.1 += 1,
            _ => sc.runs.push((v, 1)),
        }
    }
    loop {
        if sc.runs.len() < 3 {
            return None;
        }
        // Dark and light modules differ in width after binarization, so each
        // color has its own median.
        let med = [false, true].map(|color| {
            sc.lens.clear();
            sc.lens
                .extend(sc.runs.iter().filter(|r| r.0 == color).map(|r| r.1));
            sc.lens.sort_unstable();
            sc.lens.get(sc.lens.len() / 2).copied().unwrap_or(0) as f32
        });
        let Some(i) = sc
            .runs
            .iter()
            .position(|r| (r.1 as f32) < (0.4 * med[usize::from(r.0)]).max(1.5))
        else {
            break;
        };
        let last = sc.runs.len() - 1;
        if i == 0 {
            let l = sc.runs[0].1;
            sc.runs[1].1 += l;
            sc.runs.remove(0);
        } else if i == last {
            let l = sc.runs[i].1;
            sc.runs[i - 1].1 += l;
            sc.runs.pop();
        } else {
            let l = sc.runs[i].1 + sc.runs[i + 1].1;
            sc.runs[i - 1].1 += l;
            sc.runs.drain(i..i + 2);
        }
    }
    let med = [false, true].map(|color| {
        sc.lens.clear();
        sc.lens
            .extend(sc.runs.iter().filter(|r| r.0 == color).map(|r| r.1));
        sc.lens.sort_unstable();
        sc.lens.get(sc.lens.len() / 2).copied().unwrap_or(1) as f32
    });
    let regular = sc
        .runs
        .iter()
        .filter(|r| {
            let m = med[usize::from(r.0)];
            (0.55 * m..=1.8 * m).contains(&(r.1 as f32))
        })
        .count();
    Some((
        sc.runs.len(),
        regular as f32 / sc.runs.len() as f32,
        sc.runs[0].0,
    ))
}

/// Re-fits the four side lines of a quad to the boundary pixels: for many
/// positions along a side the first dark pixel walking inwards gives an offset,
/// and a RANSAC line through those offsets (the tips of the clock modules or
/// the solid L) replaces the hull-derived side. Returns the input when the
/// fit is not trustworthy.
fn refine_quad(
    plane: Plane<'_>,
    q: &[Point; 4],
    module: f32,
    sc: &mut SampleScratch,
) -> [Point; 4] {
    let center = Point::new(
        (q[0].x + q[1].x + q[2].x + q[3].x) / 4.0,
        (q[0].y + q[1].y + q[2].y + q[3].y) / 4.0,
    );
    let reach_in = (0.8 * module).max(3.0);
    let reach_out = (0.25 * module).max(2.0);
    let tol = (0.2 * module).max(0.7);
    let mut lines = [((0.0f32, 0.0f32), (0.0f32, 0.0f32)); 4];
    let mut rng = 0x2545_F491_4F6C_DD1Du64;
    for k in 0..4 {
        let (a, b) = (q[k], q[(k + 1) % 4]);
        let len = dist(a, b).max(1.0);
        let dir = ((b.x - a.x) / len, (b.y - a.y) / len);
        let mut n = (dir.1, -dir.0);
        if n.0 * (center.x - a.x) + n.1 * (center.y - a.y) < 0.0 {
            n = (-n.0, -n.1);
        }
        let count = ((len / 1.5) as usize).clamp(12, 128);
        sc.edge.clear();
        for i in 0..count {
            let x = len * (i as f32 + 0.5) / count as f32;
            let base = (a.x + dir.0 * x, a.y + dir.1 * x);
            let mut s = -reach_out;
            while s <= reach_in {
                if plane_at(plane, Point::new(base.0 + n.0 * s, base.1 + n.1 * s)) {
                    sc.edge.push((x, s));
                    break;
                }
                s += 0.5;
            }
        }
        if sc.edge.len() < 8 {
            return *q;
        }
        // RANSAC over point pairs far apart along the side.
        let mut best: Option<(usize, f32, f32, f32)> = None;
        for _ in 0..64 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let i = (rng % sc.edge.len() as u64) as usize;
            let j = ((rng >> 24) % sc.edge.len() as u64) as usize;
            let (p, r) = (sc.edge[i], sc.edge[j]);
            if (p.0 - r.0).abs() < 0.3 * len {
                continue;
            }
            let g = (r.1 - p.1) / (r.0 - p.0);
            let c = p.1 - g * p.0;
            let (mut n_in, mut err) = (0usize, 0.0f32);
            for &(x, o) in &sc.edge {
                let e = (o - (c + g * x)).abs();
                if e <= tol {
                    n_in += 1;
                    err += e;
                }
            }
            if best.is_none_or(|b| n_in > b.0 || (n_in == b.0 && err < b.3)) {
                best = Some((n_in, c, g, err));
            }
        }
        let Some((n_in, mut c, mut g, _)) = best else {
            return *q;
        };
        if n_in * 10 < sc.edge.len() * 4 || n_in < 6 {
            return *q;
        }
        for _ in 0..2 {
            sc.inliers.clear();
            sc.inliers.extend(
                sc.edge
                    .iter()
                    .copied()
                    .filter(|&(x, o)| (o - (c + g * x)).abs() <= tol),
            );
            let n = sc.inliers.len() as f32;
            if n < 6.0 {
                break;
            }
            let (mx, mo) = sc
                .inliers
                .iter()
                .fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
            let (mx, mo) = (mx / n, mo / n);
            let (mut sxx, mut sxo) = (0.0, 0.0);
            for p in &sc.inliers {
                sxx += (p.0 - mx) * (p.0 - mx);
                sxo += (p.0 - mx) * (p.1 - mo);
            }
            if sxx > 1e-6 {
                g = sxo / sxx;
                c = mo - g * mx;
            }
        }
        let p0 = (a.x + n.0 * c, a.y + n.1 * c);
        let d = (dir.0 + n.0 * g, dir.1 + n.1 * g);
        let dl = d.0.hypot(d.1);
        lines[k] = (p0, (d.0 / dl, d.1 / dl));
    }
    let mut out = *q;
    for k in 0..4 {
        let (l0, l1) = (lines[(k + 3) % 4], lines[k]);
        let det = l0.1.0 * l1.1.1 - l0.1.1 * l1.1.0;
        if det.abs() < 1e-3 {
            return *q;
        }
        let t = ((l1.0.0 - l0.0.0) * l1.1.1 - (l1.0.1 - l0.0.1) * l1.1.0) / det;
        let p = Point::new(l0.0.0 + l0.1.0 * t, l0.0.1 + l0.1.1 * t);
        if !p.x.is_finite() || !p.y.is_finite() || p.distance(q[k]) > 2.5 * module + 2.0 {
            return *q;
        }
        out[k] = p;
    }
    out
}

/// Finds the L corner, the reading orientation and the module counts of the
/// clock tracks for a quad whose corners run clockwise on screen.
/// Insets in pixels at which a side is probed for solidity.
const INSETS: [f32; 8] = [1.2, 2.5, 4.0, 6.0, 9.0, 14.0, 22.0, 34.0];

/// Largest dark fraction along the side over several insets below one tenth
/// of the perpendicular extent (stops early at `enough`), and the largest
/// over the first two insets only.
fn solid_any(
    plane: Plane<'_>,
    h: &Perspective,
    k: usize,
    len: f32,
    perp: f32,
    enough: f32,
    coarse: bool,
) -> (f32, f32) {
    let n = if coarse {
        ((len / 1.5).ceil() as usize).clamp(24, 96)
    } else {
        ((len / 0.75).ceil() as usize).clamp(24, 256)
    };
    let mut best = 0.0f32;
    let mut small = 0.0f32;
    for inset in INSETS {
        // A module is at most one eighth of the side and of its perpendicular extent.
        if inset > (0.1 * perp).min(0.9 * len / 8.0) && inset > INSETS[0] {
            break;
        }
        let f = dark_fraction(plane, h, k, (inset / perp).clamp(0.002, 0.4), n);
        best = best.max(f);
        if inset <= INSETS[1] {
            small = small.max(f);
        }
        if best >= enough {
            break;
        }
    }
    (best, small)
}

/// Side lengths, inset fractions and solid fractions of the four sides.
struct Sides {
    h: Perspective,
    len: [f32; 4],
    /// Inset fractions along u and v.
    eps: [f32; 2],
    /// Largest dark fraction per side over the insets.
    solid: [f32; 4],
    /// Largest dark fraction per side over the two innermost-probed insets.
    small: [f32; 4],
}

fn side_solidity(
    plane: Plane<'_>,
    quad: &[Point; 4],
    m: f32,
    enough: f32,
    coarse: bool,
) -> Option<Sides> {
    let h = Perspective::square_to_quad(*quad)?;
    let len = [0, 1, 2, 3].map(|k| dist(quad[k], quad[(k + 1) % 4]));
    // Sides 0 and 2 run along u, sides 1 and 3 along v.
    let perp_u = (len[1] + len[3]) / 2.0;
    let perp_v = (len[0] + len[2]) / 2.0;
    let eps_u = (0.5 * m / perp_u).clamp(0.002, 0.3);
    let eps_v = (0.5 * m / perp_v).clamp(0.002, 0.3);
    let both = [0, 1, 2, 3].map(|k| {
        let perp = if k % 2 == 0 { perp_u } else { perp_v };
        solid_any(plane, &h, k, len[k], perp, enough, coarse)
    });
    Some(Sides {
        h,
        len,
        eps: [eps_u, eps_v],
        solid: both.map(|b| b.0),
        small: both.map(|b| b.1),
    })
}

/// The L finder corner: the corner whose two sides are solid while the other
/// two are not.
fn l_corner(solid: &[f32; 4], small: &[f32; 4], solid_min: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for c in 0..4 {
        let s = solid[(c + 3) % 4].min(solid[c]);
        let others = small[(c + 1) % 4].max(small[(c + 2) % 4]);
        if s >= solid_min && others < CLOCK_SOLID_MAX && best.is_none_or(|b| s > b.1) {
            best = Some((c, s));
        }
    }
    best.map(|b| b.0)
}

/// `solid` and `small` of the four sides as `side_solidity` finds them for
/// the precheck, probing the sides lazily: an L corner has one side along u
/// and one along v that are solid, so when both sides along u (0 and 2) are
/// not, no corner can pass and the other two are not probed. `None` then, as
/// `l_corner` would answer.
fn precheck_sides(plane: Plane<'_>, quad: &[Point; 4]) -> Option<([f32; 4], [f32; 4])> {
    let h = Perspective::square_to_quad(*quad)?;
    let len = [0, 1, 2, 3].map(|k| dist(quad[k], quad[(k + 1) % 4]));
    let perp_u = (len[1] + len[3]) / 2.0;
    let perp_v = (len[0] + len[2]) / 2.0;
    let mut solid = [0.0f32; 4];
    let mut small = [0.0f32; 4];
    for k in [0, 2, 1, 3] {
        if k == 1 && solid[0].max(solid[2]) < PRECHECK_SOLID_MIN {
            return None;
        }
        let perp = if k % 2 == 0 { perp_u } else { perp_v };
        (solid[k], small[k]) = solid_any(plane, &h, k, len[k], perp, PRECHECK_SOLID_MIN, true);
    }
    Some((solid, small))
}

/// Cheap test on the raw quad: a module size estimate and two solid sides
/// meeting at a corner. Returns the module size.
pub(crate) fn precheck(plane: Plane<'_>, quad: &[Point; 4], sc: &mut SampleScratch) -> Option<f32> {
    // The solid L test is the cheapest and rejects most quads; it does not
    // depend on the module size.
    let (solid, small) = precheck_sides(plane, quad)?;
    l_corner(&solid, &small, PRECHECK_SOLID_MIN)?;
    let m = module_estimate(plane, quad, sc)?;
    let sides = [0, 1, 2, 3].map(|k| dist(quad[k], quad[(k + 1) % 4]));
    let (lo, hi) = sides
        .iter()
        .fold((f32::MAX, 0.0f32), |a, &s| (a.0.min(s), a.1.max(s)));
    // A symbol is at least 8 and at most 144 modules long on each side.
    if lo < 5.0 * m || hi > 300.0 * m {
        return None;
    }
    Some(m)
}

pub(crate) fn analyze(
    plane: Plane<'_>,
    quad: &[Point; 4],
    m: f32,
    sc: &mut SampleScratch,
) -> Option<Layout> {
    let quad = &refine_quad(plane, quad, m, sc);

    let Sides {
        h,
        len,
        eps: [eps_u, eps_v],
        solid,
        small,
    } = side_solidity(plane, quad, m, 2.0, false)?;
    let eps_of = |k: usize| if k.is_multiple_of(2) { eps_u } else { eps_v };
    let c = l_corner(&solid, &small, SOLID_MIN)?;
    let (ia, ib, id) = ((c + 3) % 4, (c + 1) % 4, (c + 2) % 4);
    let (pa, pb, pc) = (quad[ia], quad[ib], quad[c]);
    let cr = (pa.x - pc.x) * (pb.y - pc.y) - (pa.y - pc.y) * (pb.x - pc.x);
    let (itl, ibr) = if cr > 0.0 { (ia, ib) } else { (ib, ia) };
    let corners = [quad[itl], quad[id], quad[ibr], quad[c]];
    let samples = |k: usize| ((5.0 * len[k] / m).ceil() as usize).clamp(64, 1536);
    // Top track: TL -> TR; right track: BR -> TR.
    let side_of = |f: usize, g: usize| if (f + 1) % 4 == g { f } else { g };
    // Bold prints fill the gaps of the clock tracks below their tips, so a
    // shallower probe is tried when the first one finds no regular modules.
    let mut tracks = None;
    for scale in [1.0, 0.45] {
        let top = clock_runs(
            plane,
            &h,
            itl,
            id,
            eps_of(side_of(itl, id)) * scale,
            samples(side_of(itl, id)),
            sc,
        );
        std::mem::swap(&mut sc.samples, &mut sc.top_samples);
        let right = clock_runs(
            plane,
            &h,
            ibr,
            id,
            eps_of(side_of(ibr, id)) * scale,
            samples(side_of(ibr, id)),
            sc,
        );
        std::mem::swap(&mut sc.samples, &mut sc.right_samples);
        if let (Some(top), Some(right)) = (top, right)
            && top.1 >= REGULAR_MIN
            && right.1 >= REGULAR_MIN
            && top.0 >= 8
            && right.0 >= 6
        {
            tracks = Some((top, right));
            break;
        }
    }
    let (top, right) = tracks?;
    Some(Layout {
        corners,
        rows_est: right.0,
        cols_est: top.0,
        rows_geo: dist(corners[1], corners[2]) / m,
        cols_geo: dist(corners[0], corners[1]) / m,
    })
}

/// A size hypothesis: table entry and whether the sampled grid is the
/// reflected (mirrored) reading, which swaps its rows and columns.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Hypothesis {
    pub size: &'static SymbolSize,
    pub swapped: bool,
}

impl Hypothesis {
    /// Sampled grid rows and columns.
    pub fn grid_dims(&self) -> (usize, usize) {
        let (r, c) = (usize::from(self.size.rows), usize::from(self.size.cols));
        if self.swapped { (c, r) } else { (r, c) }
    }

    /// Region counts and sizes of the sampled grid as
    /// (regions_v, regions_h, region_rows, region_cols).
    pub fn regions(&self) -> (usize, usize, usize, usize) {
        let s = self.size;
        let (rv, rh) = (usize::from(s.regions.0), usize::from(s.regions.1));
        let (rr, rc) = (usize::from(s.region_size.0), usize::from(s.region_size.1));
        if self.swapped {
            (rh, rv, rc, rr)
        } else {
            (rv, rh, rr, rc)
        }
    }
}

/// Up to `out.len()` size hypotheses closest to the counted modules, best first.
/// Size hypotheses of a layout: the counted modules and the size implied by
/// the side lengths (clock runs merge on bold prints and split on blurred
/// ones, the geometry does not).
pub(crate) fn snap_layout(
    layout: &Layout,
    sc: &SampleScratch,
    out: &mut [Option<Hypothesis>],
    weak: bool,
) {
    let mut counted = [None; 4];
    snap_estimates(
        &[
            (layout.rows_est as f32, layout.cols_est as f32),
            (layout.rows_geo, layout.cols_geo),
        ],
        &mut counted,
    );
    let mut tracks = [None; 4];
    snap_tracks(sc, weak, &mut tracks);
    // Interleave the two rankings, dropping repeats.
    let same =
        |a: &Hypothesis, b: &Hypothesis| std::ptr::eq(a.size, b.size) && a.swapped == b.swapped;
    out.iter_mut().for_each(|o| *o = None);
    let mut n = 0;
    for i in 0..4 {
        for cand in [tracks[i], counted[i]].into_iter().flatten() {
            if n < out.len() && !out[..n].iter().flatten().any(|h| same(h, &cand)) {
                out[n] = Some(cand);
                n += 1;
            }
        }
    }
}

/// Share of clock samples that agree with `n` alternating modules starting dark.
fn track_agreement(samples: &[bool], n: usize) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let total = samples.len();
    let agree = samples
        .iter()
        .enumerate()
        .filter(|&(i, &v)| v == ((i * n / total).is_multiple_of(2)))
        .count();
    agree as f32 / total as f32
}

/// Sizes ranked by how well the sampled clock tracks match their module
/// count (robust against runs that merge on bold prints and split on blur).
/// Both tracks must agree with at least `TRACK_AGREE_MIN`.
fn snap_tracks(sc: &SampleScratch, weak_ok: bool, out: &mut [Option<Hypothesis>]) {
    const TRACK_AGREE_MIN: f32 = 0.72;
    const TRACK_STRONG: f32 = 0.8;
    let mut best: [(f32, Option<Hypothesis>); 4] = [(0.0, None); 4];
    // Sizes with one track in strong agreement and the other not (the far
    // end of the other track is lost to blur or perspective), best first;
    // they only fill the slots the two-track matches leave empty.
    let mut weak: [(f32, Option<Hypothesis>); 4] = [(0.0, None); 4];
    let mut cache: [(u16, f32, f32); 40] = [(0, 0.0, 0.0); 40];
    let mut used = 0;
    let mut scores = |n: u16| -> (f32, f32) {
        if let Some(c) = cache[..used].iter().find(|c| c.0 == n) {
            return (c.1, c.2);
        }
        let r = (
            track_agreement(&sc.right_samples, usize::from(n)),
            track_agreement(&sc.top_samples, usize::from(n)),
        );
        if used < cache.len() {
            cache[used] = (n, r.0, r.1);
            used += 1;
        }
        r
    };
    for size in spec::sizes() {
        for swapped in [false, true] {
            if swapped && size.rows == size.cols {
                continue;
            }
            let hyp = Hypothesis { size, swapped };
            let (gr, gc) = hyp.grid_dims();
            let (rows_score, _) = scores(gr as u16);
            let (_, cols_score) = scores(gc as u16);
            let both = rows_score >= TRACK_AGREE_MIN && cols_score >= TRACK_AGREE_MIN;
            let one = weak_ok && rows_score.max(cols_score) >= TRACK_STRONG;
            if !both && !one {
                continue;
            }
            // Squares are by far the most common sizes: prefer them.
            let bonus = if size.rows == size.cols { 0.05 } else { 0.0 };
            let (list, score) = if both {
                (&mut best, rows_score + cols_score)
            } else {
                (&mut weak, rows_score.max(cols_score) + bonus)
            };
            let mut item = (score, Some(hyp));
            for slot in list.iter_mut() {
                if item.0 > slot.0 {
                    std::mem::swap(slot, &mut item);
                }
            }
        }
    }
    let mut spare = weak.iter().filter_map(|w| w.1);
    for b in best.iter_mut() {
        if b.1.is_none() {
            b.1 = spare.next();
        }
    }
    for (o, b) in out.iter_mut().zip(best) {
        *o = b.1;
    }
}

/// Up to `out.len()` sizes whose best cost over the `(rows, cols)` estimates
/// is acceptable, cheapest first.
fn snap_estimates(ests: &[(f32, f32)], out: &mut [Option<Hypothesis>]) {
    out.iter_mut().for_each(|o| *o = None);
    let mut best: [(f32, Option<Hypothesis>); 4] = [(f32::MAX, None); 4];
    let cost = |er: f32, ec: f32, r: f32, c: f32| (er - r).abs() / r + (ec - c).abs() / c;
    for size in spec::sizes() {
        let (r, c) = (f32::from(size.rows), f32::from(size.cols));
        for swapped in [false, true] {
            if swapped && size.rows == size.cols {
                continue;
            }
            let k = ests
                .iter()
                .map(|&(er, ec)| {
                    if swapped {
                        cost(er, ec, c, r)
                    } else {
                        cost(er, ec, r, c)
                    }
                })
                .fold(f32::MAX, f32::min);
            if k > SNAP_MAX_COST {
                continue;
            }
            // Insert into the sorted short list.
            let mut item = (k, Some(Hypothesis { size, swapped }));
            for slot in best.iter_mut() {
                if item.0 < slot.0 {
                    std::mem::swap(slot, &mut item);
                }
            }
        }
    }
    for (o, b) in out.iter_mut().zip(best) {
        *o = b.1;
    }
}

/// Finds the pixel position of the clock transition expected at module
/// coordinates `at` along `axis`, where the module before is `dark_before`.
fn locate(
    plane: Plane<'_>,
    h: &Mapper,
    at: (f64, f64),
    axis: (f64, f64),
    dark_before: bool,
    step: f64,
) -> Option<(f64, f64, f64)> {
    let mut best: Option<f64> = None;
    let mut prev: Option<(f64, bool)> = None;
    let mut s = -0.5;
    while s <= 0.5 + 1e-9 {
        let (x, y) = h.map(at.0 + axis.0 * s, at.1 + axis.1 * s)?;
        let v = plane_at(plane, Point::new(x as f32, y as f32));
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
    let s = best?;
    if s.abs() > 0.42 {
        return None;
    }
    let (x, y) = h.map(at.0 + axis.0 * s, at.1 + axis.1 * s)?;
    Some((x, y, s))
}

/// Refines the corner homography with the clock transitions of the outer
/// clock tracks and of the interior alternating lines of multi-region symbols.
pub(crate) fn refine(
    plane: Plane<'_>,
    corners: &[Point; 4],
    hyp: &Hypothesis,
    sc: &mut SampleScratch,
) -> Option<Mapper> {
    let (rows, cols) = hyp.grid_dims();
    let (rv, rh, rr, rc) = hyp.regions();
    let mut h = Mapper::from_corners(corners, rows, cols)?;
    let module = f64::from(h.module_px(rows, cols));
    if module < 2.0 {
        return Some(h);
    }
    let step = (0.2 / module).clamp(0.02, 0.1);
    let corner_pairs = |sc: &mut SampleScratch, w: f64| {
        let (cw, ch) = (cols as f64, rows as f64);
        let uv = [(0.0, 0.0), (cw, 0.0), (cw, ch), (0.0, ch)];
        for (&(u, v), p) in uv.iter().zip(corners) {
            sc.pairs.push((u, v, f64::from(p.x), f64::from(p.y), w));
        }
    };
    for _ in 0..2 {
        sc.pairs.clear();
        sc.transitions.clear();
        // Top row and interior alternating rows: transitions between columns.
        for i in 0..rv {
            let y = (i * (rr + 2)) as f64 + 0.5;
            for c in 1..cols {
                let dark_before = (c - 1) % 2 == 0;
                if let Some((x, py, _)) =
                    locate(plane, &h, (c as f64, y), (1.0, 0.0), dark_before, step)
                {
                    sc.transitions.push((c as f64, y, x, py));
                }
            }
        }
        // Right column and interior alternating columns: transitions between rows.
        for j in 0..rh {
            let x = ((j + 1) * (rc + 2)) as f64 - 0.5;
            for r in 1..rows {
                let dark_before = (rows - r) % 2 == 0;
                if let Some((px, py, _)) =
                    locate(plane, &h, (x, r as f64), (0.0, 1.0), dark_before, step)
                {
                    sc.transitions.push((x, r as f64, px, py));
                }
            }
        }
        let n = sc.transitions.len();
        if n < 8 {
            return Some(h);
        }
        let wc = (n as f64 / 8.0).clamp(3.0, 20.0);
        corner_pairs(sc, wc);
        for &(u, v, x, y) in &sc.transitions {
            sc.pairs.push((u, v, x, y, 1.0));
        }
        let Some(mut fit) = Mapper::fit(&sc.pairs) else {
            return Some(h);
        };
        // Drop outliers once and refit.
        let limit = (0.35 * module).max(0.8);
        let before = sc.pairs.len();
        sc.pairs.retain(|&(u, v, x, y, w)| {
            w != 1.0
                || fit
                    .map(u, v)
                    .is_some_and(|(fx, fy)| (fx - x).hypot(fy - y) <= limit)
        });
        if sc.pairs.len() < before
            && let Some(f2) = Mapper::fit(&sc.pairs)
        {
            fit = f2;
        }
        // The refit must keep the corners near the detected quad.
        let ok = [
            (0.0, 0.0),
            (cols as f64, 0.0),
            (cols as f64, rows as f64),
            (0.0, rows as f64),
        ]
        .iter()
        .zip(corners)
        .all(|(&(u, v), p)| {
            fit.map(u, v).is_some_and(|(x, y)| {
                (x - f64::from(p.x)).hypot(y - f64::from(p.y)) <= 0.75 * module + 0.5
            })
        });
        if !ok {
            return Some(h);
        }
        h = fit;
    }
    Some(h)
}

/// Module centres of the two clock tracks in the unit square of a symbol of
/// `rows` x `cols` modules, with the colour each must have: top row dark at
/// even columns, right column dark at even distance from the bottom.
fn outer_pattern(rows: usize, cols: usize, out: &mut Vec<(f32, f32, bool)>) {
    out.clear();
    let (w, h) = (cols as f32, rows as f32);
    for c in 0..cols {
        out.push(((c as f32 + 0.5) / w, 0.5 / h, c % 2 == 0));
    }
    for r in 0..rows {
        out.push((
            (w - 0.5) / w,
            (r as f32 + 0.5) / h,
            (rows - 1 - r).is_multiple_of(2),
        ));
    }
}

/// Wrong modules among the clock points read through `h`, counting stops
/// as soon as it exceeds `limit`; `u32::MAX` when a point cannot be mapped.
fn outer_cost(plane: Plane<'_>, h: &Perspective, pattern: &[(f32, f32, bool)], limit: u32) -> u32 {
    let mut bad = 0u32;
    for &(u, v, dark) in pattern {
        let Some(p) = h.map(Point::new(u, v)) else {
            return u32::MAX;
        };
        bad += u32::from(plane_at(plane, p) != dark);
        if bad > limit {
            break;
        }
    }
    bad
}

/// Moves the three free corners of the quad (the bottom-left corner of the
/// L stays) to the positions where the two clock tracks match best, in two
/// passes: a window of one module in steps of half a module, then half a
/// module in quarter steps (the size is dropped when more than 30 % of the
/// track modules are wrong after the first). Returns the corners and the
/// number of wrong clock modules. A window of two modules (steps of one)
/// reads a few more symbols and admitted a false positive in the
/// skewed-datamatrix set.
pub(crate) fn corner_search(
    plane: Plane<'_>,
    corners: &[Point; 4],
    hyp: &Hypothesis,
    sc: &mut SampleScratch,
) -> Option<([Point; 4], u32)> {
    let (rows, cols) = hyp.grid_dims();
    let module = (dist(corners[0], corners[1]) / cols as f32
        + dist(corners[0], corners[3]) / rows as f32)
        / 2.0;
    if module < MIN_SEARCH_MODULE {
        return None;
    }
    outer_pattern(rows, cols, &mut sc.pattern);
    let pattern = &sc.pattern;
    let mut best = *corners;
    let mut best_cost = outer_cost(
        plane,
        &Perspective::square_to_quad(best)?,
        pattern,
        u32::MAX - 1,
    );
    let len = pattern.len() as u32;
    for (win, step) in [(2i32, 0.5f32), (2, 0.25)] {
        for k in [1usize, 2, 0] {
            let base = best[k];
            let mut pick = (best_cost, 0, base);
            for iy in -win..=win {
                for ix in -win..=win {
                    let p = Point::new(
                        base.x + ix as f32 * step * module,
                        base.y + iy as f32 * step * module,
                    );
                    let mut c = best;
                    c[k] = p;
                    let Some(h) = Perspective::square_to_quad(c) else {
                        continue;
                    };
                    // Ties go to the smallest move.
                    let v = outer_cost(plane, &h, pattern, pick.0);
                    let d = ix * ix + iy * iy;
                    if v < pick.0 || (v == pick.0 && d < pick.1) {
                        pick = (v, d, p);
                    }
                }
            }
            best_cost = pick.0;
            best[k] = pick.2;
        }
        if best_cost * 10 > len * 3 {
            return None;
        }
    }
    Some((best, best_cost))
}

/// Sizes for which the outer patterns fit best after a corner search, for
/// quads whose own size estimate is unreliable: `(share of wrong modules,
/// hypothesis, corners)`, best first.
pub(crate) fn broad_hypotheses(
    plane: Plane<'_>,
    layout: &Layout,
    sc: &mut SampleScratch,
    out: &mut Vec<(f32, Hypothesis, [Point; 4])>,
) {
    out.clear();
    let corners = &layout.corners;
    let lw = dist(corners[0], corners[1]);
    let lh = dist(corners[0], corners[3]);
    for size in spec::sizes() {
        for swapped in [false, true] {
            if swapped && size.rows == size.cols {
                continue;
            }
            let hyp = Hypothesis { size, swapped };
            let (r, c) = hyp.grid_dims();
            let (pw, ph) = (lw / c as f32, lh / r as f32);
            if pw < MIN_SEARCH_MODULE || ph < MIN_SEARCH_MODULE || pw > 2.0 * ph || ph > 2.0 * pw {
                continue;
            }
            // The counted clock runs are off by blur and perspective, but not
            // by more than this factor.
            let off = |n: usize, est: usize| {
                let q = n as f32 / est as f32;
                !(1.0 / EST_FACTOR..=EST_FACTOR).contains(&q)
            };
            if off(r, layout.rows_est) || off(c, layout.cols_est) {
                continue;
            }
            let Some((c2, cost)) = corner_search(plane, corners, &hyp, sc) else {
                continue;
            };
            let share = cost as f32 / (r + c) as f32;
            if share <= 0.2 {
                out.push((share, hyp, c2));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
}

/// Per-boundary corrections of the module coordinates along each axis,
/// measured on the clock tracks: the module pitch of a real print is not
/// perfectly uniform, and a homography cannot follow that.
#[derive(Debug, Clone, Default)]
pub(crate) struct Warp {
    /// Offset in modules of the true column boundary `c` (index `c`, 0..=cols).
    du: Vec<f32>,
    /// Same for row boundaries.
    dv: Vec<f32>,
}

impl Warp {
    fn at(d: &[f32], t: f64) -> f64 {
        let last = d.len() - 1;
        let t = t.clamp(0.0, last as f64);
        let i = (t.floor() as usize).min(last - 1);
        let f = t - i as f64;
        f64::from(d[i]) * (1.0 - f) + f64::from(d[i + 1]) * f
    }

    fn apply(&self, u: f64, v: f64) -> (f64, f64) {
        (u + Self::at(&self.du, u), v + Self::at(&self.dv, v))
    }
}

/// Measures the clock transition offsets of `h` on every alternating line
/// (top rows of all regions, right columns of all regions) and returns the
/// per-boundary corrections. `None` when too few transitions are found.
pub(crate) fn clock_warp(plane: Plane<'_>, h: &Mapper, hyp: &Hypothesis) -> Option<Warp> {
    let (rows, cols) = hyp.grid_dims();
    let (rv, rh, rr, rc) = hyp.regions();
    let module = f64::from(h.module_px(rows, cols));
    if module < 2.0 {
        return None;
    }
    let step = (0.2 / module).clamp(0.02, 0.1);
    let mut du = vec![(0.0f64, 0u32); cols + 1];
    let mut dv = vec![(0.0f64, 0u32); rows + 1];
    for i in 0..rv {
        let y = (i * (rr + 2)) as f64 + 0.5;
        for (c, acc) in du.iter_mut().enumerate().take(cols).skip(1) {
            let dark_before = (c - 1) % 2 == 0;
            if let Some((_, _, s)) = locate(plane, h, (c as f64, y), (1.0, 0.0), dark_before, step)
            {
                acc.0 += s;
                acc.1 += 1;
            }
        }
    }
    for j in 0..rh {
        let x = ((j + 1) * (rc + 2)) as f64 - 0.5;
        for (r, acc) in dv.iter_mut().enumerate().take(rows).skip(1) {
            let dark_before = (rows - r) % 2 == 0;
            if let Some((_, _, s)) = locate(plane, h, (x, r as f64), (0.0, 1.0), dark_before, step)
            {
                acc.0 += s;
                acc.1 += 1;
            }
        }
    }
    let finish = |acc: &[(f64, u32)]| -> Option<Vec<f32>> {
        let n = acc.len() - 1;
        let known: Vec<(usize, f64)> = (1..n)
            .filter(|&i| acc[i].1 > 0)
            .map(|i| (i, acc[i].0 / f64::from(acc[i].1)))
            .collect();
        if known.len() * 2 < n.saturating_sub(1) {
            return None;
        }
        let mut out = vec![0.0f32; n + 1];
        for (i, o) in out.iter_mut().enumerate() {
            let below = known.iter().rev().find(|k| k.0 <= i);
            let above = known.iter().find(|k| k.0 >= i);
            *o = match (below, above) {
                (Some(b), Some(a)) if a.0 != b.0 => {
                    let f = (i - b.0) as f64 / (a.0 - b.0) as f64;
                    (b.1 * (1.0 - f) + a.1 * f) as f32
                }
                (Some(b), _) => b.1 as f32,
                (_, Some(a)) => a.1 as f32,
                _ => 0.0,
            };
        }
        Some(out)
    };
    Some(Warp {
        du: finish(&du)?,
        dv: finish(&dv)?,
    })
}

/// Module-coordinate positions of the `n - 1` boundaries of an alternating
/// clock track, read along the line through `h`: `line(s)` is the module
/// coordinate pair of position `s` (0 to `n`) along the track and `first_dark`
/// whether the module at `s = 0` is dark. Runs shorter than 0.4 of the median
/// are merged away; `None` unless exactly `n - 1` boundaries remain.
fn track_boundaries(
    plane: Plane<'_>,
    h: &Mapper,
    n: usize,
    module: f64,
    first_dark: bool,
    line: impl Fn(f64) -> (f64, f64),
    runs: &mut Vec<(bool, f64, f64)>,
) -> Option<Vec<f64>> {
    let step = (0.2 / module).clamp(0.01, 0.1);
    runs.clear();
    let mut s = 0.0;
    while s < n as f64 {
        let (u, v) = line(s);
        let (x, y) = h.map(u, v)?;
        let dark = plane_at(plane, Point::new(x as f32, y as f32));
        match runs.last_mut() {
            Some(r) if r.0 == dark => r.2 = s + step,
            _ => runs.push((dark, s, s + step)),
        }
        s += step;
    }
    // Merge runs shorter than 0.4 module (noise) into their neighbours.
    while let Some(i) = runs.iter().position(|r| r.2 - r.1 < 0.4) {
        if runs.len() < 3 {
            return None;
        }
        if i == 0 {
            runs[1].1 = runs[0].1;
            runs.remove(0);
        } else if i == runs.len() - 1 {
            runs[i - 1].2 = runs[i].2;
            runs.pop();
        } else {
            runs[i - 1].2 = runs[i + 1].2;
            runs.drain(i..i + 2);
        }
    }
    if runs.len() != n || runs[0].0 != first_dark {
        return None;
    }
    Some(runs.windows(2).map(|w| (w[0].2 + w[1].1) / 2.0).collect())
}

/// Warp that places every column and row boundary where the clock tracks
/// show it, for prints whose module pitch drifts by more than a clock
/// transition can capture (a stretched or compressed axis). Needs the
/// number of transitions of the top track and the right track to match the
/// hypothesis exactly; an axis that does not match keeps its pitch.
pub(crate) fn track_warp(
    plane: Plane<'_>,
    h: &Mapper,
    hyp: &Hypothesis,
    sc: &mut SampleScratch,
) -> Option<Warp> {
    let (rows, cols) = hyp.grid_dims();
    let module = f64::from(h.module_px(rows, cols));
    if module < 2.0 {
        return None;
    }
    let top = track_boundaries(
        plane,
        h,
        cols,
        module,
        true,
        |s| (s, 0.5),
        &mut sc.track_runs,
    );
    let right = track_boundaries(
        plane,
        h,
        rows,
        module,
        rows % 2 == 1,
        |s| (cols as f64 - 0.5, s),
        &mut sc.track_runs,
    );
    if top.is_none() && right.is_none() {
        return None;
    }
    let mut du = vec![0.0f32; cols + 1];
    let mut dv = vec![0.0f32; rows + 1];
    if let Some(t) = top {
        for (i, b) in t.iter().enumerate() {
            du[i + 1] = (b - (i + 1) as f64) as f32;
        }
    }
    if let Some(r) = right {
        for (i, b) in r.iter().enumerate() {
            dv[i + 1] = (b - (i + 1) as f64) as f32;
        }
    }
    Some(Warp { du, dv })
}

/// Reads the module grid through the mapper. Returns false when too many
/// module centers fall outside the image.
pub(crate) fn sample_grid(
    plane: Plane<'_>,
    h: &Mapper,
    rows: usize,
    cols: usize,
    out: &mut BitMatrix,
) -> bool {
    sample_grid_warped(plane, h, None, rows, cols, out, false)
}

/// `sample_grid` with the module coordinates first corrected by `warp`; `strict` also refuses a grid with
/// any module centre outside the image (the retries that search the geometry never extrapolate).
pub(crate) fn sample_grid_warped(
    plane: Plane<'_>,
    h: &Mapper,
    warp: Option<&Warp>,
    rows: usize,
    cols: usize,
    out: &mut BitMatrix,
    strict: bool,
) -> bool {
    out.resize(cols, rows);
    let map = |u: f64, v: f64| {
        let (u, v) = warp.map_or((u, v), |w| w.apply(u, v));
        h.map(u, v)
    };
    let module = h.module_px(rows, cols);
    if module <= 0.0 {
        return false;
    }
    let multi = module >= 3.0;
    let mut outside = 0usize;
    for r in 0..rows {
        for c in 0..cols {
            let (u, v) = (c as f64 + 0.5, r as f64 + 0.5);
            let Some((x, y)) = map(u, v) else {
                return false;
            };
            let at = |x: f64, y: f64| plane_at(plane, Point::new(x as f32, y as f32));
            if x < 0.0 || y < 0.0 || x >= plane.width() as f64 || y >= plane.height() as f64 {
                outside += 1;
                continue;
            }
            let dark = if multi {
                let mut votes = usize::from(at(x, y));
                for (du, dv) in [(-0.25, 0.0), (0.25, 0.0), (0.0, -0.25), (0.0, 0.25)] {
                    if let Some((sx, sy)) = map(u + du, v + dv) {
                        votes += usize::from(at(sx, sy));
                    }
                }
                votes >= 3
            } else {
                at(x, y)
            };
            if dark {
                out.set(c, r, true);
            }
        }
    }
    outside * 10 <= rows * cols && (!strict || outside == 0)
}

#[cfg(test)]
mod tests;
