//! PDF417 reader: seeds from the pattern marks of the linear scan, boundary,
//! row lines, grid votes and decoding.

use std::cell::RefCell;

use breader_core::{
    Content, Effort, FrameContext, ImageView, Pdf417Meta, Perspective, Point, Quad, Symbol,
    SymbolMeta, Symbology, SymbologyReader, SymbologySet,
};
use breader_linear::stacked::{EdgeParams, Mark, MarkKind, MarkSet, RawMark, scan_marks};

use crate::detect::{Boundary, grow, grow_other, rank_seeds};
use crate::geom::{Vec2, at, dot, perp, sub, unit};
use crate::grid::{Obs, Solved, central_lines, lines_with_rows, pitch_fit, solve, symbol_extent};
use crate::row::{Entry, RowRead, walk};
use crate::scan::Scanner;

/// PDF417 and compact PDF417 reader. It works on gray profiles and handles
/// both polarities itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct Pdf417Reader;

/// Per-effort search limits.
#[derive(Debug, Clone, Copy)]
struct Params {
    /// Seeds whose boundary is grown per frame.
    seeds: usize,
    /// Read the rows once more with smoothed edges when the first read fails.
    smooth_retry: bool,
    /// Row directions turned by these angles (degrees) away from the boundary's
    /// normal are tried when the straight read fails.
    tilts: &'static [f32],
}

impl Params {
    fn for_effort(e: Effort) -> Self {
        match e {
            Effort::Fast => Self {
                seeds: 6,
                smooth_retry: false,
                tilts: &[],
            },
            Effort::Balanced => Self {
                seeds: 12,
                smooth_retry: true,
                tilts: &[2.5, 5.0, 9.0],
            },
            Effort::Exhaustive => Self {
                seeds: 32,
                smooth_retry: true,
                tilts: &[2.0, 4.0, 7.0, 11.0, 16.0],
            },
        }
    }
}

/// Distance of the profile ends from the symbol, in modules.
const LEAD: f32 = 5.0;
/// Longest symbol in modules: start, indicators, 30 columns, stop.
const MAX_MODULES: f32 = 17.0 * 33.0 + 18.0;
/// Most row lines read for one symbol.
const MAX_LINES: usize = 700;
/// Row lines stay at most this far apart, in modules.
const LINE_SPACING: f32 = 1.25;
/// Line spacing of the dense pass for rows only a couple of modules high, in
/// modules.
const DENSE_SPACING: f32 = 0.15;
/// Dense lines with a row number needed before the row pitch is fitted.
const MIN_DENSE_ROWS: usize = 12;
/// Lines averaged per row in the dense pass, and the half width of their
/// spread in row pitches.
const MEAN_LINES: usize = 5;
const MEAN_HALF: f32 = 0.25;
/// Most lines read beyond each end of the lines of a pass, and the shortfall,
/// in row pitches, under which an end is left alone.
const MAX_EXTEND: usize = 250;
const EXTEND_SLACK: f32 = 0.75;
/// Lines with a row number needed before the ends are looked at.
const MIN_EXTEND_LINES: usize = 8;
/// Clean codewords on the sampled lines that make a tilted reading worth a
/// full pass.
const MIN_TILT_SCORE: usize = 24;
/// Sine of the largest tilt that is first tried as an axis-aligned symbol.
const AXIS_TOLERANCE: f32 = 0.02;
/// Boundary positions of warped lines are scaled to this many units, so that
/// the row fit sees steps well above its noise floor.
const WARP_UNITS: f32 = 100.0;

#[derive(Debug, Default)]
struct Work {
    scanner: Scanner,
    obs: Vec<Obs>,
    /// Opposite-side outer edges seen on the lines of the last pass.
    far: Vec<Point>,
    explored: Vec<Boundary>,
    quads: Vec<Quad>,
}

thread_local! {
    static WORK: RefCell<Work> = RefCell::new(Work::default());
}

