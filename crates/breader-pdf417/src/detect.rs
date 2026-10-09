//! From start and stop patterns on the scan plan to the boundary of a
//! symbol: seed ordering and boundary growth with a line fit.

use breader_core::{ImageView, Point};
use breader_linear::stacked::{EdgeParams, Mark, MarkKind};

use crate::geom::{Line, Vec2, at, dot, perp, robust_fit, sub, unit};
use crate::scan::Scanner;

/// Most seeds ranked; the rest of a crowded frame is left alone.
const MAX_RANKED: usize = 600;
/// Lines on each side of a seed.
const MAX_STEPS: usize = 600;
/// Consecutive lines without the pattern that end the growth on one side.
const MAX_MISSES: usize = 10;
/// Fewest pattern hits of a boundary.
const MIN_HITS: usize = 4;
/// Shortest boundary, in modules.
const MIN_EXTENT: f32 = 6.0;
/// The scan line must cross the boundary at least this far from parallel
/// (sine of the angle).
const MIN_CROSSING: f32 = 0.35;

/// A side of a symbol found from its start or stop pattern.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Boundary {
    pub kind: MarkKind,
    pub inverted: bool,
    /// The line of the pattern's outer edge, `dir` along the boundary.
    pub line: Line,
    /// Unit vector along the rows, from the start side to the stop side.
    pub rows: Vec2,
    /// Extent of the hits along `line`.
    pub lo: f32,
    pub hi: f32,
    /// Samples per module along the rows.
    pub module: f32,
}

/// Indices of `marks` in the order seeds should be tried: patterns with many
/// agreeing neighbours first (a symbol is crossed by several scanlines, a
/// stray pattern by one).
pub(crate) fn rank_seeds(marks: &[Mark], limit: usize) -> Vec<usize> {
    let n = marks.len().min(MAX_RANKED);
    let center = |m: &Mark| Point::new((m.from.x + m.to.x) / 2.0, (m.from.y + m.to.y) / 2.0);
    let mut scored: Vec<(usize, usize)> = (0..n)
        .map(|i| {
            let (a, ca) = (&marks[i], center(&marks[i]));
            let reach = 120.0 * a.module;
            let near = (0..n)
                .filter(|&j| {
                    let b = &marks[j];
                    j != i
                        && b.kind == a.kind
                        && b.inverted == a.inverted
                        && b.module < 1.6 * a.module
                        && b.module > a.module / 1.6
                        && center(b).distance(ca) <= reach
                })
                .count();
            (i, near)
        })
        .collect();
    scored.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(marks[a.0].module.total_cmp(&marks[b.0].module))
            .then(a.0.cmp(&b.0))
    });
    scored.into_iter().take(limit).map(|(i, _)| i).collect()
}

/// Where a boundary growth starts: the outer edge of a pattern, the
/// direction of the lines to scan and the module along them.
#[derive(Debug, Clone, Copy)]
struct Origin {
    kind: MarkKind,
    inverted: bool,
    outer: Point,
    d: Vec2,
    module: f32,
}

/// The pattern of `o.kind` on the line through `center` along `o.d`,
/// expected `pred` samples from `center` (for its outer edge) within `tol`.
/// Returns its outer edge position and module.
fn probe(
    scanner: &mut Scanner,
    img: &ImageView<'_>,
    o: &Origin,
    center: Point,
    pred: f32,
    tol: f32,
    params: &EdgeParams,
) -> Option<(f32, f32)> {
    let m0 = o.module;
    let range = match o.kind {
        MarkKind::Start => (pred - tol - 4.0 * m0, pred + tol + 22.0 * m0),
        MarkKind::Stop => (pred - tol - 22.0 * m0, pred + tol + 4.0 * m0),
    };
    if !scanner.load(img, center, o.d, range, o.inverted, params) {
        return None;
    }
    scanner
        .marks
        .iter()
        .filter(|m| m.kind == o.kind && !m.inverted && m.to > m.from)
        .filter(|m| m.module > 0.6 * m0 && m.module < 1.6 * m0)
        .map(|m| {
            let p = if o.kind == MarkKind::Start {
                m.from
            } else {
                m.to
            };
            (scanner.t_of(p), m.module)
        })
        .filter(|(t, _)| (t - pred).abs() <= tol)
        .min_by(|a, b| (a.0 - pred).abs().total_cmp(&(b.0 - pred).abs()))
}

