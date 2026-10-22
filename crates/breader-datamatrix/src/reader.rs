//! Data Matrix reader orchestration: candidates, orientation, sampling and decoding.

use std::cell::RefCell;

use breader_core::{
    BitMatrix, Content, DataMatrixMeta, Effort, FrameContext, Plane, Point, Quad, Reject, Symbol,
    SymbolMeta, Symbology, SymbologyReader, SymbologySet,
};

use crate::check;
use crate::decode::{DecodedDm, decode_grid};
use crate::detect::{Candidate, Detector};
use crate::sample::{
    Hypothesis, Layout, Mapper, SampleScratch, Warp, analyze, broad_hypotheses, clock_warp,
    precheck, refine, sample_grid, sample_grid_warped, snap_layout, track_warp,
};

/// Sizes decoded after the corner search of `Work::read_searched`.
const SEARCHED_TRIES: usize = 4;

/// Data Matrix ECC 200 reader; handles both polarities itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct DataMatrixReader;

/// Per-effort search limits.
#[derive(Debug, Clone, Copy)]
struct Params {
    /// Candidates examined per polarity and pooling level.
    max_candidates: usize,
    /// Size hypotheses tried per candidate.
    hypotheses: usize,
    /// Grids that may fail to decode in one scan.
    max_decodes: usize,
    /// Pooling levels of the candidate search (4, 8, 16, ...).
    levels: usize,
    /// First level of the second stage, run only while no symbol is known.
    late_level: usize,
    /// Consecutive candidates (best first) without a finder-like L after which
    /// a polarity is abandoned: the tail of the ranking is texture.
    max_misses: usize,
    /// Whether light-on-dark symbols are searched too.
    invert: bool,
    /// Connected components fitted per polarity and level.
    components: usize,
    /// Whether a candidate that did not decode gets the geometry searches
    /// (clock-track warp, corner search over every size).
    search: bool,
}

impl Params {
    fn for_effort(e: Effort) -> Self {
        match e {
            Effort::Fast => Self {
                max_candidates: 16,
                hypotheses: 2,
                max_decodes: 24,
                levels: 3,
                late_level: 2,
                components: 48,
                max_misses: 48,
                invert: false,
                search: false,
            },
            Effort::Balanced => Self {
                max_candidates: 64,
                hypotheses: 3,
                max_decodes: 96,
                levels: 3,
                late_level: 3,
                components: 128,
                max_misses: 96,
                invert: true,
                search: true,
            },
            Effort::Exhaustive => Self {
                max_candidates: 192,
                hypotheses: 3,
                max_decodes: 300,
                levels: 5,
                late_level: 5,
                components: 512,
                max_misses: 384,
                invert: true,
                search: true,
            },
        }
    }
}

/// Buffers reused across scans of one thread.
#[derive(Debug, Default)]
struct Work {
    detector: Detector,
    scratch: SampleScratch,
    grid: BitMatrix,
    /// Whether the last candidate looked like a Data Matrix (L and clock tracks).
    layout_ok: bool,
    /// Sizes ranked by `broad_hypotheses`, kept to reuse the buffer.
    ranked: Vec<(f32, Hypothesis, [Point; 4])>,
}

thread_local! {
    static WORK: RefCell<Work> = RefCell::new(Work::default());
}

struct Found {
    decoded: DecodedDm,
    mapper: Mapper,
    rows: usize,
    cols: usize,
}

impl Work {
    /// Reads one candidate; `decodes` counts the grids handed to the decoder.
    fn read(
        &mut self,
        plane: Plane<'_>,
        cand: &Candidate,
        module: f32,
        params: &Params,
        decodes: &mut usize,
    ) -> Option<Found> {
        let layout = analyze(plane, &cand.quad, module, &mut self.scratch);
        self.layout_ok = layout.is_some();
        let layout = layout?;
        let mut hyps = [None; 3];
        snap_layout(
            &layout,
            &self.scratch,
            &mut hyps[..params.hypotheses.min(3)],
            params.search,
        );
        for hyp in hyps.iter().flatten() {
            if *decodes >= params.max_decodes {
                return None;
            }
            if let Some(f) =
                self.try_hypothesis(plane, &layout.corners, hyp, params.search, decodes)
            {
                return Some(f);
            }
        }
        if params.search {
            return self.read_searched(plane, &layout, params, decodes);
        }
        None
    }