/// Where the corners of a symbol are, as a function of the position along
/// its boundary.
#[derive(Debug, Clone, Copy)]
enum Frame {
    /// Rows run straight along `rows` from the boundary line.
    Straight {
        boundary: Boundary,
        rows: Vec2,
        /// Turn of the rows at the ends of the boundary, one way at the
        /// start of its extent and the other at the end (a fan).
        fan: f32,
        width: f32,
    },
    /// Corners of a perspective view: `h` maps (x, t / `WARP_UNITS`) with x
    /// from 0 (start side) to 1 (stop side).
    Warped { h: Perspective },
}

impl Frame {
    fn point(&self, x: f32, t: f32) -> Option<Point> {
        match self {
            Self::Straight {
                boundary: b,
                rows,
                fan,
                width,
            } => {
                let rows = fanned(*rows, *fan, b, t);
                let p = b.line.point(t);
                let left = match b.kind {
                    MarkKind::Start => p,
                    MarkKind::Stop => at(p, rows, -width),
                };
                Some(at(left, rows, x * width))
            }
            Self::Warped { h } => h.map(Point::new(x, t / WARP_UNITS)),
        }
    }
}

/// A symbol found in pass-image coordinates.
struct Found {
    solved: Solved,
    frame: Frame,
    /// Boundary position range of the lines read.
    span: (f32, f32),
}

/// One row line to sample.
#[derive(Debug, Clone, Copy)]
struct LineSpec {
    origin: Point,
    dir: Vec2,
    len: f32,
    /// Expected profile position of the start pattern's outer edge and of the
    /// stop pattern's outer edge.
    start_at: Option<f32>,
    stop_at: Option<f32>,
    module: f32,
    inverted: bool,
}

impl Work {
    fn explored(&self, seed: &Mark) -> bool {
        let p = match seed.kind {
            MarkKind::Start => seed.from,
            MarkKind::Stop => seed.to,
        };
        let slack = 3.0 * seed.module;
        self.explored.iter().any(|b| {
            b.kind == seed.kind
                && b.line.offset(p).abs() <= slack
                && (b.lo - slack..=b.hi + slack).contains(&b.line.along(p))
        })
    }

    /// Reads the row lines along `b`, perpendicular to it.
    fn read_straight(
        &mut self,
        img: &ImageView<'_>,
        b: &Boundary,
        edge: &EdgeParams,
        snap: bool,
        (tilt, fan): (f32, f32),
        dense: bool,
    ) -> Option<Found> {
        let m = b.module;
        let base = turn(b.rows, tilt);
        // Rows along the pixel grid are read on whole pixel lines, which is
        // the only way to read rows that are a pixel high.
        let axis = if snap && fan == 0.0 {
            axis_aligned(base)
        } else {
            None
        };
        let spacing = if dense { DENSE_SPACING } else { LINE_SPACING };
        let step = if axis.is_some() {
            if m < 2.5 {
                1.0
            } else {
                (spacing * m).round().max(1.0)
            }
        } else {
            (spacing * m).max(0.8)
        };
        let rows = axis.unwrap_or(base);
        let (lo, hi) = (b.lo - 0.5 * step, b.hi + 0.5 * step);
        let count = (((hi - lo) / step).ceil() as usize + 1).min(MAX_LINES);
        let step = if count > 1 {
            (hi - lo) / (count - 1) as f32
        } else {
            step
        };
        // Middle first, so the width of the symbol is known early.
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by_key(|&k| (2 * k).abs_diff(count - 1));
        let mut reach = 1.1 * MAX_MODULES * m;
        let lead = LEAD * m;
        self.obs.clear();
        self.far.clear();
        let mut width = 0.0f32;
        for k in order {
            let t = lo + step * k as f32;
            let mut q = b.line.point(t);
            if axis.is_some() {
                q = Point::new(q.x.round(), q.y.round());
            }
            let spec = row_spec(b, fanned(rows, fan, b, t), q, reach);
            let Some((read, far)) = self.read_line(img, &spec, edge) else {
                continue;
            };
            if read.stop.is_some() && width == 0.0 {
                width = read.to - read.from;
                reach = 1.15 * width + lead;
            }
            if let Some(p) = far {
                self.far.push(p);
            }
            self.obs.push(Obs { t, read });
        }
        self.obs.sort_by(|a, c| a.t.total_cmp(&c.t));
        if self.obs.len() < 3 {
            return None;
        }
        let solved = if dense {
            if lines_with_rows(&self.obs) < MIN_DENSE_ROWS {
                return None;
            }
            let fit = pitch_fit(&self.obs)?;
            // The boundary may end rows short of the symbol.
            if self.extend(img, b, edge, (rows, fan), (step, reach), Some(fit)) {
                self.obs.sort_by(|a, c| a.t.total_cmp(&c.t));
            }
            let fit = pitch_fit(&self.obs).unwrap_or(fit);
            // Mean profiles per row and the single lines that lie in a row
            // vote together: neither reads every row alone.
            let mut all = self.mean_rows(img, b, edge, (rows, fan), fit, reach);
            all.extend(central_lines(&self.obs).unwrap_or_default());
            all.sort_by(|a, c| a.t.total_cmp(&c.t));
            solve(&all)?
        } else {
            match solve(&self.obs) {
                Some(s) => s,
                None => {
                    if !self.extend(img, b, edge, (rows, fan), (step, reach), None) {
                        return None;
                    }
                    self.obs.sort_by(|a, c| a.t.total_cmp(&c.t));
                    solve(&self.obs)?
                }
            }
        };
        if width == 0.0 {
            let mean_module: f32 =
                self.obs.iter().map(|o| o.read.module).sum::<f32>() / self.obs.len() as f32;
            let units = 17 * (solved.decoded.layout.cols + 2) + 1;
            width = mean_module * units as f32;
        }
        Some(Found {
            solved,
            frame: Frame::Straight {
                boundary: *b,
                rows,
                fan,
                width,
            },
            span: (b.lo, b.hi),
        })
    }

