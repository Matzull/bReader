//! Aztec reader orchestration: bullseye candidates, core fit, orientation,
//! layer sampling and decoding.

use std::cell::RefCell;

use breader_core::{
    AztecMeta, BitMatrix, Effort, FrameCache, FrameContext, Plane, Point, Quad, Symbol, SymbolMeta,
    Symbology, SymbologyReader, SymbologySet,
};

use crate::decode::{DecodedAztec, Shape, decode_grid, probe};
use crate::detect::{BullseyeSearch, Candidate};
use crate::sample::{
    CoreFit, Crossing, Mapper, anchor_grid, bullseye_match, cast_rays, fit_core,
    grid_lines_plausible, orient, refine_core, refine_symbol, sample_core, sample_grid, unorient,
};
use crate::spec;

/// Aztec Code reader (compact, full-range and Runes); handles both
/// polarities itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct AztecReader;

/// Per-effort search limits.
#[derive(Debug, Clone, Copy)]
struct Params {
    row_step: usize,
    /// Bullseyes examined per frame, best first.
    max_candidates: usize,
    /// Grids that may fail to decode in one scan.
    max_decodes: usize,
    /// Whether light-on-dark symbols are searched too.
    invert: bool,
    /// Fit plans tried per candidate.
    plans: usize,
    /// Whether later passes are skipped while no symbol is known.
    first_pass_only: bool,
}

impl Params {
    fn for_effort(e: Effort) -> Self {
        match e {
            Effort::Fast => Self {
                row_step: 3,
                max_candidates: 8,
                max_decodes: 6,
                invert: false,
                plans: 2,
                first_pass_only: true,
            },
            Effort::Balanced => Self {
                row_step: 2,
                max_candidates: 24,
                max_decodes: 16,
                invert: true,
                plans: 4,
                first_pass_only: true,
            },
            // Exhaustive, and any preset added later.
            _ => Self {
                row_step: 1,
                max_candidates: 64,
                max_decodes: 48,
                invert: true,
                plans: 4,
                first_pass_only: false,
            },
        }
    }
}

/// Ring boundaries fitted for a compact core, and for a full-range one.
const COMPACT_RINGS: u8 = 4;
const FULL_RINGS: u8 = 6;
/// Share of the bullseye modules that must agree before a core that fails the
/// probe is fitted again.
const MIN_BULLSEYE_MATCH: f32 = 0.9;
/// Frames up to this many pixels are searched in every pass: it costs next to
/// nothing, and a small clean symbol may only binarize in the second one.
const SMALL_FRAME_PIXELS: usize = 160_000;
/// Least module size, in pixels, at which aligning a whole grid on its modules works.
const MIN_REFINE_MODULE: f32 = 3.5;
/// Least module size, in pixels, worth sampling.
const MIN_MODULE: f32 = 1.2;
/// Rounds of casting rays again from the center the last fit found, and the
/// distance (in modules) under which the center counts as found.
const CENTER_ROUNDS: usize = 3;
const CENTER_TOLERANCE: f32 = 0.03;
/// Bullseye candidates tried per accepted one, at most.
const CANDIDATE_BUDGET_FACTOR: usize = 8;

#[derive(Debug, Default)]
struct Work {
    search: BullseyeSearch,
    crossings: Vec<Crossing>,
    core: BitMatrix,
    grid: BitMatrix,
    /// A bullseye was found that did not decode.
    suspect: bool,
}

/// Per-frame note from one pass to the next: some pass saw a bullseye it could
/// not read.
#[derive(Debug, Default)]
struct Suspect(bool);

impl FrameCache for Suspect {
    fn reset(&mut self) {
        self.0 = false;
    }
}

thread_local! {
    static WORK: RefCell<Work> = RefCell::new(Work::default());
}

struct Found {
    decoded: DecodedAztec,
    mapper: Mapper,
    size: usize,
    rot: usize,
    mirror: bool,
}

/// A reading of the core that survived the probe.
#[derive(Debug, Clone, Copy)]
struct Try {
    rot: usize,
    mirror: bool,
    shape: Shape,
}

/// One attempt at reading a candidate: how its core is fitted. Later plans
/// run only when the earlier ones did not decode.
#[derive(Debug, Clone, Copy)]
struct Plan {
    /// Ring boundaries fitted.
    rings: u8,
    /// See `fit_core`: corner crossings left out of the fit.
    corner_skip: f32,
    /// Moves the core corners onto the bullseye and orientation marks, for a
    /// full-range (`Some(true)`) or a compact (`Some(false)`) bullseye.
    refine: Option<bool>,
}

