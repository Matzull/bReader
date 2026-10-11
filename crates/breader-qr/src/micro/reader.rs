//! Micro QR candidates from the finder patterns of the QR scan.

use breader_core::{
    BitMatrix, Content, Effort, Homography, Plane, Point, QrMeta, Quad, Symbol, SymbolMeta,
    Symbology,
};

use super::decode::{DecodedMicro, decode_grid};
use super::geom::{Corners, TEMPLATE_MIN, fit_pose, mapping_for_quadrant};
use super::refine::refine_corners;
use crate::detect::Finder;
use crate::reader::Known;
use crate::sample::{Gray, Mapper, read_module, sample_grid_local};

/// Per-effort limits.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MicroParams {
    /// Finders examined per pass.
    pub max_candidates: usize,
    /// Retries after a failed decode: local threshold, corner refinement.
    pub retries: bool,
    /// Row step of the finder scan, which weights the finders' strength.
    pub row_step: usize,
    /// Factor from plane to image coordinates.
    pub scale: (f32, f32),
}

impl MicroParams {
    pub fn new(e: Effort, row_step: usize, scale: (f32, f32)) -> Self {
        let base = match e {
            Effort::Fast => (4, false),
            Effort::Balanced => (10, true),
            Effort::Exhaustive => (40, true),
        };
        Self {
            max_candidates: base.0,
            retries: base.1,
            row_step,
            scale,
        }
    }
}

/// Mismatches allowed in the 64 finder and separator modules.
const FINDER_TOLERANCE: u32 = 6;
/// Fraction (percent) of in-image quiet-zone samples that must be light.
const QUIET_MIN_PERCENT: u32 = 75;

/// Whole-grid structure: finder with separator and timing patterns. The
/// timing run is short, so M1 tolerates no error at all.
fn structure_ok(g: &BitMatrix, d: usize) -> bool {
    let mut bad = 0;
    for y in 0..8usize {
        for x in 0..8usize {
            let want = x < 7 && y < 7 && (x as i32 - 3).abs().max((y as i32 - 3).abs()) != 2;
            bad += u32::from(g.get(x, y) != want);
        }
    }
    if bad > FINDER_TOLERANCE {
        return false;
    }
    let mut tbad = 0;
    for i in 8..d {
        let want = i % 2 == 0;
        tbad += u32::from(g.get(i, 0) != want) + u32::from(g.get(0, i) != want);
    }
    let allowed = if d == 11 { 0 } else { (d as u32 - 8) / 3 };
    tbad <= allowed
}

/// The ring one module outside the symbol must be mostly light.
fn quiet_ok(bin: Plane<'_>, h: &Mapper, d: usize) -> bool {
    let (mut light, mut total) = (0u32, 0u32);
    let df = d as f32;
    for i in 0..d {
        let t = i as f32 + 0.5;
        for p in [(t, -0.5), (-0.5, t), (t, df + 0.5), (df + 0.5, t)] {
            let Some(q) = h.map(Point::new(p.0, p.1)) else {
                return false;
            };
            if q.x >= 0.0 && q.y >= 0.0 {
                let (x, y) = (q.x as usize, q.y as usize);
                if x < bin.width() && y < bin.height() {
                    total += 1;
                    light += u32::from(!bin.get(x, y));
                }
            }
        }
    }
    total == 0 || light * 100 >= total * QUIET_MIN_PERCENT
}

fn sample(bin: Plane<'_>, h: &Mapper, d: usize, multi: bool, grid: &mut BitMatrix) -> bool {
    grid.resize(d, d);
    let mut outside = 0;
    for y in 0..d {
        for x in 0..d {
            match read_module(bin, h, x, y, multi) {
                None => return false,
                Some(None) => outside += 1,
                Some(Some(v)) => grid.set(x, y, v),
            }
        }
    }
    outside * 20 <= d * d
}

struct Found {
    decoded: DecodedMicro,
    h: Mapper,
    dim: usize,
}