    /// Reads more lines when the rows the indicators number run beyond the
    /// ends of the lines read so far (a boundary that stops short, rows that
    /// blur or curve at the ends of the symbol). Returns whether lines were
    /// added.
    fn extend(
        &mut self,
        img: &ImageView<'_>,
        b: &Boundary,
        edge: &EdgeParams,
        (rows, fan): (Vec2, f32),
        (step, reach): (f32, f32),
        fit: Option<(f32, f32)>,
    ) -> bool {
        // Most failing seeds are not symbols: leave them before allocating.
        if lines_with_rows(&self.obs) < MIN_EXTEND_LINES {
            return false;
        }
        let Some((from, to, pitch)) = symbol_extent(&self.obs, fit) else {
            return false;
        };
        let (have_lo, have_hi) = match (self.obs.first(), self.obs.last()) {
            (Some(a), Some(c)) => (a.t, c.t),
            _ => return false,
        };
        // Whole rows are worth reading for, not a sliver of one.
        let slack = EXTEND_SLACK * pitch;
        let mut added = 0usize;
        let mut ts = Vec::new();
        if from < have_lo - slack {
            let n = (((have_lo - from) / step).ceil() as usize).min(MAX_EXTEND);
            ts.extend((1..=n).map(|k| have_lo - step * k as f32));
        }
        if to > have_hi + slack {
            let n = (((to - have_hi) / step).ceil() as usize).min(MAX_EXTEND);
            ts.extend((1..=n).map(|k| have_hi + step * k as f32));
        }
        for t in ts {
            let q = b.line.point(t);
            let spec = row_spec(b, fanned(rows, fan, b, t), q, reach);
            if let Some((read, _)) = self.read_line(img, &spec, edge) {
                self.obs.push(Obs { t, read });
                added += 1;
            }
        }
        added > 0
    }