    /// Last resort for a candidate whose clock tracks and finder look right
    /// but whose size estimate or geometry did not decode: every size is
    /// scored by how well its clock tracks fit after a search of the
    /// three free corners, and the best few are decoded.
    fn read_searched(
        &mut self,
        plane: Plane<'_>,
        layout: &Layout,
        params: &Params,
        decodes: &mut usize,
    ) -> Option<Found> {
        let mut ranked = std::mem::take(&mut self.ranked);
        broad_hypotheses(plane, layout, &mut self.scratch, &mut ranked);
        let mut found = None;
        for (_, hyp, c2) in ranked.iter().take(SEARCHED_TRIES) {
            if *decodes >= params.max_decodes {
                break;
            }
            let Some(m2) = refine(plane, c2, hyp, &mut self.scratch) else {
                continue;
            };
            let mut got = self.try_grid(plane, &m2, None, hyp, decodes);
            if got.is_none()
                && let Some(w) = track_warp(plane, &m2, hyp, &mut self.scratch)
            {
                got = self.try_grid(plane, &m2, Some(&w), hyp, decodes);
            }
            if let Some(decoded) = got {
                let (rows, cols) = hyp.grid_dims();
                found = Some(Found {
                    decoded,
                    mapper: m2,
                    rows,
                    cols,
                });
                break;
            }
        }
        self.ranked = ranked;
        found
    }

    fn try_hypothesis(
        &mut self,
        plane: Plane<'_>,
        corners: &[Point; 4],
        hyp: &Hypothesis,
        search: bool,
        decodes: &mut usize,
    ) -> Option<Found> {
        let (rows, cols) = hyp.grid_dims();
        let mapper = refine(plane, corners, hyp, &mut self.scratch)?;
        if !sample_grid(plane, &mapper, rows, cols, &mut self.grid) {
            return None;
        }
        let plausible = check::plausible(&self.grid, hyp);
        let mut decoded = if plausible {
            let d = decode_grid(&self.grid);
            if d.is_err() {
                *decodes += 1;
            }
            d
        } else {
            Err(Reject::Sampling)
        };
        if decoded.is_err() {
            // Retry through a mapping that follows the module pitch measured
            // on the clock tracks.
            if let Some(w) = clock_warp(plane, &mapper, hyp)
                && sample_grid_warped(plane, &mapper, Some(&w), rows, cols, &mut self.grid, false)
                && check::plausible(&self.grid, hyp)
            {
                decoded = decode_grid(&self.grid);
                if decoded.is_err() {
                    *decodes += 1;
                }
            }
        }
        if decoded.is_err()
            && search
            && let Some(w) = track_warp(plane, &mapper, hyp, &mut self.scratch)
            && let Some(d) = self.try_grid(plane, &mapper, Some(&w), hyp, decodes)
        {
            // A stretched or compressed axis: every boundary sits where the
            // clock tracks show it.
            decoded = Ok(d);
        }
        let decoded = decoded.ok()?;
        Some(Found {
            decoded,
            mapper,
            rows,
            cols,
        })
    }

    /// Samples the grid through `mapper` (and `warp`) and decodes it when it
    /// matches the patterns of the hypothesis. The retries that search the
    /// geometry never read a module outside the image.
    fn try_grid(
        &mut self,
        plane: Plane<'_>,
        mapper: &Mapper,
        warp: Option<&Warp>,
        hyp: &Hypothesis,
        decodes: &mut usize,
    ) -> Option<DecodedDm> {
        let (rows, cols) = hyp.grid_dims();
        if !sample_grid_warped(plane, mapper, warp, rows, cols, &mut self.grid, true)
            || !check::plausible(&self.grid, hyp)
        {
            return None;
        }
        let d = decode_grid(&self.grid);
        if d.is_err() {
            *decodes += 1;
        }
        d.ok()
    }
}