fn try_mapping(
    bin: Plane<'_>,
    gray: Option<Gray<'_>>,
    h: Homography,
    d: usize,
    params: MicroParams,
    grid: &mut BitMatrix,
) -> Option<Found> {
    let m = Mapper::Fitted(h);
    let px = module_px(&m, d);
    if !quiet_ok(bin, &m, d) {
        return None;
    }
    let found = |decoded: DecodedMicro, h: Mapper| Found { decoded, h, dim: d };
    let mut plausible = false;
    if sample(bin, &m, d, px >= 3.0, grid) && structure_ok(grid, d) {
        plausible = true;
        if let Ok(decoded) = decode_grid(grid) {
            return Some(found(decoded, m));
        }
    }
    if params.retries
        && let Some(g) = gray
        && sample_grid_local(g, bin.inverted(), &m, d, grid)
        && structure_ok(grid, d)
        && let Ok(decoded) = decode_grid(grid)
    {
        return Some(found(decoded, m));
    }
    if params.retries
        && plausible
        && let Some(m2) = refine_corners(bin, &m, d)
    {
        if sample(bin, &m2, d, px >= 3.0, grid)
            && structure_ok(grid, d)
            && let Ok(decoded) = decode_grid(grid)
        {
            return Some(found(decoded, m2));
        }
        if let Some(g) = gray
            && sample_grid_local(g, bin.inverted(), &m2, d, grid)
            && structure_ok(grid, d)
            && let Ok(decoded) = decode_grid(grid)
        {
            return Some(found(decoded, m2));
        }
    }
    None
}

/// Module size in pixels at the middle of the symbol.
fn module_px(m: &Mapper, d: usize) -> f32 {
    let c = d as f32 / 2.0;
    match (
        m.map(Point::new(c, c)),
        m.map(Point::new(c + 1.0, c)),
        m.map(Point::new(c, c + 1.0)),
    ) {
        (Some(p), Some(a), Some(b)) => (p.distance(a) + p.distance(b)) / 2.0,
        _ => 0.0,
    }
}

fn build_symbol(found: &Found, scale: (f32, f32)) -> Option<Symbol> {
    let d = found.dim as f32;
    let mut pts = [Point::default(); 4];
    for (p, (x, y)) in pts.iter_mut().zip([(0.0, 0.0), (d, 0.0), (d, d), (0.0, d)]) {
        let q = found.h.map(Point::new(x, y))?;
        *p = Point::new(q.x * scale.0, q.y * scale.1);
    }
    let quad = Quad::new(pts);
    let dec = &found.decoded;
    Some(Symbol {
        symbology: Symbology::MicroQr,
        data: dec.data.clone(),
        text: None,
        segments: dec.segments.clone(),
        content: Content::Unparsed,
        quad,
        orientation_deg: quad.orientation_deg(),
        // ISO/IEC 15424 modifier for Micro QR not verified against the standard.
        aim_id: *b"]Q1",
        meta: SymbolMeta::Qr(QrMeta {
            version: dec.version,
            ec_level: dec.ec_level,
            mask: dec.mask,
            errors_corrected: dec.errors_corrected,
            eci: Vec::new(),
            mirrored: dec.mirrored,
        }),
        quality: 1.0 / (1.0 + 0.1 * dec.errors_corrected as f32),
    })
}

/// Tries the finders of one polarity as Micro QR corners.
pub(crate) fn scan(
    bin: Plane<'_>,
    gray: Option<Gray<'_>>,
    finders: &[Finder],
    params: MicroParams,
    known: &mut Vec<Known>,
    out: &mut Vec<Symbol>,
) {
    let (row_step, scale) = (params.row_step, params.scale);
    let strength = |f: &Finder| f.count as f32 * row_step as f32 / (7.0 * f.module.max(0.5));
    let mut order: Vec<usize> = (0..finders.len())
        .filter(|&i| finders[i].inv == bin.inverted() && finders[i].module >= 1.4)
        .collect();
    order.sort_by(|&a, &b| strength(&finders[b]).total_cmp(&strength(&finders[a])));
    let mut grid = BitMatrix::default();
    let mut tried = 0;
    for i in order {
        let f = &finders[i];
        if known.iter().any(|k| k.covers(f.pos, f.module, scale)) {
            continue;
        }
        if tried >= params.max_candidates {
            break;
        }
        tried += 1;
        let Some((pose, score)) = fit_pose(bin, f.pos, f.module) else {
            continue;
        };
        debug_assert!(score >= TEMPLATE_MIN);
        let corners = Corners::measure(bin, pose);
        for q in 0..4 {
            let Some((d, h)) = mapping_for_quadrant(bin, &corners, q) else {
                continue;
            };
            let Some(found) = try_mapping(bin, gray, h, d, params, &mut grid) else {
                continue;
            };
            if let Some(sym) = build_symbol(&found, scale) {
                known.extend(Known::new(&sym, scale));
                out.push(sym);
                break;
            }
        }
    }
}