    /// One observation per row of a dense pass whose row pitch is known:
    /// the mean profile of a few lines around the row's centre.
    fn mean_rows(
        &mut self,
        img: &ImageView<'_>,
        b: &Boundary,
        edge: &EdgeParams,
        (rows, fan): (Vec2, f32),
        (slope, offset): (f32, f32),
        reach: f32,
    ) -> Vec<Obs> {
        let (t_lo, t_hi) = match (self.obs.first(), self.obs.last()) {
            (Some(a), Some(c)) => (a.t, c.t),
            _ => return Vec::new(),
        };
        let (r_a, r_b) = (slope * t_lo + offset, slope * t_hi + offset);
        let (first, last) = (
            r_a.min(r_b).ceil().max(0.0) as usize,
            r_a.max(r_b).floor() as usize,
        );
        let pitch = 1.0 / slope.abs();
        let n = MEAN_LINES;
        let half = MEAN_HALF;
        let mut out = Vec::new();
        let mut origins = Vec::with_capacity(n);
        for r in first..=last {
            let t = (r as f32 - offset) / slope;
            let q = b.line.point(t);
            let spec = row_spec(b, fanned(rows, fan, b, t), q, reach);
            origins.clear();
            for j in 0..n {
                let d = if n > 1 {
                    (j as f32 / (n - 1) as f32 - 0.5) * 2.0 * half * pitch
                } else {
                    0.0
                };
                let p = b.line.point(t + d);
                origins.push(Point::new(
                    spec.origin.x + p.x - q.x,
                    spec.origin.y + p.y - q.y,
                ));
            }
            if let Some((read, _)) = self.read_mean(img, &spec, &origins, edge) {
                out.push(Obs { t, read });
            }
        }
        out
    }

    /// How well rows along `rows` read: the clean codewords on a few lines.
    fn score(
        &mut self,
        img: &ImageView<'_>,
        b: &Boundary,
        (rows, fan): (Vec2, f32),
        edge: &EdgeParams,
    ) -> usize {
        const LINES: usize = 9;
        let reach = 1.1 * MAX_MODULES * b.module;
        let mut total = 0;
        for k in 0..LINES {
            let t = b.lo + (b.hi - b.lo) * (k as f32 + 0.5) / LINES as f32;
            let spec = row_spec(b, fanned(rows, fan, b, t), b.line.point(t), reach);
            if let Some((read, _)) = self.read_line(img, &spec, edge) {
                total += read.cw[..read.len].iter().filter(|c| c.sure).count();
            }
        }
        total
    }

    /// Reads rows along a direction turned away from the boundary's normal,
    /// the best of `tilts` (degrees, tried on both sides) by `score`.
    fn read_tilted(
        &mut self,
        img: &ImageView<'_>,
        b: &Boundary,
        edge: &EdgeParams,
        tilts: &[f32],
    ) -> Option<Found> {
        let mut best = (0usize, 0.0f32, 0.0f32);
        for &deg in tilts {
            for sign in [1.0f32, -1.0] {
                let angle = sign * deg.to_radians();
                // A turn of the whole row direction, then a fan.
                for (tilt, fan) in [(angle, 0.0), (0.0, angle)] {
                    let score = self.score(img, b, (turn(b.rows, tilt), fan), edge);
                    if score > best.0 {
                        best = (score, tilt, fan);
                    }
                }
            }
        }
        (best.0 >= MIN_TILT_SCORE)
            .then(|| self.read_straight(img, b, edge, false, (best.1, best.2), false))
            .flatten()
    }

    /// Reads rows between the start and stop boundaries, along the lines the
    /// perspective through their ends gives.
    fn read_warped(
        &mut self,
        img: &ImageView<'_>,
        start: &Boundary,
        stop: &Boundary,
        edge: &EdgeParams,
    ) -> Option<Found> {
        let (top_a, bot_a) = (start.line.point(start.hi), start.line.point(start.lo));
        let (top_b, bot_b) = (stop.line.point(stop.hi), stop.line.point(stop.lo));
        let flip = (top_a.distance(top_b) + bot_a.distance(bot_b))
            > (top_a.distance(bot_b) + bot_a.distance(top_b));
        let (top_b, bot_b) = if flip { (bot_b, top_b) } else { (top_b, bot_b) };
        let h = Perspective::square_to_quad([top_a, top_b, bot_b, bot_a])?;
        let m = 0.5 * (start.module + stop.module);
        let height = 0.5 * (top_a.distance(bot_a) + top_b.distance(bot_b));
        let step = (LINE_SPACING * m).max(0.8);
        let count = ((height / step).ceil() as usize + 2).clamp(3, MAX_LINES);
        let lead = LEAD * m;
        self.obs.clear();
        self.far.clear();
        for k in 0..count {
            let f = (k as f32 + 0.5) / count as f32;
            let (Some(a), Some(b)) = (h.map(Point::new(0.0, f)), h.map(Point::new(1.0, f))) else {
                return None;
            };
            let Some(dir) = unit(sub(b, a)) else {
                continue;
            };
            let span = a.distance(b);
            let spec = LineSpec {
                origin: at(a, dir, -lead),
                dir,
                len: span + 2.0 * lead,
                start_at: Some(lead),
                stop_at: Some(span + lead),
                module: m,
                inverted: start.inverted,
            };
            if let Some((read, _)) = self.read_line(img, &spec, edge) {
                self.obs.push(Obs {
                    t: f * WARP_UNITS,
                    read,
                });
            }
        }
        if self.obs.len() < 3 {
            return None;
        }
        let solved = solve(&self.obs)?;
        Some(Found {
            solved,
            frame: Frame::Warped { h },
            span: (0.0, WARP_UNITS),
        })
    }

