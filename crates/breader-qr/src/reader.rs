//! QR reader orchestration: finders, triplets, sampling and decoding.

use breader_core::{
    BitMatrix, Content, FrameContext, Homography, Plane, Point, QrMeta, Quad, Symbol, SymbolMeta,
    Symbology, SymbologyReader, SymbologySet,
};

use crate::decode::{DecodedQr, decode_grid, read_version};
use crate::detect::{FinderSearch, Triplet, candidate_dims, find_alignment};
use crate::sample::{
    Gray, Mapper, finders_plausible, fourth_point, module_axes, module_size, pattern_cells,
    predict_alignment, refine_finder, sample_grid, sample_grid_local, timing_plausible,
    timing_warp, transform,
};
use crate::spec::{dimension, version_for_dimension};

#[derive(Debug, Clone, Copy, Default)]
pub struct QrReader;

/// Finder size ratio above which a failed alignment search is repeated in a
/// much wider window (strong perspective moves the pattern far).
const WIDE_SEARCH_RATIO: f32 = 1.2;
/// Half-widths (in boundaries) of the moving average applied to the pitch
/// corrections measured on the timing patterns; the raw values carry the
/// jitter of the binarized edges.
const WARP_SMOOTHING: [usize; 2] = [2, 5];
/// Failed alignment searches after which a fit without any found pattern
/// is abandoned.
const MAX_MISSES: usize = 2;

/// Per-effort search limits.
#[derive(Debug, Clone, Copy)]
struct Params {
    row_step: usize,
    max_finders: usize,
    /// Detected triplets that may fail to decode before the pass gives up.
    max_failures: usize,
    /// Same for triplets whose third finder was inferred from a timing pattern.
    max_recovered_failures: usize,
}

impl Params {
    fn for_effort(e: breader_core::Effort) -> Self {
        match e {
            breader_core::Effort::Fast => Self {
                row_step: 3,
                max_finders: 320,
                max_failures: 3,
                max_recovered_failures: 4,
            },
            breader_core::Effort::Balanced => Self {
                row_step: 2,
                max_finders: 320,
                max_failures: 4,
                max_recovered_failures: 6,
            },
            breader_core::Effort::Exhaustive => Self {
                row_step: 1,
                max_finders: 640,
                max_failures: 24,
                max_recovered_failures: 24,
            },
        }
    }
}

/// Buffers reused across candidates within one scan.
#[derive(Debug, Default)]
struct Work<'g> {
    gray: Option<Gray<'g>>,
    search: FinderSearch,
    grid: BitMatrix,
    runs: Vec<u32>,
    pts: Vec<(Point, Point)>,
    cells: Vec<(Point, bool)>,
    /// Grids that passed the plausibility checks since the last `try_triplet`.
    plausible: u32,
}

struct Found {
    decoded: DecodedQr,
    h: Mapper,
    dim: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// Fit over every alignment pattern found progressively.
    Refined,
    /// Homography through the bottom-right alignment pattern.
    Corner,
    Affine,
}

