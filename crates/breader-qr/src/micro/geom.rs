//! Geometry of a Micro QR candidate: finder pose, timing-line analysis and
//! the affine frame that maps module coordinates to the image.

use breader_core::{Homography, Plane, Point};

/// Finder template agreement (of 81 modules) below which a candidate is dropped.
pub(crate) const TEMPLATE_MIN: i32 = 62;

/// Tolerance in modules for a timing edge around its predicted position.
const EDGE_TOL: f32 = 0.45;

fn rot90(p: Point) -> Point {
    Point::new(-p.y, p.x)
}

fn axpy(p: Point, k: f32, d: Point) -> Point {
    Point::new(p.x + k * d.x, p.y + k * d.y)
}

fn len(p: Point) -> f32 {
    p.x.hypot(p.y)
}

/// Pixel value at `p`; `None` outside the image.
pub(crate) fn dark(plane: Plane<'_>, p: Point) -> Option<bool> {
    if p.x < 0.0 || p.y < 0.0 {
        return None;
    }
    let (x, y) = (p.x as usize, p.y as usize);
    (x < plane.width() && y < plane.height()).then(|| plane.get(x, y))
}

/// Finder center and module vector `u` (pixels per module along the
/// finder's x axis; the y axis is `u` turned by 90 degrees).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pose {
    pub c: Point,
    pub u: Point,
}

fn template_score(plane: Plane<'_>, c: Point, u: Point) -> i32 {
    template_score_from(plane, c, u, i32::MIN).unwrap_or(i32::MIN)
}

/// [`template_score`], or `None` as soon as the rows still to come cannot
/// lift the agreement to `at_least`. The callers keep a score only when it
/// reaches the best one so far, so a candidate that falls short is dropped
/// without reading the rest of its 81 modules; a score that is returned is
/// the full one.
fn template_score_from(plane: Plane<'_>, c: Point, u: Point, at_least: i32) -> Option<i32> {
    let v = rot90(u);
    let mut s = 0;
    for (row, j) in (-4i32..=4).enumerate() {
        for i in -4i32..=4 {
            let r = i.abs().max(j.abs());
            let want = r <= 3 && r != 2;
            let p = Point::new(
                c.x + i as f32 * u.x + j as f32 * v.x,
                c.y + i as f32 * u.y + j as f32 * v.y,
            );
            s += i32::from((dark(plane, p) == Some(true)) == want);
        }
        if s + 9 * (8 - row as i32) < at_least {
            return None;
        }
    }
    Some(s)
}

/// Parameters (angle in degrees, module size, center) with the best
/// score; ties are averaged so a plateau yields its middle.
struct Best {
    score: i32,
    sum: [f32; 4],
    n: f32,
}

impl Best {
    fn new() -> Self {
        Self {
            score: -1,
            sum: [0.0; 4],
            n: 0.0,
        }
    }

    /// Offers the pose at `(c, u)` scored by the template; a candidate that
    /// cannot reach the best score is not scored to the end.
    fn offer(&mut self, plane: Plane<'_>, c: Point, u: Point, p: [f32; 4]) {
        if let Some(score) = template_score_from(plane, c, u, self.score) {
            self.add(score, p);
        }
    }

    fn add(&mut self, score: i32, p: [f32; 4]) {
        if score > self.score {
            *self = Self {
                score,
                sum: p,
                n: 1.0,
            };
        } else if score == self.score {
            for (a, b) in self.sum.iter_mut().zip(p) {
                *a += b;
            }
            self.n += 1.0;
        }
    }

    fn mean(&self) -> [f32; 4] {
        self.sum.map(|v| v / self.n.max(1.0))
    }
}

fn unit(deg: f32, m: f32) -> Point {
    let (s, c) = deg.to_radians().sin_cos();
    Point::new(m * c, m * s)
}

