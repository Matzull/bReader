//! rMQR candidates from the finder patterns of the QR scan.
//!
//! The finder at the top left gives a coarse frame; the timing patterns along
//! the four edges and the vertical ones at the alignment columns then pin the
//! mapping down over the whole (up to 139 modules long) symbol.

use breader_core::{
    BitMatrix, Homography, Plane, Point, QrMeta, Quad, Symbol, SymbolMeta, Symbology,
};

use super::decode::{DecodedRmqr, decode_grid, left_format};
use super::spec::{DARK, LIGHT, SIZES, Template, align_centers, template};
use crate::micro::geom::{Pose, dark};
use crate::sample::{Gray, Mapper, read_module};

/// Mismatches allowed in the finder with its separators.
const FINDER_TOLERANCE: u32 = 6;
/// Other known modules that may disagree, as a share (percent).
const PATTERN_TOLERANCE_PERCENT: usize = 10;
/// Fraction (percent) of in-image quiet-zone samples that must be light.
const QUIET_MIN_PERCENT: u32 = 75;
/// Smallest share (percent) of the timing edges that must be found.
const EDGE_MIN_PERCENT: usize = 75;
/// Search window around a predicted edge, in modules.
const WINDOW: f32 = 0.55;

/// A boundary between two modules of known, different values.
#[derive(Debug, Clone, Copy)]
struct Probe {
    /// Module-space point on the boundary.
    at: Point,
    /// The boundary is crossed along y (otherwise along x).
    vertical: bool,
    /// The module before the boundary is dark.
    before_dark: bool,
    /// Outer boundary of the symbol: needs a light quiet zone, so missing
    /// edges do not count against the symbol.
    optional: bool,
    /// Timing line the boundary belongs to (255: none), whose own edges
    /// predict the next one while the mapping is still being found.
    group: u8,
}

/// Buffers kept between scans.
#[derive(Debug, Default)]
pub(crate) struct RmqrBuf {
    grid: BitMatrix,
    fmt: BitMatrix,
    pts: Vec<(Point, Point)>,
    seed: Vec<(Point, Point)>,
    probes: Vec<Probe>,
}

fn rot90_corner(c: [Point; 4], q: usize, mirrored: bool) -> [(Point, Point); 4] {
    let m = [
        Point::new(0.0, 0.0),
        Point::new(7.0, 0.0),
        Point::new(7.0, 7.0),
        Point::new(0.0, 7.0),
    ];
    let mut out = [(Point::default(), Point::default()); 4];
    for (i, o) in out.iter_mut().enumerate() {
        let k = if mirrored {
            (q + 4 - i) % 4
        } else {
            (i + q) % 4
        };
        *o = (m[i], c[k]);
    }
    out
}

/// Number of timing boundaries after the finder used to pin down the frame.
const START_EDGES: usize = 10;

/// Refits the finder frame with the first boundaries of the top timing row
/// (dark to light at 7, then alternating). `None` when too few are found,
/// which also rejects wrong orientations.
fn start_mapping(
    bin: Plane<'_>,
    finder: &[(Point, Point); 4],
    h0: &Homography,
    pts: &mut Vec<(Point, Point)>,
) -> Option<Homography> {
    pts.clear();
    let mut h = *h0;
    // The outer edges of the finder pin its position down better than its
    // fitted center does. A point found on an edge is only as good as the
    // prediction along that edge, so the fit is repeated.
    for _ in 0..3 {
        pts.clear();
        for i in 0..7 {
            let t = i as f32 + 0.5;
            let sides = [
                (Point::new(t, 0.0), true, false),
                (Point::new(t, 7.0), true, true),
                (Point::new(0.0, t), false, false),
                (Point::new(7.0, t), false, true),
            ];
            for (at, vertical, before_dark) in sides {
                let probe = Probe {
                    at,
                    vertical,
                    before_dark,
                    optional: true,
                    group: 255,
                };
                if let Some(img) = locate(bin, &h, &probe, 0.6) {
                    pts.push((at, img));
                }
            }
        }
        if pts.len() < 20 {
            pts.clear();
            pts.extend_from_slice(finder);
            break;
        }
        h = best_fit(pts).unwrap_or(h);
    }
    let mut found = 0;
    for k in 0..START_EDGES {
        let x = 7 + k;
        let probe = Probe {
            at: Point::new(x as f32, 0.5),
            vertical: false,
            before_dark: x % 2 == 1,
            optional: false,
            group: 0,
        };
        if let Some(img) = locate(bin, &h, &probe, 0.7) {
            pts.push((probe.at, img));
            found += 1;
            h = best_fit(pts).unwrap_or(h);
        }
    }
    if found < START_EDGES - 4 {
        return None;
    }
    best_fit(pts)
}

