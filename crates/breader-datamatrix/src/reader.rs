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
    Hypothesis, Mapper, SampleScratch, analyze, clock_warp, precheck, refine, sample_grid,
    sample_grid_warped, snap_layout,
};

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
    /// Consecutive candidates (best first) without a finder-like L after which
    /// a polarity is abandoned: the tail of the ranking is texture.
    max_misses: usize,
    /// Whether light-on-dark symbols are searched too.
    invert: bool,
    /// Connected components fitted per polarity and level.
    components: usize,
}

impl Params {
    fn for_effort(e: Effort) -> Self {
        match e {
            Effort::Fast => Self {
                max_candidates: 16,
                hypotheses: 2,
                max_decodes: 24,
                levels: 2,
                components: 48,
                max_misses: 48,
                invert: false,
            },
            Effort::Balanced => Self {
                max_candidates: 64,
                hypotheses: 3,
                max_decodes: 96,
                levels: 3,
                components: 128,
                max_misses: 96,
                invert: true,
            },
            Effort::Exhaustive => Self {
                max_candidates: 192,
                hypotheses: 3,
                max_decodes: 300,
                levels: 5,
                components: 512,
                max_misses: 384,
                invert: true,
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
        );
        for hyp in hyps.iter().flatten() {
            if *decodes >= params.max_decodes {
                return None;
            }
            if let Some(f) = self.try_hypothesis(plane, &layout.corners, hyp, decodes) {
                return Some(f);
            }
        }
        None
    }

    fn try_hypothesis(
        &mut self,
        plane: Plane<'_>,
        corners: &[Point; 4],
        hyp: &Hypothesis,
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
                && sample_grid_warped(plane, &mapper, Some(&w), rows, cols, &mut self.grid)
                && check::plausible(&self.grid, hyp)
            {
                decoded = decode_grid(&self.grid);
                if decoded.is_err() {
                    *decodes += 1;
                }
            }
        }
        let decoded = decoded.ok()?;
        Some(Found {
            decoded,
            mapper,
            rows,
            cols,
        })
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
        work.detector.find(
            raw,
            inverted,
            params.levels,
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