/// Fits the finder template (center, rotation, module size) around a
/// detected finder. Returns the pose and its agreement score.
pub(crate) fn fit_pose(plane: Plane<'_>, c0: Point, m0: f32) -> Option<(Pose, i32)> {
    if !(m0 > 0.5 && m0 < 200.0) {
        return None;
    }
    // Angles are scanned outward from zero; the first best one wins since
    // the plateau of a small finder wraps around at a quarter turn.
    let mut best = (-1, 0.0f32);
    for k in 0..15 {
        let a = ((k + 1) / 2) as f32 * 6.0 * if k % 2 == 1 { 1.0 } else { -1.0 };
        // Only a strictly better score replaces the best one.
        if let Some(sc) = template_score_from(plane, c0, unit(a, m0), best.0 + 1) {
            best = (sc, a);
        }
    }
    let a1 = best.1;
    let mut b = Best::new();
    for da in -5..=5 {
        for ds in [0.9f32, 0.95, 1.0, 1.05, 1.1] {
            let (a, m) = (a1 + da as f32, m0 * ds);
            b.offer(plane, c0, unit(a, m), [a, m, c0.x, c0.y]);
        }
    }
    let [a2, m2, ..] = b.mean();
    let u = unit(a2, m2);
    let v = rot90(u);
    let mut b = Best::new();
    for j in -2..=2 {
        for i in -2..=2 {
            let c = axpy(axpy(c0, i as f32 * 0.2, u), j as f32 * 0.2, v);
            b.offer(plane, c, u, [a2, m2, c.x, c.y]);
        }
    }
    let [_, _, cx, cy] = b.mean();
    let c1 = Point::new(cx, cy);
    let mut b = Best::new();
    for da in -3..=3 {
        for ds in [0.97f32, 1.0, 1.03] {
            let (a, m) = (a2 + da as f32 * 0.5, m2 * ds);
            b.offer(plane, c1, unit(a, m), [a, m, cx, cy]);
        }
    }
    let [a3, m3, ..] = b.mean();
    let u = unit(a3, m3);
    let score = template_score(plane, c1, u);
    let pose = if len(u) >= 3.0 {
        refine_by_edges(plane, Pose { c: c1, u })
    } else {
        Pose { c: c1, u }
    };
    (score >= TEMPLATE_MIN).then_some((pose, score))
}

/// Centers the pose on the outer edges of the finder and rescales it to
/// their distance. The template score is flat over a large part of a module
/// when modules span many pixels, so this decides the sub-module position.
fn refine_by_edges(plane: Plane<'_>, pose: Pose) -> Pose {
    let mut pose = pose;
    for _ in 0..2 {
        let v = rot90(pose.u);
        let m = len(pose.u);
        let mut shift = Point::default();
        let mut scale = 0.0;
        let mut axes = 0.0;
        for dir in [pose.u, v] {
            let e = Point::new(dir.x / m, dir.y / m);
            let (p1, p2) = (axpy(pose.c, 3.0, dir), axpy(pose.c, -3.0, dir));
            let ne = Point::new(-e.x, -e.y);
            let (Some(t1), Some(t2)) = (
                edge_distance(plane, p1, e, 1.0 * m),
                edge_distance(plane, p2, ne, 1.0 * m),
            ) else {
                continue;
            };
            let k = (t1 - t2) / 2.0;
            shift = axpy(shift, k, e);
            scale += (6.0 * m + t1 + t2) / (7.0 * m);
            axes += 1.0;
        }
        if axes == 0.0 {
            break;
        }
        pose = Pose {
            c: Point::new(pose.c.x + shift.x, pose.c.y + shift.y),
            u: Point::new(pose.u.x * scale / axes, pose.u.y * scale / axes),
        };
    }
    pose
}

/// Distance in pixels from `p` along `dir` (unit) until the first light
/// pixel, `None` when `p` is not dark or no edge lies within `max`.
fn edge_distance(plane: Plane<'_>, p: Point, dir: Point, max: f32) -> Option<f32> {
    if dark(plane, p) != Some(true) {
        return None;
    }
    let mut t = 0.2;
    while t <= max {
        if dark(plane, axpy(p, t, dir)) == Some(false) {
            return Some(t - 0.1);
        }
        t += 0.2;
    }
    None
}

/// Line through points: (point, unit direction), by total least squares.
fn fit_line(pts: &[Point]) -> Option<(Point, Point)> {
    if pts.len() < 4 {
        return None;
    }
    let n = pts.len() as f32;
    let (cx, cy) = (
        pts.iter().map(|p| p.x).sum::<f32>() / n,
        pts.iter().map(|p| p.y).sum::<f32>() / n,
    );
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for p in pts {
        let (dx, dy) = (p.x - cx, p.y - cy);
        sxx += dx * dx;
        sxy += dx * dy;
        syy += dy * dy;
    }
    let theta = 0.5 * (2.0 * sxy).atan2(sxx - syy);
    let (s, c) = theta.sin_cos();
    Some((Point::new(cx, cy), Point::new(c, s)))
}