/// Walks along the top timing row from the edges already found and returns
/// the symbol width the end of the row implies: alternating boundaries, three
/// dark modules at every alignment column and at the right end.
fn track_width(bin: Plane<'_>, h: &Homography, seed: &[(Point, Point)]) -> Option<usize> {
    // The last boundaries found, newest first: (boundary x, image point).
    const KEEP: usize = 8;
    let mut recent = [(0usize, Point::default()); KEEP];
    let mut n = 0;
    for (m, img) in seed.iter().rev() {
        if (m.y - 0.5).abs() < 1e-3 && m.x >= 7.0 && n < KEEP {
            recent[n] = (m.x as usize, *img);
            n += 1;
        }
    }
    if n < 2 {
        return None;
    }
    let probe_at = |x: usize| Probe {
        at: Point::new(x as f32, 0.5),
        vertical: false,
        before_dark: x % 2 == 1,
        optional: false,
        group: 255,
    };
    let (mut x, mut last) = recent[0];
    for _ in 0..160 {
        let (x0, p0) = recent[n - 1];
        if x <= x0 {
            return None;
        }
        let k = (x - x0) as f32;
        let dir = Point::new((last.x - p0.x) / k, (last.y - p0.y) / k);
        // After a boundary at an odd x the module is light and the next
        // boundary follows at once; after an even one the module is dark and
        // may stay so for three modules.
        let gaps: &[usize] = if x % 2 == 1 { &[1] } else { &[1, 3] };
        let mut next = None;
        for &g in gaps {
            let guess = Point::new(last.x + dir.x * g as f32, last.y + dir.y * g as f32);
            if let Some(p) = locate_near(bin, h, &probe_at(x + g), 0.45, Some(guess)) {
                next = Some((x + g, p));
                break;
            }
        }
        let Some((nx, p)) = next else {
            break;
        };
        recent.copy_within(0..KEEP - 1, 1);
        recent[0] = (nx, p);
        n = (n + 1).min(KEEP);
        x = nx;
        last = p;
    }
    // The row ends where no boundary follows: after the dark corner modules
    // (the last boundary is a dark-to-light one at the width) or at the image
    // border (the last boundary starts the corner run).
    let w = if x % 2 == 1 { x } else { x + 3 };
    matches!(w, 27 | 43 | 59 | 77 | 99 | 139).then_some(w)
}

/// Left format information read through a coarse mapping: the 6-bit value.
fn coarse_format(bin: Plane<'_>, h: &Homography, fmt: &mut BitMatrix) -> Option<(u32, u8)> {
    fmt.resize(12, 6);
    let m = Mapper::Fitted(*h);
    for y in 1..6 {
        for x in 8..12 {
            if x == 11 && y > 3 {
                continue;
            }
            match read_module(bin, &m, x, y, false)? {
                Some(true) => fmt.set(x, y, true),
                Some(false) => {}
                None => return None,
            }
        }
    }
    left_format(fmt)
}

/// Where the boundary of `probe` lies in the image under `h`, searched within
/// `window` modules of the prediction.
fn locate(bin: Plane<'_>, h: &Homography, probe: &Probe, window: f32) -> Option<Point> {
    locate_near(bin, h, probe, window, None)
}