    /// One row line: the codewords read on it and, when the pattern on the
    /// far side was seen, its outer edge in image coordinates.
    fn read_line(
        &mut self,
        img: &ImageView<'_>,
        spec: &LineSpec,
        edge: &EdgeParams,
    ) -> Option<(RowRead, Option<Point>)> {
        self.read_mean(img, spec, &[spec.origin], edge)
    }

    /// As `read_line` on the mean profile of parallel lines starting at
    /// `origins`.
    fn read_mean(
        &mut self,
        img: &ImageView<'_>,
        spec: &LineSpec,
        origins: &[Point],
        edge: &EdgeParams,
    ) -> Option<(RowRead, Option<Point>)> {
        let m = spec.module;
        let s = &mut self.scanner;
        if !s.load_mean(img, origins, spec.dir, (0.0, spec.len), spec.inverted, edge) {
            return None;
        }
        let ok =
            |k: &&RawMark| k.to > k.from && !k.inverted && k.module > 0.5 * m && k.module < 2.0 * m;
        let tol = 3.5 * m;
        let nearest = |marks: &[RawMark], kind: MarkKind, at: f32, end: bool| {
            marks
                .iter()
                .filter(ok)
                .filter(|k| k.kind == kind)
                .map(|k| (k, (s.t_of(if end { k.to } else { k.from }) - at).abs()))
                .filter(|&(_, d)| d <= tol)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(k, _)| *k)
        };
        let stop = spec
            .stop_at
            .and_then(|at| nearest(&s.marks, MarkKind::Stop, at, true));
        let start = match (spec.start_at, &stop) {
            (Some(at), _) => nearest(&s.marks, MarkKind::Start, at, false)
                .map(|k| Entry::after(&k))
                .or_else(|| {
                    // The pattern itself is too worn to match: the codewords
                    // are expected 17 modules after it.
                    let target = at + 17.0 * m;
                    (0..s.runs.len())
                        .filter(|&i| {
                            s.runs.is_bar(i) && (s.t_of(s.runs.start(i)) - target).abs() <= 0.7 * m
                        })
                        .min_by(|&a, &b| {
                            let d = |i: usize| (s.t_of(s.runs.start(i)) - target).abs();
                            d(a).total_cmp(&d(b))
                        })
                        .map(|first| Entry {
                            first,
                            module: m,
                            from: s.pos_of(at),
                            to: s.pos_of(target),
                        })
                }),
            (None, Some(stop)) => s
                .marks
                .iter()
                .filter(ok)
                .filter(|k| k.kind == MarkKind::Start && k.to < stop.from - 17.0 * m)
                .max_by(|x, y| x.from.total_cmp(&y.from))
                .map(Entry::after),
            (None, None) => None,
        };
        let start = start?;
        let mut read = RowRead::default();
        if !walk(&s.runs, &s.marks, &start, &mut read) {
            return None;
        }
        // The far side: the stop pattern's outer edge when reading from the
        // start, the start pattern's when only the stop was anchored.
        let far = if spec.start_at.is_some() {
            read.stop.map(|_| read.to)
        } else {
            Some(read.from)
        }
        .map(|p| at(spec.origin, spec.dir, s.t_of(p)));
        Some((read, far))
    }

    /// Tries to read the symbol behind boundary `b`: straight lines first,
    /// then, when the first pass saw the far side, lines between both
    /// boundaries.
    fn read_symbol(
        &mut self,
        img: &ImageView<'_>,
        b: &Boundary,
        edge: &EdgeParams,
    ) -> Option<Found> {
        if axis_aligned(b.rows).is_some()
            && let Some(f) = self.read_straight(img, b, edge, true, (0.0, 0.0), false)
        {
            return Some(f);
        }
        if let Some(f) = self.read_straight(img, b, edge, false, (0.0, 0.0), false) {
            return Some(f);
        }
        self.read_far_side(img, b, edge)
    }

    /// The warped read from the far side the straight pass saw.
    fn read_far_side(
        &mut self,
        img: &ImageView<'_>,
        b: &Boundary,
        edge: &EdgeParams,
    ) -> Option<Found> {
        if self.far.len() < 4 {
            return None;
        }
        let mut far = std::mem::take(&mut self.far);
        far.sort_by(|a, c| a.x.total_cmp(&c.x).then(a.y.total_cmp(&c.y)));
        let seed = far[far.len() / 2];
        let other = grow_other(&mut self.scanner, img, b, seed, edge)?;
        let (start, stop) = match b.kind {
            MarkKind::Start => (b, &other),
            MarkKind::Stop => (&other, b),
        };
        self.read_warped(img, start, stop, edge)
    }
}