fn intersect(l1: (Point, Point), l2: (Point, Point)) -> Option<Point> {
    let (p, d) = l1;
    let (q, e) = l2;
    let det = d.x * -e.y - d.y * -e.x;
    if det.abs() < 0.2 {
        return None;
    }
    let (rx, ry) = (q.x - p.x, q.y - p.y);
    let s = (rx * -e.y - ry * -e.x) / det;
    Some(axpy(p, s, d))
}

/// Image corners of the finder in the pose's frame, in the order
/// (-,-), (+,-), (+,+), (-,+) in units of (u, v).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Corners([Point; 4]);

impl Corners {
    /// Each side is the line through points found by walking outwards from
    /// the dark ring to the first light pixel, so perspective and uneven
    /// scale show up. Falls back to the similarity corners when a side
    /// cannot be measured.
    pub fn measure(plane: Plane<'_>, pose: Pose) -> Self {
        let (u, v) = (pose.u, rot90(pose.u));
        let m = len(u);
        let nominal = |sx: f32, sy: f32| axpy(axpy(pose.c, 3.5 * sx, u), 3.5 * sy, v);
        let fallback = [
            nominal(-1.0, -1.0),
            nominal(1.0, -1.0),
            nominal(1.0, 1.0),
            nominal(-1.0, 1.0),
        ];
        // From the ring at 3 modules along the outward normal `n`, sliding
        // along `t`.
        let side = |n: Point, t: Point| -> Option<(Point, Point)> {
            let un = Point::new(n.x / m, n.y / m);
            let mut pts = [Point::default(); 6];
            let mut k = 0;
            for i in 0..6 {
                let off = i as f32 - 2.5;
                let start = axpy(axpy(pose.c, 3.0, n), off, t);
                if let Some(d) = edge_distance(plane, start, un, 0.9 * m) {
                    pts[k] = axpy(start, d, un);
                    k += 1;
                }
            }
            fit_line(&pts[..k])
        };
        let neg = |p: Point| Point::new(-p.x, -p.y);
        let (Some(top), Some(right), Some(bottom), Some(left)) =
            (side(neg(v), u), side(u, v), side(v, u), side(neg(u), v))
        else {
            return Self(fallback);
        };
        let (Some(tl), Some(tr), Some(br), Some(bl)) = (
            intersect(top, left),
            intersect(top, right),
            intersect(bottom, right),
            intersect(bottom, left),
        ) else {
            return Self(fallback);
        };
        let c = [tl, tr, br, bl];
        // Each corner must lie within a module and a half of the similarity
        // estimate.
        if c.iter()
            .zip(&fallback)
            .any(|(a, b)| a.distance(*b) > 1.5 * m)
        {
            return Self(fallback);
        }
        Self(c)
    }
}

/// Timing edges along row 0 or column 0 as module coordinates of the
/// transitions: dark to light at x = 7, then alternating at 8, 9, ... until
/// the quiet zone. `edges[i]` is the measured position of the transition
/// expected at `7 + i`.
#[derive(Debug, Clone, Copy)]
pub(super) struct Timing {
    pub n: usize,
    pub edges: [f32; 12],
}

const SAMPLES: usize = 129;
const STEP: f32 = 0.125;
const S0: f32 = 4.0;