/// [`locate`] around `center` (image point) instead of the mapped boundary.
fn locate_near(
    bin: Plane<'_>,
    h: &Homography,
    probe: &Probe,
    window: f32,
    center: Option<Point>,
) -> Option<Point> {
    let e = if probe.vertical {
        Point::new(0.0, 0.5)
    } else {
        Point::new(0.5, 0.0)
    };
    let c = match center {
        Some(c) => c,
        None => h.map(probe.at)?,
    };
    let a = h.map(Point::new(probe.at.x - e.x, probe.at.y - e.y))?;
    let b = h.map(Point::new(probe.at.x + e.x, probe.at.y + e.y))?;
    let d = Point::new(b.x - a.x, b.y - a.y);
    let len = d.x.hypot(d.y);
    if !(1.0..300.0).contains(&len) {
        return None;
    }
    let n = ((2.0 * window * len / 0.4).ceil() as usize).max(6);
    let mut best: Option<f32> = None;
    let mut prev: Option<bool> = None;
    for i in 0..=n {
        let t = -window + 2.0 * window * i as f32 / n as f32;
        let v = dark(bin, Point::new(c.x + t * d.x, c.y + t * d.y));
        if let (Some(p), Some(v)) = (prev, v)
            && p == probe.before_dark
            && v != p
        {
            let tm = t - window / n as f32;
            if best.is_none_or(|b| tm.abs() < b.abs()) {
                best = Some(tm);
            }
        }
        prev = v;
    }
    best.map(|t| Point::new(c.x + t * d.x, c.y + t * d.y))
}

fn collect_probes(t: &Template, probes: &mut Vec<Probe>) {
    probes.clear();
    let (w, h) = (t.w, t.h);
    let known = |x: usize, y: usize| matches!(t.cell(x, y), LIGHT | DARK);
    for y in [0, h - 1] {
        for x in 1..w {
            if known(x - 1, y) && known(x, y) && t.cell(x - 1, y) != t.cell(x, y) {
                probes.push(Probe {
                    at: Point::new(x as f32, y as f32 + 0.5),
                    vertical: false,
                    before_dark: t.cell(x - 1, y) == DARK,
                    optional: false,
                    group: u8::from(y != 0),
                });
            }
        }
    }
    let aligns = align_centers(w);
    for k in 0..2 + aligns.len() {
        let x = match k {
            0 => 0,
            1 => w - 1,
            _ => usize::from(aligns[k - 2]),
        };
        for y in 1..h {
            if known(x, y - 1) && known(x, y) && t.cell(x, y - 1) != t.cell(x, y) {
                probes.push(Probe {
                    at: Point::new(x as f32 + 0.5, y as f32),
                    vertical: true,
                    before_dark: t.cell(x, y - 1) == DARK,
                    optional: false,
                    group: (2 + k) as u8,
                });
            }
        }
    }
    // The outer boundary of the dark edge modules against the quiet zone.
    for x in 0..w {
        if t.cell(x, 0) == DARK {
            probes.push(Probe {
                at: Point::new(x as f32 + 0.5, 0.0),
                vertical: true,
                before_dark: false,
                optional: true,
                group: 255,
            });
        }
        if t.cell(x, h - 1) == DARK {
            probes.push(Probe {
                at: Point::new(x as f32 + 0.5, h as f32),
                vertical: true,
                before_dark: true,
                optional: true,
                group: 255,
            });
        }
    }
    for y in 0..h {
        if t.cell(0, y) == DARK {
            probes.push(Probe {
                at: Point::new(0.0, y as f32 + 0.5),
                vertical: false,
                before_dark: false,
                optional: true,
                group: 255,
            });
        }
        if t.cell(w - 1, y) == DARK {
            probes.push(Probe {
                at: Point::new(w as f32, y as f32 + 0.5),
                vertical: false,
                before_dark: true,
                optional: true,
                group: 255,
            });
        }
    }
    // Right and bottom boundary of the finder against its separators.
    for i in 0..7 {
        let t = i as f32 + 0.5;
        probes.push(Probe {
            at: Point::new(7.0, t),
            vertical: false,
            before_dark: true,
            optional: true,
            group: 255,
        });
        if h > 7 {
            probes.push(Probe {
                at: Point::new(t, 7.0),
                vertical: true,
                before_dark: true,
                optional: true,
                group: 255,
            });
        }
    }
    probes.sort_by(|a, b| a.at.x.total_cmp(&b.at.x).then(a.at.y.total_cmp(&b.at.y)));
}