/// The row line through boundary point `q` along `rows`, `reach` samples
/// away from the boundary on the symbol's side.
fn row_spec(b: &Boundary, rows: Vec2, q: Point, reach: f32) -> LineSpec {
    let lead = LEAD * b.module;
    match b.kind {
        MarkKind::Start => LineSpec {
            origin: at(q, rows, -lead),
            dir: rows,
            len: lead + reach,
            start_at: Some(lead),
            stop_at: None,
            module: b.module,
            inverted: b.inverted,
        },
        MarkKind::Stop => LineSpec {
            origin: at(q, rows, -reach),
            dir: rows,
            len: reach + lead,
            start_at: None,
            stop_at: Some(reach),
            module: b.module,
            inverted: b.inverted,
        },
    }
}

/// The row direction `rows` at boundary position `t` when the rows fan out
/// by `fan` radians at each end of the boundary's extent.
fn fanned(rows: Vec2, fan: f32, b: &Boundary, t: f32) -> Vec2 {
    if fan == 0.0 {
        return rows;
    }
    let u = (2.0 * t - (b.lo + b.hi)) / (b.hi - b.lo).max(1e-3);
    turn(rows, fan * u.clamp(-1.5, 1.5))
}

/// `v` turned by `angle` radians.
fn turn(v: Vec2, angle: f32) -> Vec2 {
    let (s, c) = angle.sin_cos();
    (c * v.0 - s * v.1, s * v.0 + c * v.1)
}

/// The exact axis direction of `rows` when it lies within a degree of one.
fn axis_aligned(rows: Vec2) -> Option<Vec2> {
    if rows.1.abs() < AXIS_TOLERANCE {
        Some((rows.0.signum(), 0.0))
    } else if rows.0.abs() < AXIS_TOLERANCE {
        Some((0.0, rows.1.signum()))
    } else {
        None
    }
}

/// Whether a start boundary and a stop boundary can be the two ends of the
/// rows of one symbol: the stop side lies ahead along the rows, the rows
/// directions agree and so do heights and modules, within the limits of a
/// perspective view.
fn pairable(start: &Boundary, stop: &Boundary) -> bool {
    if start.kind != MarkKind::Start
        || stop.kind != MarkKind::Stop
        || start.inverted != stop.inverted
    {
        return false;
    }
    let mid = |b: &Boundary| b.line.point(0.5 * (b.lo + b.hi));
    let (a, b) = (mid(start), mid(stop));
    let delta = sub(b, a);
    let along = dot(delta, start.rows);
    let across = dot(delta, perp(start.rows)).abs();
    let m = 0.5 * (start.module + stop.module);
    let (hs, ht) = (start.hi - start.lo, stop.hi - stop.lo);
    let ratio = |x: f32, y: f32| x.max(y) / x.min(y).max(1e-3);
    along > 40.0 * m
        && along < 1.3 * MAX_MODULES * m
        && dot(start.rows, stop.rows) > 0.8
        && ratio(hs, ht) < 2.5
        && ratio(start.module, stop.module) < 2.5
        && across <= 0.5 * (hs + ht) + 0.6 * along
}