fn build_symbol(found: &Found, scale: (f32, f32)) -> Option<Symbol> {
    let (w, h) = (found.cols as f32, found.rows as f32);
    let mut pts = [Point::default(); 4];
    for (p, (u, v)) in pts.iter_mut().zip([(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]) {
        let q = found.mapper.map_point(u, v)?;
        *p = Point::new(q.x * scale.0, q.y * scale.1);
    }
    let quad = Quad::new(pts);
    let dec = &found.decoded;
    Some(Symbol {
        symbology: Symbology::DataMatrix,
        data: dec.data.clone(),
        text: None,
        segments: dec.segments.clone(),
        content: Content::Unparsed,
        quad,
        orientation_deg: quad.orientation_deg(),
        aim_id: dec.aim_id,
        meta: SymbolMeta::DataMatrix(DataMatrixMeta {
            rows: dec.rows,
            cols: dec.cols,
            errors_corrected: dec.errors_corrected,
            mirrored: dec.mirrored,
            eci: dec.eci.clone(),
            structured_append: dec.structured_append,
            reader_init: dec.reader_programming,
        }),
        quality: 1.0 / (1.0 + 0.1 * dec.errors_corrected as f32),
    })
}

fn scan_with(work: &mut Work, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
    let effort = ctx.options().effort;
    let params = Params::for_effort(effort);
    let scale = ctx.scale();
    let mut known: Vec<Quad> = out
        .iter()
        .filter(|s| s.symbology == Symbology::DataMatrix)
        .map(|s| s.quad)
        .collect();
    // The first pass (original image, first binarizer) finds nearly every
    // symbol. Later passes only repeat the search when a symbol is already
    // known (more may be hidden) or at Exhaustive effort.
    let pass = ctx.pass();
    let first_pass = pass.preprocessor.is_none() && pass.binarizer == 0;
    if !first_pass && known.is_empty() && effort != Effort::Exhaustive {
        return;
    }
    let raw = ctx.binary();
    if raw.width() < 10 || raw.height() < 10 {
        return;
    }
    let mut decodes = 0usize;
    for inverted in [false, true] {
        if inverted && !params.invert {
            continue;
        }
        // The coarsest level of Fast is a second stage: it costs a fit
        // pass on every frame and finds only large symbols, so it runs
        // when the finer levels left no symbol (D-046).
        let late = params.late_level.min(params.levels);
        for stage in 0..2 {
            let (from, to) = if stage == 0 {
                (0, late)
            } else {
                (late, params.levels)
            };
            if from >= to || (stage == 1 && !known.is_empty()) {
                continue;
            }
            work.detector.find(
                raw,
                inverted,
                from,
                to,
                if inverted {
                    params.components / 3
                } else {
                    params.components
                },
                params.max_candidates * 32,
            );
            let plane = Plane::new(raw, inverted);
            let mut analyzed = 0usize;
            let mut misses = 0usize;
            for ci in 0..work.detector.candidates.len() {
                if decodes >= params.max_decodes {
                    return;
                }
                let cand = work.detector.candidates[ci];
                let c = Quad::new(cand.quad).center();
                let center = Point::new(c.x * scale.0, c.y * scale.1);
                if known.iter().any(|q| q.contains(center)) {
                    continue;
                }
                let Some(module) = precheck(plane, &cand.quad, &mut work.scratch) else {
                    misses += 1;
                    if misses > params.max_misses {
                        break;
                    }
                    continue;
                };
                misses = 0;
                if analyzed >= params.max_candidates {
                    break;
                }
                let Some(found) = work.read(plane, &cand, module, &params, &mut decodes) else {
                    if work.layout_ok {
                        analyzed += 1;
                    }
                    continue;
                };
                if let Some(sym) = build_symbol(&found, scale) {
                    known.push(sym.quad);
                    out.push(sym);
                }
            }
        }
    }
}

impl SymbologyReader for DataMatrixReader {
    fn name(&self) -> &'static str {
        "datamatrix"
    }

    fn symbologies(&self) -> SymbologySet {
        SymbologySet::EMPTY.with(Symbology::DataMatrix)
    }

    fn handles_inversion(&self) -> bool {
        true
    }

    fn scan(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
        WORK.with(|cell| match cell.try_borrow_mut() {
            Ok(mut work) => scan_with(&mut work, ctx, out),
            Err(_) => scan_with(&mut Work::default(), ctx, out),
        });
    }
}

#[cfg(test)]
mod tests;