/// Least-squares affine map of the pairs, as a homography.
fn fit_affine(pairs: &[(Point, Point)]) -> Option<Homography> {
    // Normal equations of [x y 1] * coefficients = target, shared by both outputs.
    let mut m = [[0.0f64; 3]; 3];
    let mut bx = [0.0f64; 3];
    let mut by = [0.0f64; 3];
    for (s, d) in pairs {
        let v = [f64::from(s.x), f64::from(s.y), 1.0];
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += v[i] * v[j];
            }
            bx[i] += v[i] * f64::from(d.x);
            by[i] += v[i] * f64::from(d.y);
        }
    }
    let solve = |b: [f64; 3]| -> Option<[f64; 3]> {
        let mut a = [[0.0f64; 4]; 3];
        for i in 0..3 {
            a[i][..3].copy_from_slice(&m[i]);
            a[i][3] = b[i];
        }
        for c in 0..3 {
            let p = (c..3).max_by(|&x, &y| a[x][c].abs().total_cmp(&a[y][c].abs()))?;
            if a[p][c].abs() < 1e-9 {
                return None;
            }
            a.swap(c, p);
            for r in 0..3 {
                if r != c {
                    let f = a[r][c] / a[c][c];
                    let row = a[c];
                    for k in c..4 {
                        a[r][k] -= f * row[k];
                    }
                }
            }
        }
        Some([a[0][3] / a[0][0], a[1][3] / a[1][1], a[2][3] / a[2][2]])
    };
    let (cx, cy) = (solve(bx)?, solve(by)?);
    let at = |x: f32, y: f32| {
        Point::new(
            (cx[0] * f64::from(x) + cx[1] * f64::from(y) + cx[2]) as f32,
            (cy[0] * f64::from(x) + cy[1] * f64::from(y) + cy[2]) as f32,
        )
    };
    let probe = [(0.0, 0.0), (64.0, 0.0), (64.0, 64.0), (0.0, 64.0)];
    let four = probe.map(|(x, y)| (Point::new(x, y), at(x, y)));
    Homography::fit(&four)
}

fn rms(h: &Homography, pairs: &[(Point, Point)]) -> f32 {
    let mut sum = 0.0f32;
    for (s, d) in pairs {
        match h.map(*s) {
            Some(p) => sum += p.distance(*d).powi(2),
            None => return f32::MAX,
        }
    }
    (sum / pairs.len().max(1) as f32).sqrt()
}

/// The projective fit, or the affine one when it explains the points about
/// as well (a long, thin symbol leaves the perspective terms to noise).
fn best_fit(pairs: &[(Point, Point)]) -> Option<Homography> {
    let p = Homography::fit(pairs);
    let a = fit_affine(pairs);
    match (p, a) {
        (Some(p), Some(a)) => {
            let (rp, ra) = (rms(&p, pairs), rms(&a, pairs));
            Some(if ra <= rp * 1.1 + 0.08 { a } else { p })
        }
        (p, a) => p.or(a),
    }
}