/// Marks that may belong to the far end of the symbol behind `b`, best first.
fn partners(marks: &[Mark], b: &Boundary, limit: usize) -> Vec<usize> {
    let mid = b.line.point(0.5 * (b.lo + b.hi));
    let height = b.hi - b.lo;
    let sign = if b.kind == MarkKind::Start { 1.0 } else { -1.0 };
    let mut found: Vec<(f32, usize)> = marks
        .iter()
        .enumerate()
        .filter(|(_, m)| m.kind != b.kind && m.inverted == b.inverted)
        .filter_map(|(i, m)| {
            let outer = if m.kind == MarkKind::Stop {
                m.to
            } else {
                m.from
            };
            let d = sub(outer, mid);
            let along = sign * dot(d, b.rows);
            let across = dot(d, perp(b.rows)).abs();
            let ratio = m.module.max(b.module) / m.module.min(b.module);
            (along > 40.0 * b.module
                && along < 1.3 * MAX_MODULES * b.module
                && ratio < 2.5
                && across <= height + 0.5 * along)
                .then_some((across, i))
        })
        .collect();
    found.sort_by(|a, c| a.0.total_cmp(&c.0));
    found.into_iter().take(limit).map(|(_, i)| i).collect()
}

fn centre(m: &Mark) -> Point {
    Point::new((m.from.x + m.to.x) / 2.0, (m.from.y + m.to.y) / 2.0)
}

/// The corners of a found symbol in pass-image coordinates, in symbol order.
fn corners(f: &Found) -> Option<[Point; 4]> {
    let layout = f.solved.decoded.layout;
    let (t0, t1) = match f.solved.fit {
        Some((slope, offset)) if slope.abs() > 1e-4 => (
            (-0.5 - offset) / slope,
            (layout.rows as f32 - 0.5 - offset) / slope,
        ),
        _ => (f.span.1, f.span.0),
    };
    let q = [
        f.frame.point(0.0, t0)?,
        f.frame.point(1.0, t0)?,
        f.frame.point(1.0, t1)?,
        f.frame.point(0.0, t1)?,
    ];
    q.iter()
        .all(|p| p.x.is_finite() && p.y.is_finite())
        .then_some(q)
}

fn build_symbol(f: &Found, scale: (f32, f32)) -> Option<Symbol> {
    let dec = &f.solved.decoded;
    let pts = corners(f)?.map(|p| Point::new(p.x * scale.0, p.y * scale.1));
    let quad = Quad::new(pts);
    Some(Symbol {
        symbology: Symbology::Pdf417,
        data: dec.data.clone(),
        text: None,
        segments: dec.segments.clone(),
        content: Content::Unparsed,
        quad,
        orientation_deg: quad.orientation_deg(),
        aim_id: dec.aim_id,
        meta: SymbolMeta::Pdf417(Pdf417Meta {
            rows: dec.layout.rows as u16,
            cols: dec.layout.cols as u16,
            ec_level: dec.layout.ec_level,
            errors_corrected: dec.errors_corrected,
            erasures: dec.erasures,
            compact: f.solved.compact,
            macro_pdf417: dec.macro_pdf417.clone(),
            reader_init: dec.reader_init,
        }),
        quality: 1.0 / (1.0 + 0.1 * (dec.errors_corrected + dec.erasures) as f32),
    })
}