impl Work<'_> {
    /// Decodes a triplet; failures are retried once with finder centers
    /// refined against the finder template.
    fn try_triplet(&mut self, bin: Plane<'_>, t: &Triplet) -> Option<Found> {
        self.plausible = 0;
        if let Some(f) = self.try_refined(bin, t) {
            return Some(f);
        }
        // Perspective correction of the fourth corner is a guess from the
        // finder sizes; fall back to the plain parallelogram when it moved it.
        let module = (t.tl.module + t.tr.module + t.bl.module) / 3.0;
        let plain = Triplet {
            parallel: true,
            ..*t
        };
        if self.plausible > 0 && fourth_point(t).distance(fourth_point(&plain)) > 0.7 * module {
            return self.try_refined(bin, &plain);
        }
        None
    }

    fn try_refined(&mut self, bin: Plane<'_>, t: &Triplet) -> Option<Found> {
        if !t.synthetic
            && let Some(f) = self.try_dims(bin, t)
        {
            return Some(f);
        }
        // Refining finder centers only helps when a grid looked plausible.
        if !t.synthetic && self.plausible == 0 {
            return None;
        }
        let module = (t.tl.module + t.tr.module + t.bl.module) / 3.0;
        if module < 1.8 {
            return None;
        }
        let dim = if t.dim_hint != 0 {
            t.dim_hint
        } else {
            candidate_dims(t.dim).0[0]
        };
        let (ex, ey) = module_axes(t, dim);
        let mut r = *t;
        for f in [&mut r.tl, &mut r.tr, &mut r.bl] {
            f.pos = refine_finder(bin, f.pos, ex, ey);
        }
        self.try_dims(bin, &r)
    }

    /// Tries the dimension candidates of a triplet until one decodes.
    fn try_dims(&mut self, bin: Plane<'_>, t: &Triplet) -> Option<Found> {
        let (cands, n) = candidate_dims(t.dim);
        let mut queue = [0usize; 5];
        let mut len = 0;
        if t.dim_hint != 0 {
            queue[0] = t.dim_hint;
            len = 1;
        }
        for &c in &cands[..n] {
            if !queue[..len].contains(&c) {
                queue[len] = c;
                len += 1;
            }
        }
        let mut tried = [0usize; 5];
        let mut ntried = 0;
        let mut head = 0;
        while head < len && ntried < 4 {
            let dim = queue[head];
            head += 1;
            if tried[..ntried].contains(&dim) {
                continue;
            }
            tried[ntried] = dim;
            ntried += 1;
            // Cheap plausibility tests (timing patterns for small symbols, finder
            // blocks for all) come before the far costlier alignment search.
            let implausible = transform(t, dim, None, Point::default()).is_some_and(|h| {
                (dim != t.dim_hint && dim < 45 && !timing_plausible(bin, &h, dim))
                    || !finders_plausible(bin, &h, dim)
            });
            let mut first = true;
            let mut refined = false;
            // The first mapping that sampled a plausible grid, kept for the
            // retries below.
            let mut retry: Option<(Mapper, f32)> = None;
            for variant in [Variant::Refined, Variant::Corner, Variant::Affine] {
                if implausible {
                    break;
                }
                if refined && variant != Variant::Refined {
                    // A fit over every found pattern supersedes the cruder guesses.
                    break;
                }
                let h = match variant {
                    Variant::Refined if dim >= 45 => {
                        let h = self.refine(bin, t, dim);
                        refined = h.is_some();
                        h
                    }
                    Variant::Corner if dim >= 25 => self
                        .alignment(bin, t, dim)
                        .and_then(|a| transform(t, dim, Some(a), Point::default())),
                    Variant::Affine => transform(t, dim, None, Point::default()),
                    _ => None,
                };
                let Some(h) = h else {
                    continue;
                };
                let Some(m) = module_size(&h, dim) else {
                    continue;
                };
                if !sample_grid(bin, &h, dim, m, &mut self.grid) {
                    continue;
                }
                self.plausible += 1;
                if first
                    && dim >= 45
                    && let Some(v) = read_version(&self.grid)
                {
                    let vdim = dimension(v);
                    if vdim != dim {
                        if len < queue.len() && !queue[..len].contains(&vdim) {
                            // Examine the dimension the version info names next.
                            queue.copy_within(head..len, head + 1);
                            queue[head] = vdim;
                            len += 1;
                        }
                        retry = None;
                        break;
                    }
                }
                first = false;
                if let Ok(decoded) = decode_grid(&self.grid)
                    && version_for_dimension(dim) == Some(decoded.version)
                {
                    return Some(Found { decoded, h, dim });
                }
                if retry.is_none() {
                    retry = Some((h, m));
                }
            }
            if let Some((h, m)) = retry
                && let Some(found) = self.resample(bin, dim, &h, m)
            {
                return Some(found);
            }
            if dim <= 41
                && let Some(found) = self.perturbed(bin, t, dim)
            {
                return Some(found);
            }
        }
        None
    }

    /// Decodes the sampled grid when it is a valid symbol of `dim`.
    fn decode(&self, dim: usize) -> Option<DecodedQr> {
        decode_grid(&self.grid)
            .ok()
            .filter(|d| version_for_dimension(dim) == Some(d.version))
    }

    /// Retries a grid whose Reed-Solomon failed with sampling that follows
    /// the print more closely than the homography: modules compared with
    /// their neighbourhood in the luma image, then the module pitch measured
    /// on the finders and timing patterns (at two smoothing widths, each
    /// applied twice since the first correction moves the lines measured).
    fn resample(&mut self, bin: Plane<'_>, dim: usize, h: &Mapper, m: f32) -> Option<Found> {
        if let Some(gray) = self.gray
            && sample_grid_local(gray, bin.inverted(), h, dim, &mut self.grid)
            && let Some(decoded) = self.decode(dim)
        {
            return Some(Found {
                decoded,
                h: h.clone(),
                dim,
            });
        }
        for smooth in WARP_SMOOTHING {
            let mut w = h.clone();
            for _ in 0..2 {
                let Some(next) = timing_warp(bin, &w, dim, smooth) else {
                    break;
                };
                w = next;
                if sample_grid(bin, &w, dim, m, &mut self.grid)
                    && let Some(decoded) = self.decode(dim)
                {
                    return Some(Found { decoded, h: w, dim });
                }
            }
        }
        None
    }

    /// Small symbols have no (or one) alignment pattern to pin down the
    /// fourth corner under perspective; Reed-Solomon arbitrates between
    /// shifted guesses of that point.
    fn perturbed(&mut self, bin: Plane<'_>, t: &Triplet, dim: usize) -> Option<Found> {
        const STEPS: [(f32, f32); 16] = [
            (1.0, 1.0),
            (-1.0, -1.0),
            (1.0, 0.0),
            (0.0, 1.0),
            (-1.0, 0.0),
            (0.0, -1.0),
            (1.0, -1.0),
            (-1.0, 1.0),
            (2.0, 2.0),
            (-2.0, -2.0),
            (2.0, 0.0),
            (0.0, 2.0),
            (-2.0, 0.0),
            (0.0, -2.0),
            (2.0, -2.0),
            (-2.0, 2.0),
        ];
        let base = if dim >= 25 {
            Some(predict_alignment(t, dim))
        } else {
            None
        };
        let (ex, ey) = module_axes(t, dim);
        for (sx, sy) in STEPS {
            let shift = Point::new(sx * ex.x + sy * ey.x, sx * ex.y + sy * ey.y);
            let Some(h) = transform(t, dim, base, shift) else {
                continue;
            };
            let Some(m) = module_size(&h, dim) else {
                continue;
            };
            if !sample_grid(bin, &h, dim, m, &mut self.grid) {
                continue;
            }
            self.plausible += 1;
            if let Ok(decoded) = decode_grid(&self.grid)
                && version_for_dimension(dim) == Some(decoded.version)
            {
                return Some(Found { decoded, h, dim });
            }
        }
        None
    }

    fn alignment(&mut self, bin: Plane<'_>, t: &Triplet, dim: usize) -> Option<Point> {
        let pred = predict_alignment(t, dim);
        let module = (t.tl.module + t.tr.module + t.bl.module) / 3.0;
        let axes = module_axes(t, dim);
        let near = module * (4.0 + dim as f32 / 16.0);
        let first = find_alignment(bin, &mut self.runs, pred, axes, near);
        // Strong perspective moves the pattern far from the affine prediction.
        let mods = [t.tl.module, t.tr.module, t.bl.module];
        let mx = mods.iter().copied().fold(0.0, f32::max);
        let mn = mods.iter().copied().fold(f32::MAX, f32::min);
        let wide = mx >= mn * WIDE_SEARCH_RATIO;
        first.or_else(|| {
            wide.then(|| find_alignment(bin, &mut self.runs, pred, axes, near * 3.0))
                .flatten()
        })
    }

    /// Grows a homography from the finders by locating alignment patterns
    /// one after another, nearest to the known points first.
    fn refine(&mut self, bin: Plane<'_>, t: &Triplet, dim: usize) -> Option<Mapper> {
        let version = version_for_dimension(dim)?;
        let d = dim as f32;
        let (tl, tr, bl) = (t.tl.pos, t.tr.pos, t.bl.pos);
        self.pts.clear();
        self.pts.extend([
            (Point::new(3.5, 3.5), tl),
            (Point::new(d - 3.5, 3.5), tr),
            (Point::new(3.5, d - 3.5), bl),
            (Point::new(d - 3.5, d - 3.5), fourth_point(t)),
        ]);
        self.cells.clear();
        self.cells.extend(
            pattern_cells(version)
                .map(|(r, c)| (Point::new(f32::from(c) + 0.5, f32::from(r) + 0.5), false)),
        );
        let mut model = Homography::fit(&self.pts)?;
        let mut real = 0usize;
        let mut misses = 0usize;
        loop {
            let known = if real == 0 { 3 } else { self.pts.len() };
            let mut best: Option<(usize, f32)> = None;
            for (i, (c, done)) in self.cells.iter().enumerate() {
                if *done {
                    continue;
                }
                let dist = self.pts[..known]
                    .iter()
                    .map(|(m, _)| m.distance(*c))
                    .fold(f32::MAX, f32::min);
                if best.is_none_or(|b| dist < b.1) {
                    best = Some((i, dist));
                }
            }
            let Some((ci, _)) = best else {
                break;
            };
            self.cells[ci].1 = true;
            let c = self.cells[ci].0;
            let (Some(pred), Some(px), Some(py)) = (
                model.map(c),
                model.map(Point::new(c.x + 1.0, c.y)),
                model.map(Point::new(c.x, c.y + 1.0)),
            ) else {
                continue;
            };
            let ex = Point::new(px.x - pred.x, px.y - pred.y);
            let ey = Point::new(py.x - pred.x, py.y - pred.y);
            let module = (ex.x.hypot(ex.y) + ey.x.hypot(ey.y)) / 2.0;
            let radius = module * if real == 0 { 4.5 } else { 3.0 };
            let Some(found) = find_alignment(bin, &mut self.runs, pred, (ex, ey), radius) else {
                misses += 1;
                if real == 0 && misses >= MAX_MISSES {
                    break;
                }
                continue;
            };
            if real == 0 {
                self.pts.truncate(3);
            }
            self.pts.push((c, found));
            real += 1;
            match Homography::fit(&self.pts) {
                Some(m) => model = m,
                None => {
                    self.pts.pop();
                }
            }
        }
        (real > 0).then_some(Mapper::Fitted(model))
    }
}