/// Fits the mapping over the whole symbol, growing from the finder outwards,
/// then repeats the fit with the whole model so that every point is searched
/// with a good prediction.
fn fit_mapping(
    bin: Plane<'_>,
    seed: &[(Point, Point)],
    probes: &[Probe],
    pts: &mut Vec<(Point, Point)>,
) -> Option<Homography> {
    let mut h = best_fit(seed)?;
    pts.clear();
    pts.extend_from_slice(seed);
    let (mut since, mut miss_run) = (0, 0);
    // Last edges found on each timing line: module coordinate along the line
    // and image point, newest first.
    const KEEP: usize = 8;
    let mut last = [[None::<(f32, Point)>; KEEP]; 12];
    for p in probes {
        let along = if p.vertical { p.at.y } else { p.at.x };
        let guess = (p.group < 12)
            .then(|| last[usize::from(p.group)])
            .and_then(|l| {
                let (a1, q1) = l[0]?;
                let (a0, q0) = l.iter().rev().flatten().next().copied()?;
                if (a1 - a0).abs() < 3.5 {
                    return None;
                }
                let k = (along - a1) / (a1 - a0);
                Some(Point::new(
                    q1.x + k * (q1.x - q0.x),
                    q1.y + k * (q1.y - q0.y),
                ))
            });
        let found = locate(bin, &h, p, WINDOW)
            .or_else(|| guess.and_then(|g| locate_near(bin, &h, p, WINDOW, Some(g))));
        match found {
            Some(img) => {
                pts.push((p.at, img));
                if p.group < 12 {
                    let l = &mut last[usize::from(p.group)];
                    l.copy_within(0..KEEP - 1, 1);
                    l[0] = Some((along, img));
                }
                since += 1;
                miss_run = 0;
            }
            None if p.optional => {}
            None => {
                miss_run += 1;
                if miss_run >= 8 {
                    return None;
                }
            }
        }
        if since >= 6 && pts.len() >= 8 {
            h = best_fit(pts).unwrap_or(h);
            since = 0;
        }
    }
    h = best_fit(pts).unwrap_or(h);
    for round in 0..3 {
        pts.clear();
        let (mut hits, mut needed) = (0, 0);
        for p in probes {
            needed += usize::from(!p.optional);
            if let Some(img) = locate(bin, &h, p, WINDOW * (0.7 - 0.1 * round as f32)) {
                pts.push((p.at, img));
                hits += usize::from(!p.optional);
            }
        }
        if hits * 100 < needed * EDGE_MIN_PERCENT || pts.len() < 12 {
            return None;
        }
        h = best_fit(pts)?;
    }
    Some(h)
}

fn module_px(h: &Homography, w: usize, ht: usize) -> f32 {
    let (cx, cy) = (w as f32 / 2.0, ht as f32 / 2.0);
    match (
        h.map(Point::new(cx, cy)),
        h.map(Point::new(cx + 1.0, cy)),
        h.map(Point::new(cx, cy + 1.0)),
    ) {
        (Some(p), Some(a), Some(b)) => (p.distance(a) + p.distance(b)) / 2.0,
        _ => 0.0,
    }
}

fn sample(
    bin: Plane<'_>,
    m: &Mapper,
    w: usize,
    h: usize,
    multi: bool,
    grid: &mut BitMatrix,
) -> bool {
    grid.resize(w, h);
    let mut outside = 0;
    for y in 0..h {
        for x in 0..w {
            match read_module(bin, m, x, y, multi) {
                None => return false,
                Some(None) => outside += 1,
                Some(Some(v)) => grid.set(x, y, v),
            }
        }
    }
    outside * 20 <= w * h
}

/// Finder with separators and the other function patterns against the template.
fn structure_ok(g: &BitMatrix, t: &Template) -> bool {
    let (mut bad_finder, mut bad_rest) = (0u32, 0usize);
    let rows = if t.h > 7 { 8 } else { 7 };
    for y in 0..t.h {
        for x in 0..t.w {
            let want = match t.cell(x, y) {
                DARK => true,
                LIGHT => false,
                _ => continue,
            };
            if g.get(x, y) != want {
                if x < 8 && y < rows {
                    bad_finder += 1;
                } else {
                    bad_rest += 1;
                }
            }
        }
    }
    bad_finder <= FINDER_TOLERANCE && bad_rest * 100 <= t.known * PATTERN_TOLERANCE_PERCENT
}