fn scan_with(work: &mut Work, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
    let effort = ctx.options().effort;
    let params = Params::for_effort(effort);
    let shared = ctx.shared::<MarkSet>();
    let scanned = shared.scanned;
    let mut marks = std::mem::take(&mut shared.marks);
    let scale = ctx.scale();
    let img = ctx.image();
    if !scanned {
        let mut set = MarkSet::default();
        scan_marks(&img, effort, &mut set);
        marks = set.marks;
    }
    if marks.is_empty() {
        return;
    }
    work.explored.clear();
    work.quads.clear();
    work.quads.extend(
        out.iter()
            .filter(|s| s.symbology == Symbology::Pdf417)
            .map(|s| {
                let mut q = s.quad;
                for c in &mut q.corners {
                    *c = Point::new(c.x / scale.0, c.y / scale.1);
                }
                q
            }),
    );
    let mut attempts = 0;
    let fine = EdgeParams::fine();
    let smooth = EdgeParams::smoothed();
    for idx in rank_seeds(&marks, usize::MAX) {
        let seed = &marks[idx];
        if work.quads.iter().any(|q| q.contains(centre(seed))) || work.explored(seed) {
            continue;
        }
        if attempts >= params.seeds {
            break;
        }
        attempts += 1;
        let Some(boundary) = grow(&mut work.scanner, &img, seed, &fine) else {
            continue;
        };
        let mut found = work.read_symbol(&img, &boundary, &fine);
        if found.is_none() && params.smooth_retry {
            found = work.read_symbol(&img, &boundary, &smooth);
        }
        // Rows only a couple of modules high (excise stamps): dense lines,
        // voted at the row pitch. Not at Fast, where it would only cost.
        if found.is_none() && params.smooth_retry {
            found = work.read_straight(&img, &boundary, &smooth, false, (0.0, 0.0), true);
        }
        // The other end: grown from an earlier seed, or from a pattern that
        // lies where it should.
        if found.is_none() {
            let mut others: Vec<Boundary> = work.explored.clone();
            for i in partners(&marks, &boundary, 2) {
                if let Some(o) = grow(&mut work.scanner, &img, &marks[i], &fine) {
                    others.push(o);
                }
            }
            for other in &others {
                let (start, stop) = match boundary.kind {
                    MarkKind::Start => (&boundary, other),
                    MarkKind::Stop => (other, &boundary),
                };
                if !pairable(start, stop) {
                    continue;
                }
                found = work.read_warped(&img, start, stop, &fine);
                if found.is_none() && params.smooth_retry {
                    found = work.read_warped(&img, start, stop, &smooth);
                }
                if found.is_some() {
                    break;
                }
            }
        }
        if found.is_none() && !params.tilts.is_empty() {
            found = work.read_tilted(&img, &boundary, &fine, params.tilts);
        }
        work.explored.push(boundary);
        let Some(found) = found else {
            continue;
        };
        let Some(sym) = build_symbol(&found, scale) else {
            continue;
        };
        if let Some(q) = corners(&found) {
            work.quads.push(Quad::new(q));
        }
        // The same symbol read from another of its patterns.
        let copy = out.iter().any(|o| {
            o.symbology == Symbology::Pdf417
                && o.data == sym.data
                && o.quad.center().distance(sym.quad.center())
                    < 0.5 * o.quad.corners[0].distance(o.quad.corners[2])
        });
        if !copy {
            out.push(sym);
        }
    }
}

impl SymbologyReader for Pdf417Reader {
    fn name(&self) -> &'static str {
        "pdf417"
    }

    fn symbologies(&self) -> SymbologySet {
        SymbologySet::EMPTY.with(Symbology::Pdf417)
    }

    fn uses_binary(&self) -> bool {
        false
    }

    fn handles_inversion(&self) -> bool {
        true
    }

    fn scan(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
        // Like the linear reader: one scan of the original image.
        if ctx.pass().preprocessor.is_some() || ctx.pass().inverted {
            return;
        }
        if !ctx.options().symbologies.contains(Symbology::Pdf417) {
            return;
        }
        WORK.with(|cell| match cell.try_borrow_mut() {
            Ok(mut work) => scan_with(&mut work, ctx, out),
            Err(_) => scan_with(&mut Work::default(), ctx, out),
        });
    }
}

#[cfg(test)]
mod tests;