/// Reads the timing pattern under the mapping `h` along row 0 (or column 0
/// when `column`): the finder's outer row, the separator, then alternating
/// modules that end with a dark one followed by light.
pub(super) fn analyze_timing(plane: Plane<'_>, h: &Homography, column: bool) -> Option<Timing> {
    let mut trans = [0f32; SAMPLES];
    let mut nt = 0;
    let mut prev = false;
    for i in 0..SAMPLES {
        let s = S0 + i as f32 * STEP;
        let q = if column {
            Point::new(0.5, s)
        } else {
            Point::new(s, 0.5)
        };
        let v = h.map(q).and_then(|p| dark(plane, p));
        if i == 0 {
            if v != Some(true) {
                return None;
            }
            prev = true;
            continue;
        }
        let v = v == Some(true);
        if v != prev {
            trans[nt] = S0 + (i as f32 - 0.5) * STEP;
            nt += 1;
            prev = v;
        }
    }
    if nt == 0 || !(6.2..=7.8).contains(&trans[0]) {
        return None;
    }
    // Edges follow s = alpha + beta * x; with few accepted edges the pitch
    // is taken as nominal.
    let fit = |n: usize| -> (f32, f32) {
        let nf = n as f32;
        let (mut sx, mut sy, mut sxx, mut sxy) = (0.0, 0.0, 0.0, 0.0);
        for (i, &t) in trans[..n].iter().enumerate() {
            let x = 7.0 + i as f32;
            sx += x;
            sy += t;
            sxx += x * x;
            sxy += x * t;
        }
        let den = nf * sxx - sx * sx;
        if n < 4 || den.abs() < 1e-6 {
            let off = (0..n).map(|i| trans[i] - 7.0 - i as f32).sum::<f32>();
            return (off / nf, 1.0);
        }
        let beta = (nf * sxy - sx * sy) / den;
        ((sy - beta * sx) / nf, beta)
    };
    let mut n = 1;
    while n < nt && n < 12 {
        let (alpha, beta) = fit(n);
        let pred = alpha + beta * (7.0 + n as f32);
        if (trans[n] - pred).abs() <= EDGE_TOL * beta.max(0.5) {
            n += 1;
        } else {
            break;
        }
    }
    if !matches!(n, 5 | 7 | 9 | 11) {
        return None;
    }
    let (_, beta) = fit(n);
    if !(0.7..=1.4).contains(&beta) {
        return None;
    }
    // The quiet zone follows: no edge within a module and a half.
    if n < nt && trans[n] - trans[n - 1] < 1.6 * beta {
        return None;
    }
    let mut edges = [0f32; 12];
    edges[..n].copy_from_slice(&trans[..n]);
    Some(Timing { n, edges })
}

/// Mapping from module coordinates to the image for the quadrant `q` (0 to
/// 3) of the symbol relative to the finder pose: the symbol's x axis is the
/// finder's x axis turned by `q` quarter turns. Starts from the finder's
/// corners, reads the timing patterns through it, and refits the mapping
/// with the edges found. Returns the module count and the mapping.
pub(super) fn mapping_for_quadrant(
    plane: Plane<'_>,
    corners: &Corners,
    q: usize,
    pts: &mut Vec<(Point, Point)>,
) -> Option<(usize, Homography)> {
    // Turning the symbol by a quarter moves its (0,0) corner to the next
    // finder corner.
    let at = |i: usize| corners.0[(i + q) % 4];
    let finder = [
        (Point::new(0.0, 0.0), at(0)),
        (Point::new(7.0, 0.0), at(1)),
        (Point::new(7.0, 7.0), at(2)),
        (Point::new(0.0, 7.0), at(3)),
    ];
    let mut h = Homography::fit(&finder)?;
    let mut d = 0;
    for round in 0..3 {
        let ta = analyze_timing(plane, &h, false);
        let tb = analyze_timing(plane, &h, true);
        // One clean line may propose the size in the first round; the fit
        // below must then confirm it on both.
        let n = match (ta, tb) {
            (Some(a), Some(b)) if a.n == b.n => a.n,
            _ if round == 0 => ta.map_or(0, |t| t.n).max(tb.map_or(0, |t| t.n)),
            _ => return None,
        };
        if n == 0 {
            return None;
        }
        d = n + 6;
        pts.clear();
        pts.extend_from_slice(&finder);
        for (t, column) in [(ta, false), (tb, true)] {
            let Some(t) = t.filter(|t| t.n == n) else {
                continue;
            };
            for i in 0..n {
                let x = 7.0 + i as f32;
                let (m, e) = if column {
                    (Point::new(0.5, x), Point::new(0.5, t.edges[i]))
                } else {
                    (Point::new(x, 0.5), Point::new(t.edges[i], 0.5))
                };
                pts.push((m, h.map(e)?));
            }
        }
        add_boundary_points(plane, &h, d, pts);
        h = Homography::fit(pts)?;
    }
    // The fitted mapping must still show the pattern on both axes.
    let (a, b) = (
        analyze_timing(plane, &h, false)?,
        analyze_timing(plane, &h, true)?,
    );
    (a.n + 6 == d && b.n + 6 == d).then_some((d, h))
}