/// The two rings outside the symbol must be mostly light.
fn quiet_ok(bin: Plane<'_>, m: &Mapper, w: usize, h: usize) -> bool {
    let (mut light, mut total) = (0u32, 0u32);
    let (wf, hf) = (w as f32, h as f32);
    for r in [0.5f32, 1.5] {
        let sides = [
            (w, -r, true),
            (w, hf + r, true),
            (h, -r, false),
            (h, wf + r, false),
        ];
        for (n, off, horizontal) in sides {
            for i in 0..n {
                let t = i as f32 + 0.5;
                let p = if horizontal {
                    Point::new(t, off)
                } else {
                    Point::new(off, t)
                };
                let Some(q) = m.map(p) else {
                    return false;
                };
                if let Some(v) = dark(bin, q) {
                    total += 1;
                    light += u32::from(!v);
                }
            }
        }
    }
    total == 0 || light * 100 >= total * QUIET_MIN_PERCENT
}

struct Found {
    decoded: DecodedRmqr,
    h: Mapper,
    mirrored: bool,
}

#[allow(clippy::too_many_arguments)]
fn try_version(
    bin: Plane<'_>,
    gray: Option<Gray<'_>>,
    version: usize,
    mirrored: bool,
    retries: bool,
    buf: &mut RmqrBuf,
) -> Option<Found> {
    let t = template(version);
    let (w, ht) = (t.w, t.h);
    let RmqrBuf {
        grid,
        pts,
        seed,
        probes,
        ..
    } = buf;
    collect_probes(t, probes);
    let h = fit_mapping(bin, seed, probes, pts)?;
    let m = Mapper::Fitted(h);
    let px = module_px(&h, w, ht);
    if px < 1.0 || !quiet_ok(bin, &m, w, ht) {
        return None;
    }
    let found = |decoded: DecodedRmqr, h: Mapper| Found {
        decoded,
        h,
        mirrored,
    };
    let mut plausible = false;
    if sample(bin, &m, w, ht, px >= 3.0, grid) && structure_ok(grid, t) {
        plausible = true;
        if let Ok(d) = decode_grid(grid) {
            return Some(found(d, m));
        }
    }
    if retries
        && plausible
        && let Some(g) = gray
        && sample_local(g, bin.inverted(), &m, w, ht, grid)
        && structure_ok(grid, t)
        && let Ok(d) = decode_grid(grid)
    {
        return Some(found(d, m));
    }
    None
}

/// Samples the grid from the luma image against the mean of the modules
/// around each one.
fn sample_local(
    gray: Gray<'_>,
    inverted: bool,
    m: &Mapper,
    w: usize,
    h: usize,
    grid: &mut BitMatrix,
) -> bool {
    const OFFSETS: [(f32, f32); 5] = [
        (0.0, 0.0),
        (-0.2, -0.2),
        (0.2, -0.2),
        (-0.2, 0.2),
        (0.2, 0.2),
    ];
    const RADIUS: usize = 3;
    let mut vals = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0.0;
            for (ox, oy) in OFFSETS {
                let Some(p) = m.map(Point::new(x as f32 + 0.5 + ox, y as f32 + 0.5 + oy)) else {
                    return false;
                };
                let Some(v) = gray.value_at(p) else {
                    return false;
                };
                sum += v;
            }
            vals[y * w + x] = sum / OFFSETS.len() as f32;
        }
    }
    let n = w + 1;
    let mut sat = vec![0.0f64; n * (h + 1)];
    for y in 0..h {
        for x in 0..w {
            sat[(y + 1) * n + x + 1] =
                f64::from(vals[y * w + x]) + sat[y * n + x + 1] + sat[(y + 1) * n + x]
                    - sat[y * n + x];
        }
    }
    grid.resize(w, h);
    for y in 0..h {
        for x in 0..w {
            let (y0, y1) = (y.saturating_sub(RADIUS), (y + RADIUS + 1).min(h));
            let (x0, x1) = (x.saturating_sub(RADIUS), (x + RADIUS + 1).min(w));
            let area = ((y1 - y0) * (x1 - x0)) as f64;
            let mean =
                (sat[y1 * n + x1] - sat[y0 * n + x1] - sat[y1 * n + x0] + sat[y0 * n + x0]) / area;
            if (f64::from(vals[y * w + x]) < mean) != inverted {
                grid.set(x, y, true);
            }
        }
    }
    true
}