fn build_symbol(found: &Found, scale: (f32, f32)) -> Option<Symbol> {
    let d = found.dim as f32;
    let corners = [(0.0, 0.0), (d, 0.0), (d, d), (0.0, d)];
    let mut pts = [Point::default(); 4];
    for (p, (x, y)) in pts.iter_mut().zip(corners) {
        let q = found.h.map(Point::new(x, y))?;
        *p = Point::new(q.x * scale.0, q.y * scale.1);
    }
    let quad = Quad::new(pts);
    let dec = &found.decoded;
    Some(Symbol {
        symbology: Symbology::Qr,
        data: dec.data.clone(),
        text: None,
        segments: dec.segments.clone(),
        content: Content::Unparsed,
        quad,
        orientation_deg: quad.orientation_deg(),
        aim_id: dec.aim_id,
        meta: SymbolMeta::Qr(QrMeta {
            version: dec.version,
            ec_level: dec.ec_level,
            mask: dec.mask,
            errors_corrected: dec.errors_corrected,
            eci: dec.eci.clone(),
            mirrored: dec.mirrored,
        }),
        quality: 1.0 / (1.0 + 0.1 * dec.errors_corrected as f32),
    })
}

/// A symbol already reported in this frame.
struct Known {
    quad: Quad,
    /// Module size in the coordinates of the binary plane.
    module: f32,
}