/// Pairs on the symbol's top and left boundary: the outer edge of the
/// finder's top row and left column and of the dark timing modules.
fn add_boundary_points(plane: Plane<'_>, h: &Homography, d: usize, pts: &mut Vec<(Point, Point)>) {
    let mut xs = [0f32; 12];
    let mut nx = 0;
    for k in 0..7 {
        xs[nx] = k as f32 + 0.5;
        nx += 1;
    }
    let mut k = 8;
    while k < d {
        xs[nx] = k as f32 + 0.5;
        nx += 1;
        k += 2;
    }
    for &x in &xs[..nx] {
        for column in [false, true] {
            let module = |t: f32| {
                if column {
                    Point::new(0.5 - t, x)
                } else {
                    Point::new(x, 0.5 - t)
                }
            };
            if h.map(module(0.0)).and_then(|p| dark(plane, p)) != Some(true) {
                continue;
            }
            // Walk towards the outside in steps of a twentieth of a module.
            let mut t = 0.05;
            while t <= 0.95 {
                let Some(p) = h.map(module(t)) else { break };
                if dark(plane, p) == Some(false) {
                    if t >= 0.2
                        && let Some(e) = h.map(module(t - 0.025))
                    {
                        let m = if column {
                            Point::new(0.0, x)
                        } else {
                            Point::new(x, 0.0)
                        };
                        pts.push((m, e));
                    }
                    break;
                }
                t += 0.05;
            }
        }
    }
}

#[cfg(test)]
mod score_tests {
    use super::*;
    use breader_core::BitMatrix;

    /// A noisy plane with a finder pattern of about `module` pixels at
    /// `(60, 60)` turned by `deg`.
    fn finder_plane(rng: &mut u64, module: f32, deg: f32, noise: u64) -> BitMatrix {
        let mut next = move || {
            *rng ^= *rng << 13;
            *rng ^= *rng >> 7;
            *rng ^= *rng << 17;
            *rng
        };
        let mut g = BitMatrix::new(120, 120);
        let u = unit(deg, module);
        let v = rot90(u);
        for y in 0..120 {
            for x in 0..120 {
                // Inverse of the pose: module coordinates of the pixel.
                let (dx, dy) = (x as f32 - 60.0, y as f32 - 60.0);
                let det = u.x * v.y - u.y * v.x;
                let i = (dx * v.y - dy * v.x) / det;
                let j = (dy * u.x - dx * u.y) / det;
                let r = i.abs().max(j.abs());
                let mut dark = r < 1.5 || (2.5..3.5).contains(&r);
                if next() % 100 < noise {
                    dark = !dark;
                }
                g.set(x, y, dark);
            }
        }
        g
    }

    #[test]
    fn template_score_from_is_the_full_score_or_none() {
        let mut rng = 0x1234_5678_9abc_def1u64;
        let mut cut = 0;
        for trial in 0..40 {
            let plane_bits =
                finder_plane(&mut rng, 3.0 + (trial % 5) as f32, (trial * 9) as f32, 3);
            let plane = Plane::new(&plane_bits, false);
            for k in 0..200 {
                let c = Point::new(55.0 + (k % 11) as f32, 55.0 + (k / 11 % 11) as f32);
                let u = unit(-30.0 + (k % 7) as f32 * 9.0, 2.5 + (k % 5) as f32);
                let full = template_score(plane, c, u);
                for at_least in [i32::MIN, 0, 30, 55, 60, full, full + 1, 81] {
                    let got = template_score_from(plane, c, u, at_least);
                    if full >= at_least {
                        assert_eq!(got, Some(full));
                    } else {
                        assert_eq!(got, None);
                        cut += 1;
                    }
                }
            }
        }
        assert!(cut > 1000, "{cut}");
    }

    #[test]
    fn offered_poses_leave_the_same_best_as_added_scores() {
        let mut rng = 0x0bad_cafe_1234_5678u64;
        for trial in 0..30 {
            let bits = finder_plane(&mut rng, 3.5, (trial * 11) as f32, 4);
            let plane = Plane::new(&bits, false);
            let (mut a, mut b) = (Best::new(), Best::new());
            for k in 0..120 {
                let c = Point::new(57.0 + (k % 7) as f32, 57.0 + (k / 7 % 7) as f32);
                let ang = -20.0 + (k % 13) as f32 * 3.0;
                let u = unit(ang, 3.0 + (k % 9) as f32 * 0.2);
                let p = [ang, 3.5, c.x, c.y];
                a.add(template_score(plane, c, u), p);
                b.offer(plane, c, u, p);
                assert_eq!(a.score, b.score);
                assert_eq!(a.n, b.n);
                assert_eq!(a.sum.map(f32::to_bits), b.sum.map(f32::to_bits));
            }
        }
    }
}