fn build_symbol(found: &Found, scale: (f32, f32)) -> Option<Symbol> {
    let dec = &found.decoded;
    let (w, h) = (dec.width as f32, dec.height as f32);
    let mut pts = [Point::default(); 4];
    for (p, (x, y)) in pts.iter_mut().zip([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]) {
        let q = found.h.map(Point::new(x, y))?;
        *p = Point::new(q.x * scale.0, q.y * scale.1);
    }
    let quad = Quad::new(pts);
    let mut meta = QrMeta::new(
        dec.version,
        dec.width as u16,
        dec.height as u16,
        dec.ec_level,
    );
    meta.errors_corrected = dec.errors_corrected;
    meta.eci = dec.eci.clone();
    meta.mirrored = found.mirrored;
    let mut symbol = Symbol::new(
        Symbology::Rmqr,
        dec.data.clone(),
        quad,
        dec.aim_id,
        SymbolMeta::Qr(meta),
    );
    symbol.segments = dec.segments.clone();
    symbol.quality = 1.0 / (1.0 + 0.1 * dec.errors_corrected as f32);
    Some(symbol)
}

/// Tries one finder (fitted pose and measured corners) as the top-left finder
/// of an rMQR symbol in all eight orientations.
pub(crate) fn try_finder(
    bin: Plane<'_>,
    gray: Option<Gray<'_>>,
    pose: &Pose,
    retries: bool,
    scale: (f32, f32),
    buf: &mut RmqrBuf,
) -> Option<Symbol> {
    // The finder as a square of the fitted pose: its angle is more accurate
    // than that of the measured corners, which matters over a long symbol.
    let v = Point::new(-pose.u.y, pose.u.x);
    let nominal = |sx: f32, sy: f32| {
        Point::new(
            pose.c.x + 3.5 * (sx * pose.u.x + sy * v.x),
            pose.c.y + 3.5 * (sx * pose.u.y + sy * v.y),
        )
    };
    let square = [
        nominal(-1.0, -1.0),
        nominal(1.0, -1.0),
        nominal(1.0, 1.0),
        nominal(-1.0, 1.0),
    ];
    for mirrored in [false, true] {
        for q in 0..4 {
            let finder = rot90_corner(square, q, mirrored);
            let Some(h0) = Homography::fit(&finder) else {
                continue;
            };
            let Some(h1) = start_mapping(bin, &finder, &h0, &mut buf.seed) else {
                continue;
            };
            // The left copy of the format information names the version. A
            // clean word (distance 0 or 1) is trusted; a doubtful or
            // unreadable one is followed by the other sizes, so that the
            // right copy and the other gates decide.
            let named = coarse_format(bin, &h1, &mut buf.fmt);
            let first = named.map(|(_, d6)| usize::from(d6 & 31));
            let trusted = named.is_some_and(|(d, _)| d <= 1);
            let mut order = [0usize; SIZES.len() + 1];
            let mut n = 0;
            if let Some(f) = first {
                order[0] = f;
                n = 1;
            }
            if retries && !trusted {
                // Doubtful or missing: the width at the end of the timing row
                // narrows the sizes down.
                let tw = track_width(bin, &h1, &buf.seed);
                if let Some(w) = tw {
                    for v in (0..SIZES.len()).filter(|&v| usize::from(SIZES[v].1) == w) {
                        if Some(v) != first {
                            order[n] = v;
                            n += 1;
                        }
                    }
                }
            }
            for &version in &order[..n] {
                if let Some(found) = try_version(bin, gray, version, mirrored, retries, buf)
                    && let Some(sym) = build_symbol(&found, scale)
                {
                    return Some(sym);
                }
            }
        }
    }
    None
}