const PLANS: [Plan; 4] = [
    Plan {
        rings: COMPACT_RINGS,
        corner_skip: 2.0,
        refine: None,
    },
    Plan {
        rings: FULL_RINGS,
        corner_skip: 2.0,
        refine: None,
    },
    Plan {
        rings: FULL_RINGS,
        corner_skip: 0.8,
        refine: Some(true),
    },
    Plan {
        rings: COMPACT_RINGS,
        corner_skip: 0.8,
        refine: Some(false),
    },
];

impl Work {
    /// Fits the core from rays cast at `center`, again from the fitted center
    /// while it moves: the fit assumes that the rays leave from the center of
    /// the symbol, and a quarter of a pixel off is enough to skew the corners
    /// of the core by a module.
    fn fit(&mut self, plane: Plane<'_>, center: Point, module: f32, plan: Plan) -> Option<CoreFit> {
        let Plan {
            rings, corner_skip, ..
        } = plan;
        cast_rays(plane, center, module, rings, &mut self.crossings);
        let mut fit = fit_core(center, &self.crossings, module, rings, corner_skip)?;
        let mut from = center;
        for _ in 0..CENTER_ROUNDS {
            let Some(moved) = fit.h.map(Point::new(0.0, 0.0)) else {
                break;
            };
            if moved.distance(from) <= CENTER_TOLERANCE * fit.module {
                break;
            }
            cast_rays(plane, moved, fit.module, rings, &mut self.crossings);
            let Some(better) = fit_core(moved, &self.crossings, fit.module, rings, corner_skip)
            else {
                break;
            };
            fit = better;
            from = moved;
        }
        match plan.refine {
            Some(full) => Some(refine_core(plane, &fit, full)),
            None => Some(fit),
        }
    }

    /// The readings of the core sampled through `fit` that pass the probe, in
    /// every orientation.
    fn probe_core(&mut self, plane: Plane<'_>, fit: &CoreFit) -> Vec<Try> {
        let mut tries = Vec::new();
        if !sample_core(plane, fit, &mut self.core) {
            return tries;
        }
        for mirror in [false, true] {
            for rot in 0..4 {
                let oriented = orient(&self.core, rot, mirror);
                for shape in probe(&oriented) {
                    tries.push(Try { rot, mirror, shape });
                }
            }
        }
        tries
    }

    /// Reads one candidate with the first of `plans` fit plans that decodes.
    /// `examined` counts the candidates whose bullseye fit at all and
    /// `decodes` the grids handed to the decoder.
    fn read(
        &mut self,
        plane: Plane<'_>,
        cand: &Candidate,
        plans: usize,
        budget: (&mut usize, usize),
        examined: &mut usize,
    ) -> Option<Found> {
        let (decodes, max_decodes) = budget;
        for (n, plan) in PLANS.iter().take(plans).enumerate() {
            let Some(fit) = self.fit(plane, cand.pos, cand.module, *plan) else {
                continue;
            };
            if n == 0 {
                *examined += 1;
            }
            if fit.module < MIN_MODULE {
                return None;
            }
            let tries = self.probe_core(plane, &fit);
            // Only a bullseye that is there but misaligned deserves the
            // costlier fits and, in the later passes, another look.
            let bullseye = bullseye_match(&self.core, true).max(bullseye_match(&self.core, false))
                >= MIN_BULLSEYE_MATCH;
            self.suspect |= bullseye;
            if tries.is_empty() && n >= 1 && !bullseye {
                return None;
            }
            for t in tries.into_iter().take(2) {
                if *decodes >= max_decodes {
                    return None;
                }
                if let Some(f) = self.try_shape(plane, cand, &fit, t, decodes) {
                    return Some(f);
                }
                // A compact symbol seen at an angle: the rings alone leave its
                // outer layers misplaced; align the whole grid.
                if let Shape::Layout(layout) = t.shape
                    && layout.compact
                    && fit.module >= MIN_REFINE_MODULE
                    && *decodes < max_decodes
                    && let Some(better) = refine_symbol(plane, &fit, layout.size())
                    && let Some(f) = self.try_shape(plane, cand, &better, t, decodes)
                {
                    return Some(f);
                }
            }
        }
        None
    }