/// Where the pattern is expected on line `k` and how far off it may be:
/// wide while the walk has seen at most one hit besides the origin, then
/// around the least squares line through the recent hits.
fn predict(seen: &[(f32, f32)], k: f32, step: f32, m0: f32) -> (f32, f32) {
    let recent = &seen[seen.len().saturating_sub(12)..];
    if recent.len() < 3 {
        let last = recent[recent.len() - 1];
        let slope = if recent.len() == 2 {
            (last.1 - recent[0].1) / (last.0 - recent[0].0)
        } else {
            0.0
        };
        return (last.1 + slope * (k - last.0), 1.3 * step + 2.5 * m0);
    }
    let n = recent.len() as f32;
    let (mk, mo) = recent
        .iter()
        .fold((0.0, 0.0), |a, p| (a.0 + p.0 / n, a.1 + p.1 / n));
    let (mut num, mut den) = (0.0, 0.0);
    for p in recent {
        num += (p.0 - mk) * (p.1 - mo);
        den += (p.0 - mk) * (p.0 - mk);
    }
    let slope = if den > 1e-6 { num / den } else { 0.0 };
    (mo + slope * (k - mk), 1.5 * m0 + 0.5)
}

/// Walks lines parallel to `o.d` away from `o.outer` on both sides, follows
/// the pattern and fits the boundary through the hits. With `verify`, the
/// starting line itself must show the pattern near `o.outer`.
fn grow_from(
    scanner: &mut Scanner,
    img: &ImageView<'_>,
    o: Origin,
    verify: bool,
    params: &EdgeParams,
) -> Option<Boundary> {
    let n = perp(o.d);
    let m0 = o.module;
    let step = m0.max(1.0);
    let mut outer0 = o.outer;
    let mut pts = Vec::new();
    let mut modules = Vec::new();
    if verify {
        let (off, module) = probe(scanner, img, &o, outer0, 0.0, 3.0 * m0 + step, params)?;
        outer0 = at(outer0, o.d, off);
        pts.push(outer0);
        modules.push(module);
    } else {
        pts.push(outer0);
        modules.push(m0);
    }
    for side in [1.0f32, -1.0] {
        // Hits so far on this side as (line index, offset along the lines),
        // starting with the origin; the offset of the next line is predicted
        // by their least squares line, which keeps pixel quantization from
        // steering the walk.
        let mut seen: Vec<(f32, f32)> = vec![(0.0, 0.0)];
        let mut misses = 0usize;
        for k in 1..=MAX_STEPS {
            let center = at(outer0, n, side * step * k as f32);
            let (pred, tol) = predict(&seen, k as f32, step, m0);
            match probe(scanner, img, &o, center, pred, tol, params) {
                Some((off, module)) => {
                    misses = 0;
                    seen.push((k as f32, off));
                    pts.push(at(center, o.d, off));
                    modules.push(module);
                }
                None => {
                    misses += 1;
                    if misses >= MAX_MISSES {
                        break;
                    }
                }
            }
        }
    }
    if pts.len() < MIN_HITS {
        return None;
    }
    modules.sort_by(f32::total_cmp);
    let module_d = modules[modules.len() / 2];
    let (mut line, kept) = robust_fit(&pts, 0.4 * module_d + 0.5, MIN_HITS)?;
    let mut rows = perp(line.dir);
    let crossing = dot(rows, o.d);
    if crossing.abs() < MIN_CROSSING {
        return None;
    }
    if crossing < 0.0 {
        rows = (-rows.0, -rows.1);
    }
    let module = module_d * crossing.abs();
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for &p in &kept {
        let t = line.along(p);
        lo = lo.min(t);
        hi = hi.max(t);
    }
    if hi - lo < MIN_EXTENT * module {
        return None;
    }
    // Orient the boundary so that its normal is the row direction.
    if dot(perp(line.dir), rows) < 0.0 {
        line.dir = (-line.dir.0, -line.dir.1);
        (lo, hi) = (-hi, -lo);
    }
    Some(Boundary {
        kind: o.kind,
        inverted: o.inverted,
        line,
        rows,
        lo,
        hi,
        module,
    })
}