/// A finder at least this fraction of a decoded symbol's module size inside
/// its quad belongs to that symbol; smaller ones may start a nested symbol.
const NESTED_MODULE_RATIO: f32 = 0.5;

impl Known {
    fn new(s: &Symbol, scale: (f32, f32)) -> Option<Self> {
        let SymbolMeta::Qr(meta) = &s.meta else {
            return None;
        };
        let c = s.quad.corners;
        let side = (0..4).map(|i| c[i].distance(c[(i + 1) % 4])).sum::<f32>() / 4.0;
        let dim = 17.0 + 4.0 * f32::from(meta.version);
        Some(Self {
            quad: s.quad,
            module: side / dim / ((scale.0 + scale.1) / 2.0),
        })
    }

    /// Whether a finder (plane coordinates) lies on this symbol rather than
    /// on a much smaller one nested inside it.
    fn covers(&self, pos: Point, module: f32, scale: (f32, f32)) -> bool {
        self.quad
            .contains(Point::new(pos.x * scale.0, pos.y * scale.1))
            && module >= NESTED_MODULE_RATIO * self.module
    }
}

impl SymbologyReader for QrReader {
    fn name(&self) -> &'static str {
        "qr"
    }

    fn symbologies(&self) -> SymbologySet {
        SymbologySet::EMPTY.with(Symbology::Qr)
    }

    fn handles_inversion(&self) -> bool {
        true
    }

    fn scan(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
        let params = Params::for_effort(ctx.options().effort);
        let scale = ctx.scale();
        let mut known: Vec<Known> = out
            .iter()
            .filter(|s| s.symbology == Symbology::Qr)
            .filter_map(|s| Known::new(s, scale))
            .collect();
        let original = ctx.original();
        let bin = ctx.binary();
        if bin.width() < 21 || bin.height() < 21 {
            return;
        }
        let mut work = Work {
            gray: Some(Gray {
                img: original,
                scale,
            }),
            ..Work::default()
        };
        work.search.find_finders(bin, params.row_step);
        if !known.is_empty() {
            work.search
                .finders
                .retain(|f| !known.iter().any(|k| k.covers(f.pos, f.module, scale)));
        }
        // Both polarities share the finder search; triplets and budgets are
        // per polarity.
        for inverted in [false, true] {
            let plane = Plane::new(bin, inverted);
            work.search
                .select_triplets(plane, params.max_finders, params.row_step);
            let mut failures = 0;
            let mut syn_failures = 0;
            let mut successes = 0;
            let mut ti = 0;
            while ti < work.search.triplets.len() {
                let t = work.search.triplets[ti];
                ti += 1;
                let spent = if t.synthetic {
                    syn_failures >= params.max_recovered_failures + 3 * successes
                } else {
                    failures >= params.max_failures + 3 * successes
                };
                if spent {
                    continue;
                }
                let inside = [t.tl, t.tr, t.bl]
                    .iter()
                    .any(|f| known.iter().any(|k| k.covers(f.pos, f.module, scale)));
                if inside {
                    continue;
                }
                let Some(found) = work.try_triplet(plane, &t) else {
                    if t.synthetic {
                        syn_failures += 1;
                    } else {
                        failures += 1;
                    }
                    continue;
                };
                if let Some(sym) = build_symbol(&found, scale) {
                    successes += 1;
                    known.extend(Known::new(&sym, scale));
                    out.push(sym);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