    fn try_shape(
        &mut self,
        plane: Plane<'_>,
        cand: &Candidate,
        fit: &CoreFit,
        t: Try,
        decodes: &mut usize,
    ) -> Option<Found> {
        let (mapper, size) = match t.shape {
            Shape::Rune => (Mapper::Global(fit.h), spec::RUNE_SIZE),
            Shape::Layout(layout) if layout.compact => (Mapper::Global(fit.h), layout.size()),
            Shape::Layout(layout) => {
                let size = layout.size();
                let wide = if fit.rings >= FULL_RINGS {
                    *fit
                } else {
                    self.fit(plane, cand.pos, cand.module, PLANS[1])
                        .unwrap_or(*fit)
                };
                (anchor_grid(plane, &wide, size), size)
            }
        };
        if !sample_grid(plane, &mapper, size, &mut self.grid) {
            return None;
        }
        let grid = orient(&self.grid, t.rot, t.mirror);
        if let Shape::Layout(layout) = t.shape
            && !grid_lines_plausible(&grid, layout)
        {
            return None;
        }
        *decodes += 1;
        let decoded = decode_grid(&grid, t.mirror);
        let decoded = decoded.ok()?;
        Some(Found {
            decoded,
            mapper,
            size,
            rot: t.rot,
            mirror: t.mirror,
        })
    }
}

fn build_symbol(found: &Found, scale: (f32, f32)) -> Option<Symbol> {
    let half = found.size as f32 / 2.0;
    let mut pts = [Point::default(); 4];
    for (p, (u, v)) in
        pts.iter_mut()
            .zip([(-half, -half), (half, -half), (half, half), (-half, half)])
    {
        let m = unorient(Point::new(u, v), found.rot, found.mirror);
        let q = found.mapper.map(m)?;
        *p = Point::new(q.x * scale.0, q.y * scale.1);
    }
    let quad = Quad::new(pts);
    let dec = &found.decoded;
    let mut meta = AztecMeta::new(dec.compact, dec.layers);
    meta.data_codewords = dec.data_codewords;
    meta.ec_codewords = dec.ec_codewords;
    meta.errors_corrected = dec.errors_corrected;
    meta.erasures = dec.erasures;
    meta.mirrored = dec.mirrored;
    meta.rune = dec.rune;
    meta.reader_init = dec.reader_init;
    meta.eci = dec.eci.clone();
    let mut symbol = Symbol::new(
        Symbology::Aztec,
        dec.data.clone(),
        quad,
        dec.aim_id,
        SymbolMeta::Aztec(meta),
    );
    symbol.segments = dec.segments.clone();
    symbol.quality = 1.0 / (1.0 + 0.1 * dec.errors_corrected as f32);
    Some(symbol)
}

fn scan_with(work: &mut Work, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
    let params = Params::for_effort(ctx.options().effort);
    let scale = ctx.scale();
    let mut known: Vec<Quad> = out
        .iter()
        .filter(|s| s.symbology == Symbology::Aztec)
        .map(|s| s.quad)
        .collect();
    // A bullseye shows in the first binarization of the original image; the
    // later passes only repeat the search when one is known (more may be
    // hidden), when an earlier pass saw one it could not read, on small
    // frames, or at Exhaustive effort.
    let pass = ctx.pass();
    let first_pass = pass.preprocessor.is_none() && pass.binarizer == 0;
    if !first_pass
        && known.is_empty()
        && params.first_pass_only
        && !ctx.shared::<Suspect>().0
        && ctx.image().width() * ctx.image().height() > SMALL_FRAME_PIXELS
    {
        return;
    }
    work.suspect = false;
    let raw = ctx.binary();
    if raw.width() < 15 || raw.height() < 15 {
        return;
    }
    work.search.find(raw, params.row_step, params.invert);
    let mut decodes = 0usize;
    let mut examined = 0usize;
    for ci in 0..work.search.candidates.len() {
        if examined >= params.max_candidates
            || decodes >= params.max_decodes
            || ci >= CANDIDATE_BUDGET_FACTOR * params.max_candidates
        {
            break;
        }
        let cand = work.search.candidates[ci];
        let center = Point::new(cand.pos.x * scale.0, cand.pos.y * scale.1);
        if known.iter().any(|q| q.contains(center)) {
            continue;
        }
        let plane = Plane::new(raw, cand.inv);
        let Some(found) = work.read(
            plane,
            &cand,
            params.plans,
            (&mut decodes, params.max_decodes),
            &mut examined,
        ) else {
            continue;
        };
        if let Some(sym) = build_symbol(&found, scale) {
            known.push(sym.quad);
            out.push(sym);
        }
    }
    if work.suspect {
        ctx.shared::<Suspect>().0 = true;
    }
}

impl SymbologyReader for AztecReader {
    fn name(&self) -> &'static str {
        "aztec"
    }

    fn symbologies(&self) -> SymbologySet {
        SymbologySet::EMPTY.with(Symbology::Aztec)
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