/// Grows a boundary from `o` over lines parallel to its scan direction, then
/// again over lines along the rows it found: a scan line that crosses the
/// pattern at a slant only sees the part of the boundary away from the
/// symbol's ends.
fn grow_chain(
    scanner: &mut Scanner,
    img: &ImageView<'_>,
    o: Origin,
    verify: bool,
    params: &EdgeParams,
) -> Option<Boundary> {
    let first = grow_from(scanner, img, o, verify, params)?;
    let second = grow_from(
        scanner,
        img,
        Origin {
            kind: first.kind,
            inverted: first.inverted,
            outer: first.line.point(0.5 * (first.lo + first.hi)),
            d: first.rows,
            module: first.module,
        },
        true,
        params,
    );
    Some(
        second
            .filter(|s| s.hi - s.lo >= first.hi - first.lo)
            .unwrap_or(first),
    )
}

/// The boundary of the pattern `seed` marks.
pub(crate) fn grow(
    scanner: &mut Scanner,
    img: &ImageView<'_>,
    seed: &Mark,
    params: &EdgeParams,
) -> Option<Boundary> {
    let d = unit(sub(seed.to, seed.from))?;
    let outer = match seed.kind {
        MarkKind::Start => seed.from,
        MarkKind::Stop => seed.to,
    };
    let o = Origin {
        kind: seed.kind,
        inverted: seed.inverted,
        outer,
        d,
        module: seed.module,
    };
    grow_chain(scanner, img, o, false, params)
}

/// The boundary on the other side of the symbol of `b`, with the outer edge
/// of its pattern at `outer` on a row line.
pub(crate) fn grow_other(
    scanner: &mut Scanner,
    img: &ImageView<'_>,
    b: &Boundary,
    outer: Point,
    params: &EdgeParams,
) -> Option<Boundary> {
    let kind = match b.kind {
        MarkKind::Start => MarkKind::Stop,
        MarkKind::Stop => MarkKind::Start,
    };
    let o = Origin {
        kind,
        inverted: b.inverted,
        outer,
        d: b.rows,
        module: b.module,
    };
    grow_chain(scanner, img, o, true, params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use breader_linear::stacked::Mark;

    fn mark(kind: MarkKind, x: f32, y: f32, module: f32) -> Mark {
        Mark {
            kind,
            inverted: false,
            from: Point::new(x, y),
            to: Point::new(x + 17.0 * module, y),
            module,
        }
    }

    #[test]
    fn crowded_seeds_come_first() {
        let mut marks = vec![mark(MarkKind::Start, 500.0, 10.0, 2.0)];
        for i in 0..5 {
            marks.push(mark(MarkKind::Start, 100.0, 20.0 * i as f32, 2.0));
        }
        marks.push(mark(MarkKind::Stop, 140.0, 30.0, 2.0));
        let order = rank_seeds(&marks, 3);
        assert_eq!(order.len(), 3);
        assert!(order.iter().all(|&i| (1..=5).contains(&i)));
    }

    #[test]
    fn modules_far_apart_do_not_support_each_other() {
        let marks = vec![
            mark(MarkKind::Start, 100.0, 0.0, 1.0),
            mark(MarkKind::Start, 100.0, 10.0, 4.0),
        ];
        let order = rank_seeds(&marks, 2);
        assert_eq!(order, [0, 1]);
    }
}
