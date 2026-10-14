use std::cell::RefCell;

use breader_core::{
    Effort, FrameContext, ImageView, Point, Symbol, Symbology, SymbologyReader, SymbologySet,
};

use crate::aggregate::{Aggregator, PlacedHit, Settings};
use crate::codabar::CodabarDecoder;
use crate::code39::Code39Decoder;
use crate::code128::Code128Decoder;
use crate::databar::DataBarDecoder;
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::ean::EanDecoder;
use crate::edges::{EdgeDetector, EdgeParams, Edges};
use crate::itf::ItfDecoder;
use crate::marks::{self, Mark, MarkSet, RawMark};
use crate::plan::{ScanPlan, Scanline, clip_line};
use crate::profile::{self, Oblique, ObliqueWork};
use crate::runs::Runs;

/// Decoders tried on every scanline, composed statically.
type Decoders = (
    EanDecoder,
    Code128Decoder,
    Code39Decoder,
    ItfDecoder,
    CodabarDecoder,
    DataBarDecoder,
);

const DECODERS: Decoders = (
    EanDecoder,
    Code128Decoder,
    Code39Decoder,
    ItfDecoder,
    CodabarDecoder,
    DataBarDecoder,
);

/// Decode state shared by all scanlines of one `scan` call.
#[derive(Debug, Default)]
struct Stage {
    detector: EdgeDetector,
    edges: Edges,
    runs: Runs,
    reversed: Runs,
    hits: Vec<LineHit>,
    aggregator: Aggregator,
    edge_params: Vec<EdgeParams>,
    marks: Vec<Mark>,
    raw_marks: Vec<RawMark>,
}

/// Buffers and decode state of one `scan` call.
#[derive(Debug, Default)]
struct Work {
    lines: Vec<Scanline>,
    profile: Vec<u8>,
    columns: Vec<u8>,
    oblique: Vec<u8>,
    offsets: Vec<usize>,
    seeds: Vec<PlacedHit>,
    column_xs: Vec<usize>,
    oblique_lines: Vec<Scanline>,
    oblique_work: ObliqueWork,
    stage: Stage,
}

impl Work {
    /// Empties everything a scan fills; the allocations stay.
    fn reset(&mut self) {
        self.lines.clear();
        self.seeds.clear();
        let stage = &mut self.stage;
        stage.hits.clear();
        stage.aggregator.reset();
        stage.edge_params.clear();
        stage.marks.clear();
        stage.raw_marks.clear();
    }
}

thread_local! {
    static WORK: RefCell<Work> = RefCell::new(Work::default());
}

/// Edge variants tried per scanline. Direct differentiation keeps narrow
/// bars apart; smoothing suppresses noise on wider modules. Both relocate
/// edges to the half-level crossing.
fn edge_variants(effort: Effort) -> &'static [EdgeParams] {
    const FINE: EdgeParams = EdgeParams::fine();
    const SMOOTH: EdgeParams = EdgeParams::smoothed();
    match effort {
        Effort::Fast => &[SMOOTH],
        Effort::Balanced | Effort::Exhaustive => &[FINE, SMOOTH],
    }
}

/// Scanlines added around a lone hit stop after this many consecutive
/// lines without a matching hit, or at `REFINE_MAX_LINES` per side.
const REFINE_MISSES: usize = 3;
const REFINE_MAX_LINES: usize = 24;
const REFINE_MAX_SEEDS: usize = 48;
/// Quiet zone, in modules, kept on each side of a refinement line.
const REFINE_MARGIN: f32 = 14.0;

/// Dimensions of the pass image.
#[derive(Debug, Clone, Copy)]
struct PassImage {
    width: usize,
    height: usize,
}

fn scan_profile<D: LinearDecoder>(
    decoders: &D,
    profile: &[u8],
    line: &Scanline,
    index: u32,
    step: f32,
    opts: &LineOptions,
    stage: &mut Stage,
) {
    let mut retry = false;
    for (variant, params) in stage.edge_params.iter().enumerate() {
        // The smoothed variant only matters when the unsmoothed one saw a
        // start pattern it could not decode.
        if variant > 0 && !retry {
            break;
        }
        stage.detector.detect(profile, params, &mut stage.edges);
        Runs::from_edges(&stage.edges, &mut stage.runs);
        if stage.runs.is_empty() {
            continue;
        }
        if opts.marks {
            marks::record(&stage.runs, line, &mut stage.raw_marks, &mut stage.marks);
        }
        Runs::reverse_from(&stage.runs, &mut stage.reversed);
        stage.hits.clear();
        if opts.normal {
            retry |= decoders.decode(&stage.runs, opts, &mut stage.hits);
            retry |= decoders.decode(&stage.reversed, opts, &mut stage.hits);
        }
        if opts.invert {
            let dark_symbols = stage.hits.len();
            stage.runs.flip_roles();
            stage.reversed.flip_roles();
            retry |= decoders.decode(&stage.runs, opts, &mut stage.hits);
            retry |= decoders.decode(&stage.reversed, opts, &mut stage.hits);
            for hit in &mut stage.hits[dark_symbols..] {
                hit.inverted = true;
            }
        }
        for hit in stage.hits.drain(..) {
            stage.aggregator.push(PlacedHit {
                symbology: hit.symbology,
                text: hit.text,
                addon: hit.addon,
                from: line.point_at(hit.start),
                to: line.point_at(hit.end),
                line: index,
                step,
                module: hit.module,
                quality: hit.quality,
                aim: hit.aim,
                checksum_verified: hit.checksum_verified,
                short_quiet: hit.short_quiet,
                inverted: hit.inverted,
                start_stop: hit.start_stop,
                segments: hit.segments,
                thin: hit.thin,
                part: hit.part,
            });
        }
    }
}

/// Pre-sampled profiles of the column and oblique lines of a plan.
struct Sources<'a> {
    plan: ScanPlan,
    columns: &'a [u8],
    height: usize,
    oblique: &'a [u8],
    offsets: &'a [usize],
    opts: &'a LineOptions,
}

/// Where each line's profile comes from, as running counts so that any
/// sub-range of lines can be scanned on its own.
#[derive(Debug, Clone, Copy)]
struct Cursor {
    column: usize,
    slanted: usize,
}

impl Cursor {
    fn advance(&mut self, line: &Scanline) {
        if line.from.x == line.to.x {
            self.column += 1;
        } else if Oblique::is_oblique(line) {
            self.slanted += 1;
        }
    }
}

fn scan_line(
    img: &ImageView<'_>,
    line: &Scanline,
    index: u32,
    cursor: Cursor,
    sources: &Sources<'_>,
    profile: &mut Vec<u8>,
    stage: &mut Stage,
) {
    let step = sources.plan.line_spacing(line);
    if line.from.x == line.to.x {
        let h = sources.height;
        let column = &sources.columns[cursor.column * h..(cursor.column + 1) * h];
        scan_profile(&DECODERS, column, line, index, step, sources.opts, stage);
    } else if Oblique::is_oblique(line) {
        let range = sources.offsets[cursor.slanted]..sources.offsets[cursor.slanted + 1];
        scan_profile(
            &DECODERS,
            &sources.oblique[range],
            line,
            index,
            step,
            sources.opts,
            stage,
        );
    } else {
        profile::sample(img, line, false, profile);
        scan_profile(&DECODERS, profile, line, index, step, sources.opts, stage);
    }
}

fn scan_lines(
    img: &ImageView<'_>,
    lines: &[Scanline],
    sources: &Sources<'_>,
    profile: &mut Vec<u8>,
    stage: &mut Stage,
) {
    let mut cursor = Cursor {
        column: 0,
        slanted: 0,
    };
    for (i, line) in lines.iter().enumerate() {
        scan_line(img, line, i as u32, cursor, sources, profile, stage);
        cursor.advance(line);
    }
}

/// Lines per parallel work item.
#[cfg(feature = "rayon")]
const PAR_LINES: usize = 16;
/// Frames below this many pixels are scanned on the calling thread.
#[cfg(feature = "rayon")]
const PAR_MIN_PIXELS: usize = 1 << 19;

#[cfg(feature = "rayon")]
fn parallel_worthwhile(img: &ImageView<'_>, lines: usize) -> bool {
    img.width() * img.height() >= PAR_MIN_PIXELS
        && lines >= 2 * PAR_LINES
        && rayon::current_num_threads() > 1
        && !breader_core::parallel::is_sequential()
}

/// Scans the lines in fixed chunks on the rayon pool. Every chunk collects
/// its hits on its own; they are returned in line order, so the
/// caller sees the same sequence as `scan_lines` would push.
#[cfg(feature = "rayon")]
fn scan_lines_parallel(
    img: &ImageView<'_>,
    lines: &[Scanline],
    sources: &Sources<'_>,
    template: &Stage,
) -> (Vec<PlacedHit>, Vec<Mark>) {
    use rayon::prelude::*;
    let mut starts = Vec::with_capacity(lines.len().div_ceil(PAR_LINES));
    let mut cursor = Cursor {
        column: 0,
        slanted: 0,
    };
    for chunk in lines.chunks(PAR_LINES) {
        starts.push(cursor);
        for line in chunk {
            cursor.advance(line);
        }
    }
    let fresh = || {
        let mut stage = Stage::default();
        stage.edge_params.clone_from(&template.edge_params);
        (stage, Vec::new())
    };
    let per_chunk: Vec<(Vec<PlacedHit>, Vec<Mark>)> = lines
        .par_chunks(PAR_LINES)
        .zip(starts.par_iter())
        .enumerate()
        .map_init(fresh, |(stage, profile), (c, (chunk, start))| {
            let mut cursor = *start;
            for (k, line) in chunk.iter().enumerate() {
                let index = (c * PAR_LINES + k) as u32;
                scan_line(img, line, index, cursor, sources, profile, stage);
                cursor.advance(line);
            }
            (
                stage.aggregator.take_hits(),
                std::mem::take(&mut stage.marks),
            )
        })
        .collect();
    let mut hits = Vec::new();
    let mut marks = Vec::new();
    for (h, m) in per_chunk {
        hits.extend(h);
        marks.extend(m);
    }
    (hits, marks)
}

/// Parallel lines on both sides of `seed`, stopping per side once the
/// symbol stops being read, or, with `settings`, as soon as the symbol has
/// the agreement it needs.
#[allow(clippy::too_many_arguments)]
fn refine<D: LinearDecoder>(
    decoders: &D,
    img: &ImageView<'_>,
    seed: &PlacedHit,
    pass: PassImage,
    opts: &LineOptions,
    work: &mut Work,
    next_index: &mut u32,
    settings: Option<&Settings>,
) {
    let len = seed.from.distance(seed.to);
    if len < 1.0 {
        return;
    }
    let (ux, uy) = (
        (seed.to.x - seed.from.x) / len,
        (seed.to.y - seed.from.y) / len,
    );
    let (nx, ny) = (-uy, ux);
    let opts = &LineOptions {
        normal: !seed.inverted,
        invert: seed.inverted,
        marks: false,
        ..*opts
    };
    let step = seed.module.round().clamp(2.0, 6.0);
    let pad = REFINE_MARGIN * seed.module;
    let axis_aligned = uy.abs() < 1e-4 || ux.abs() < 1e-4;
    for side in [1.0f32, -1.0] {
        let mut misses = 0;
        for k in 1..=REFINE_MAX_LINES {
            let o = side * step * k as f32;
            let a = Point::new(
                seed.from.x - ux * pad + nx * o,
                seed.from.y - uy * pad + ny * o,
            );
            let b = Point::new(seed.to.x + ux * pad + nx * o, seed.to.y + uy * pad + ny * o);
            let Some(line) = clip_line(a, b, pass.width, pass.height, axis_aligned) else {
                break;
            };
            profile::sample(img, &line, false, &mut work.profile);
            let before = work.stage.aggregator.len();
            scan_profile(
                decoders,
                &work.profile,
                &line,
                *next_index,
                step,
                opts,
                &mut work.stage,
            );
            *next_index += 1;
            // Further lines of a confirmed symbol only widen its quad.
            if settings.is_some_and(|st| work.stage.aggregator.seed_met(seed, st)) {
                return;
            }
            if work.stage.aggregator.count_matching(before, seed) > 0 {
                misses = 0;
            } else {
                misses += 1;
                if misses >= REFINE_MISSES {
                    break;
                }
            }
        }
    }
}

/// Start and stop patterns of PDF417 on the lines of the scan plan, for a
/// PDF417 reader that runs without the linear reader.
pub fn scan_marks(img: &ImageView<'_>, effort: Effort, out: &mut MarkSet) {
    let plan = ScanPlan::new(img.width(), img.height(), effort);
    let mut lines = Vec::new();
    plan.lines(&mut lines);
    let mut detector = EdgeDetector::default();
    let mut edges = Edges::default();
    let mut runs = Runs::default();
    let (mut profile, mut raw) = (Vec::new(), Vec::new());
    for line in &lines {
        profile::sample(img, line, false, &mut profile);
        for params in edge_variants(effort) {
            detector.detect(&profile, params, &mut edges);
            Runs::from_edges(&edges, &mut runs);
            marks::record(&runs, line, &mut raw, &mut out.marks);
        }
    }
    out.scanned = true;
}

/// 1D symbology reader. Reads grayscale scanlines, so it never needs the
/// binarized image.
///
/// One `scan` call covers one pass: the scan plan gives the lines, each line
/// is sampled once and decoded in both reading directions and both polarities
/// from the same edges (light bars on dark are the same runs with the roles
/// of bars and spaces swapped), lone hits are confirmed with extra parallel
/// lines, and the aggregator turns agreeing hits into symbols. The reader
/// handles inversion itself, so the pipeline never calls it on inverted passes.
#[derive(Debug, Clone, Copy, Default)]
pub struct LinearReader;

impl SymbologyReader for LinearReader {
    fn name(&self) -> &'static str {
        "linear"
    }

    fn symbologies(&self) -> SymbologySet {
        [
            Symbology::Ean13,
            Symbology::Ean8,
            Symbology::UpcA,
            Symbology::UpcE,
            Symbology::Code128,
            Symbology::Code39,
            Symbology::Itf,
            Symbology::Codabar,
            Symbology::DataBar,
            Symbology::DataBarLimited,
            Symbology::DataBarExpanded,
        ]
        .into_iter()
        .collect()
    }

    fn uses_binary(&self) -> bool {
        false
    }

    fn handles_inversion(&self) -> bool {
        true
    }

    fn scan(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
        // Downscaled variants never added a 1D read on any test set, and
        // cost as much per pixel as the original.
        if ctx.pass().preprocessor.is_some() {
            return;
        }
        self.scan_image(ctx, out);
    }
}

impl LinearReader {
    /// Scans the pass image of `ctx`, whichever variant it is.
    fn scan_image(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
        if ctx.pass().inverted || !ctx.options().symbologies.intersects(self.symbologies()) {
            return;
        }
        WORK.with(|cell| match cell.try_borrow_mut() {
            Ok(mut work) => {
                work.reset();
                self.scan_with(&mut work, ctx, out);
            }
            Err(_) => self.scan_with(&mut Work::default(), ctx, out),
        });
    }

    fn scan_with(&self, work: &mut Work, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
        let opts = LineOptions::from(ctx.options());
        let effort = ctx.options().effort;
        let min_line_agreement = ctx.options().min_line_agreement;
        let addon = ctx.options().ean_addon;
        let scale = ctx.scale();
        let img = ctx.image();
        let plan = ScanPlan::new(img.width(), img.height(), effort);
        let pass = PassImage {
            width: img.width(),
            height: img.height(),
        };
        work.stage
            .edge_params
            .extend_from_slice(edge_variants(effort));
        plan.lines(&mut work.lines);

        work.column_xs.clear();
        work.column_xs.extend(
            work.lines
                .iter()
                .filter(|l| l.from.x == l.to.x)
                .map(|l| l.from.x as usize),
        );
        profile::sample_columns(&img, &work.column_xs, false, &mut work.columns);

        let lines = std::mem::take(&mut work.lines);
        work.oblique_lines.clear();
        work.oblique_lines
            .extend(lines.iter().filter(|l| Oblique::is_oblique(l)).copied());
        profile::sample_oblique_batch(
            &img,
            &work.oblique_lines,
            false,
            &mut work.offsets,
            &mut work.oblique,
            &mut work.oblique_work,
        );
        let sources = Sources {
            plan,
            columns: &work.columns,
            height: img.height(),
            oblique: &work.oblique,
            offsets: &work.offsets,
            opts: &opts,
        };
        #[cfg(feature = "rayon")]
        let parallel = parallel_worthwhile(&img, lines.len());
        #[cfg(not(feature = "rayon"))]
        let parallel = false;
        if parallel {
            #[cfg(feature = "rayon")]
            {
                let (hits, marks) = scan_lines_parallel(&img, &lines, &sources, &work.stage);
                for hit in hits {
                    work.stage.aggregator.push(hit);
                }
                work.stage.marks.extend(marks);
            }
        } else {
            scan_lines(&img, &lines, &sources, &mut work.profile, &mut work.stage);
        }
        let plan_lines = lines.len() as u32;
        let settings = Settings {
            min_agreement: min_line_agreement,
            plan_lines,
            itf_strict: opts.itf_strict,
            addon,
            across_pad: plan.spacing(0.0).min(plan.spacing(90.0)) / 2.0,
            scale,
        };
        let mut next_index = plan_lines;
        let mut seeds = std::mem::take(&mut work.seeds);
        work.stage.aggregator.seeds(&settings, &mut seeds);
        for seed in seeds.iter().take(REFINE_MAX_SEEDS) {
            let early = (effort == Effort::Fast).then_some(&settings);
            refine(
                &DECODERS,
                &img,
                seed,
                pass,
                &opts,
                work,
                &mut next_index,
                early,
            );
        }
        work.seeds = seeds;
        work.lines = lines;
        work.stage.aggregator.finish(&settings, out);
        if opts.marks {
            let shared = ctx.shared::<MarkSet>();
            shared.marks.append(&mut work.stage.marks);
            shared.scanned = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use breader_core::{
        Binarizer, EanAddonPolicy, FrameContext, ImageBuffer, ImageView, LinearMeta, Pass,
        Preprocessor, ScanOptions, Scratch, SymbolMeta, SymbologySet,
    };

    const TEXT: &str = "5901234123457";

    struct Half;

    impl Preprocessor for Half {
        fn id(&self) -> &'static str {
            "half"
        }

        fn apply(&self, src: ImageView<'_>, dst: &mut ImageBuffer) {
            let (w, h) = (src.width() / 2, src.height() / 2);
            dst.resize(w, h);
            for y in 0..h {
                for x in 0..w {
                    let sum: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)]
                        .iter()
                        .map(|&(dx, dy)| u32::from(src.get(2 * x + dx, 2 * y + dy)))
                        .sum();
                    dst.row_mut(y)[x] = (sum / 4) as u8;
                }
            }
        }
    }

    fn scan_pass(
        img: &ImageBuffer,
        opts: &ScanOptions,
        pass: Pass,
        pre: &[Box<dyn Preprocessor>],
    ) -> Vec<Symbol> {
        let mut scratch = Scratch::new();
        let binarizers: Vec<Box<dyn Binarizer>> = Vec::new();
        let mut ctx = FrameContext::new(img.view(), opts, pre, &binarizers, &mut scratch);
        ctx.set_pass(pass);
        let mut out = Vec::new();
        LinearReader.scan_image(&mut ctx, &mut out);
        out
    }

    fn scan(img: &ImageBuffer, opts: &ScanOptions) -> Vec<Symbol> {
        let pass = Pass {
            preprocessor: None,
            binarizer: 0,
            inverted: false,
        };
        scan_pass(img, opts, pass, &[])
    }

    fn place(angle_deg: f32) -> Placement {
        Placement {
            module_px: 3.0,
            height_px: 120.0,
            center: (350.0, 260.0),
            angle_deg,
        }
    }

    fn picture(modules: &[bool], angle_deg: f32) -> ImageBuffer {
        let mut img = blank(700, 520, 230);
        draw_symbol(&mut img, modules, place(angle_deg), 30);
        img
    }

    fn angle_diff(a: f32, b: f32) -> f32 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    fn linear_meta(s: &Symbol) -> &LinearMeta {
        match &s.meta {
            SymbolMeta::Linear(m) => m,
            other => panic!("unexpected meta {other:?}"),
        }
    }

    #[test]
    fn reads_every_orientation_with_position_and_angle() {
        let modules = ean13_modules(TEXT);
        for angle in [
            0.0, 90.0, 180.0, 270.0, 12.0, 33.0, 57.0, 78.0, 104.0, 137.0, 200.0, 245.0, 303.0,
            352.0,
        ] {
            let img = picture(&modules, angle);
            let found = scan(&img, &ScanOptions::default());
            assert_eq!(found.len(), 1, "angle {angle}: {found:?}");
            let s = &found[0];
            assert_eq!(s.symbology, Symbology::Ean13);
            assert_eq!(s.data, TEXT.as_bytes());
            assert_eq!(&s.aim_id, b"]E0");
            assert!(linear_meta(s).checksum_verified);
            assert!(linear_meta(s).lines_agreeing >= 2);
            assert!(
                angle_diff(s.orientation_deg, angle) < 8.0,
                "angle {angle}: got {}",
                s.orientation_deg
            );
            let c = s.quad.center();
            assert!(
                c.distance(Point::new(350.0, 260.0)) < 20.0,
                "angle {angle}: {c:?}"
            );
            assert!((0.0..=1.0).contains(&s.quality));
        }
    }

    #[test]
    fn upca_and_ean8_report_their_own_symbology() {
        let img = picture(&upca_modules("036000291452"), 0.0);
        let found = scan(&img, &ScanOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbology, Symbology::UpcA);
        assert_eq!(found[0].data, b"036000291452");

        let img = picture(&ean8_modules("73513537"), 0.0);
        let found = scan(&img, &ScanOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbology, Symbology::Ean8);
        assert_eq!(&found[0].aim_id, b"]E4");

        let img = picture(&upce_modules("01234565"), 0.0);
        let found = scan(&img, &ScanOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbology, Symbology::UpcE);
        assert_eq!(&found[0].aim_id, b"]E0");
    }

    #[test]
    fn inverted_symbols_are_read_in_every_effort_without_an_inverted_pass() {
        let mut img = picture(&ean13_modules(TEXT), 15.0);
        invert_image(&mut img);
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            let found = scan(&img, &ScanOptions::with_effort(effort));
            assert_eq!(found.len(), 1, "{effort:?}");
            assert_eq!(found[0].data, TEXT.as_bytes());
            assert!(found[0].quad.contains(Point::new(350.0, 260.0)));
        }
        let off = ScanOptions {
            try_invert: Some(false),
            ..ScanOptions::default()
        };
        assert!(scan(&img, &off).is_empty());
        let normal = picture(&ean13_modules(TEXT), 15.0);
        assert_eq!(scan(&normal, &off).len(), 1);
    }

    #[test]
    fn inverted_passes_are_left_to_the_reader() {
        assert!(LinearReader.handles_inversion());
        let img = picture(&ean13_modules(TEXT), 0.0);
        let pass = Pass {
            preprocessor: None,
            binarizer: 0,
            inverted: true,
        };
        assert!(scan_pass(&img, &ScanOptions::default(), pass, &[]).is_empty());
    }

    #[test]
    fn dark_and_light_symbols_in_one_image_are_both_read() {
        let mut img = blank(1000, 400, 225);
        for (cx, text) in [(250.0, "5901234123457"), (750.0, "4006381333931")] {
            let p = Placement {
                module_px: 2.6,
                height_px: 100.0,
                center: (cx, 200.0),
                angle_deg: 0.0,
            };
            draw_symbol(&mut img, &ean13_modules(text), p, 30);
        }
        for y in 0..400 {
            img.row_mut(y)[500..].iter_mut().for_each(|v| *v = 255 - *v);
        }
        let found = scan(&img, &ScanOptions::default());
        let mut texts: Vec<_> = found.iter().map(|s| s.data.clone()).collect();
        texts.sort();
        assert_eq!(
            texts,
            [b"4006381333931".to_vec(), b"5901234123457".to_vec()]
        );
    }

    #[test]
    fn quad_is_reported_in_original_frame_coordinates() {
        let img = picture(&ean13_modules(TEXT), 0.0);
        let pre: Vec<Box<dyn Preprocessor>> = vec![Box::new(Half)];
        let pass = Pass {
            preprocessor: Some(0),
            binarizer: 0,
            inverted: false,
        };
        let found = scan_pass(&img, &ScanOptions::default(), pass, &pre);
        assert_eq!(found.len(), 1);
        let q = found[0].quad;
        assert!(q.contains(Point::new(350.0, 260.0)), "{q:?}");
        let width = q.corners[0].distance(q.corners[1]);
        assert!((width - 285.0).abs() < 20.0, "width {width}");
        assert!(found[0].orientation_deg.abs() < 3.0);
    }

    #[test]
    fn preprocessed_variants_are_skipped() {
        let img = picture(&ean13_modules(TEXT), 0.0);
        let pre: Vec<Box<dyn Preprocessor>> = vec![Box::new(Half)];
        let mut scratch = Scratch::new();
        let binarizers: Vec<Box<dyn Binarizer>> = Vec::new();
        let opts = ScanOptions::default();
        let mut ctx = FrameContext::new(img.view(), &opts, &pre, &binarizers, &mut scratch);
        ctx.set_pass(Pass {
            preprocessor: Some(0),
            binarizer: 0,
            inverted: false,
        });
        let mut out = Vec::new();
        LinearReader.scan(&mut ctx, &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn works_at_every_effort_and_respects_enabled_symbologies() {
        let img = picture(&ean13_modules(TEXT), 0.0);
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            let found = scan(&img, &ScanOptions::with_effort(effort));
            assert_eq!(found.len(), 1, "{effort:?}");
        }
        let other = ScanOptions {
            symbologies: SymbologySet::EMPTY
                .with(Symbology::Ean8)
                .with(Symbology::Code39),
            ..ScanOptions::default()
        };
        assert!(scan(&img, &other).is_empty());
        let qr_only = ScanOptions {
            symbologies: SymbologySet::EMPTY.with(Symbology::Qr),
            ..ScanOptions::default()
        };
        assert!(scan(&img, &qr_only).is_empty());
    }

    #[test]
    fn min_line_agreement_gates_the_result() {
        let img = picture(&ean13_modules(TEXT), 0.0);
        let with_agreement = |min_line_agreement| ScanOptions {
            min_line_agreement,
            ..ScanOptions::default()
        };
        let found = scan(&img, &with_agreement(1));
        assert_eq!(found.len(), 1);
        let lines = linear_meta(&found[0]).lines_agreeing;
        assert!(
            lines >= 2,
            "refinement adds lines around a lone hit: {lines}"
        );
        assert!(scan(&img, &with_agreement(1000)).is_empty());
    }

    #[test]
    fn addon_policy_controls_reporting() {
        let with = with_addon(ean13_modules("9780735200449"), 9, addon_modules("52495"));
        let img = picture(&with, 0.0);
        let policy = |ean_addon| ScanOptions {
            ean_addon,
            ..ScanOptions::default()
        };
        let found = scan(&img, &ScanOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(linear_meta(&found[0]).addon, None);
        assert_eq!(&found[0].aim_id, b"]E0");

        let found = scan(&img, &policy(EanAddonPolicy::Optional));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].data, b"9780735200449");
        assert_eq!(linear_meta(&found[0]).addon.as_deref(), Some("52495"));
        assert_eq!(&found[0].aim_id, b"]E3");

        assert_eq!(scan(&img, &policy(EanAddonPolicy::Required)).len(), 1);
        let plain = picture(&ean13_modules("9780735200449"), 0.0);
        assert!(scan(&plain, &policy(EanAddonPolicy::Required)).is_empty());
        let found = scan(&plain, &policy(EanAddonPolicy::Optional));
        assert_eq!(found.len(), 1);
        assert_eq!(linear_meta(&found[0]).addon, None);
    }

    #[test]
    fn several_symbols_in_one_image_are_all_reported() {
        let mut img = blank(900, 700, 225);
        let items = [
            (ean13_modules("5901234123457"), (250.0, 150.0), 0.0),
            (ean8_modules("73513537"), (650.0, 160.0), 0.0),
            (upca_modules("036000291452"), (240.0, 520.0), 90.0),
            (ean13_modules("4006381333931"), (650.0, 520.0), 180.0),
        ];
        for (modules, center, angle) in &items {
            let p = Placement {
                module_px: 2.6,
                height_px: 100.0,
                center: *center,
                angle_deg: *angle,
            };
            draw_symbol(&mut img, modules, p, 30);
        }
        let found = scan(&img, &ScanOptions::default());
        let mut texts: Vec<String> = found
            .iter()
            .map(|s| String::from_utf8_lossy(&s.data).into_owned())
            .collect();
        texts.sort();
        assert_eq!(
            texts,
            ["036000291452", "4006381333931", "5901234123457", "73513537"]
        );
    }

    #[test]
    fn a_distant_duplicate_is_a_second_symbol() {
        let mut img = blank(1000, 400, 225);
        for cx in [250.0, 750.0] {
            let p = Placement {
                module_px: 2.6,
                height_px: 100.0,
                center: (cx, 200.0),
                angle_deg: 0.0,
            };
            draw_symbol(&mut img, &ean13_modules(TEXT), p, 30);
        }
        assert_eq!(scan(&img, &ScanOptions::default()).len(), 2);
    }

    fn wide_picture(modules: &[bool], module_px: f32, angle_deg: f32) -> ImageBuffer {
        let mut img = blank(900, 600, 230);
        let p = Placement {
            module_px,
            height_px: 120.0,
            center: (450.0, 300.0),
            angle_deg,
        };
        draw_symbol(&mut img, modules, p, 30);
        img
    }

    fn code128_picture(values: &[u8], angle_deg: f32) -> ImageBuffer {
        wide_picture(&code128_modules(values), 2.6, angle_deg)
    }

    #[test]
    fn code128_reads_in_every_orientation_effort_and_polarity() {
        let mut values = vec![104];
        values.extend("Breader-128".chars().map(b));
        for angle in [0.0, 90.0, 180.0, 270.0, 17.0, 133.0, 241.0] {
            for invert in [false, true] {
                let mut img = code128_picture(&values, angle);
                if invert {
                    invert_image(&mut img);
                }
                for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
                    if effort == Effort::Fast && angle % 90.0 != 0.0 {
                        continue; // Fast scans only horizontal and vertical lines
                    }
                    let found = scan(&img, &ScanOptions::with_effort(effort));
                    assert_eq!(found.len(), 1, "angle {angle} invert {invert} {effort:?}");
                    let s = &found[0];
                    assert_eq!(s.symbology, Symbology::Code128);
                    assert_eq!(s.data, b"Breader-128");
                    assert_eq!(&s.aim_id, b"]C0");
                    assert!(linear_meta(s).checksum_verified);
                    assert!(s.quad.contains(Point::new(450.0, 300.0)));
                }
            }
        }
    }

    #[test]
    fn gs1_128_and_extended_latin1() {
        let gs1 = [
            105,
            102,
            1,
            12,
            34,
            56,
            78,
            90,
            12,
            31,
            100,
            102,
            b('1'),
            b('2'),
        ];
        let found = scan(&code128_picture(&gs1, 0.0), &ScanOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].data, b"0112345678901231\x1d12");
        assert_eq!(&found[0].aim_id, b"]C1");

        let latin = [104, b('c'), b('a'), b('f'), 100, b('i')];
        let found = scan(&code128_picture(&latin, 0.0), &ScanOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].data, [b'c', b'a', b'f', 0xe9]);
    }

    #[test]
    fn code39_reads_with_options() {
        let img = wide_picture(&code39_modules("LABEL-39", 3, 1), 2.4, 12.0);
        let found = scan(&img, &ScanOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbology, Symbology::Code39);
        assert_eq!(found[0].data, b"LABEL-39");
        assert_eq!(&found[0].aim_id, b"]A0");
        let mut inverted = img.clone();
        invert_image(&mut inverted);
        assert_eq!(
            scan(&inverted, &ScanOptions::with_effort(Effort::Fast)).len(),
            1
        );

        let full = ScanOptions {
            code39: breader_core::Code39Options {
                full_ascii: true,
                ..Default::default()
            },
            ..ScanOptions::default()
        };
        let img = wide_picture(&code39_modules("+A+B+C", 3, 1), 2.4, 0.0);
        let found = scan(&img, &full);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].data, b"abc");
        assert_eq!(&found[0].aim_id, b"]A4");
        assert_eq!(scan(&img, &ScanOptions::default())[0].data, b"+A+B+C");
    }

    #[test]
    fn itf14_needs_its_length_and_check_digit() {
        let text = with_gs1_check("0614141999996");
        let img = wide_picture(&itf_modules(&text, 3), 2.6, 7.0);
        let found = scan(&img, &ScanOptions::default());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].symbology, Symbology::Itf);
        assert_eq!(found[0].data, text.as_bytes());
        assert_eq!(&found[0].aim_id, b"]I1");
        assert!(linear_meta(&found[0]).lines_agreeing >= 3);

        let only12 = ScanOptions {
            itf: breader_core::ItfOptions {
                allowed_lengths: vec![12],
                min_length: 6,
            },
            ..ScanOptions::default()
        };
        assert!(scan(&img, &only12).is_empty(), "no partial reads");
        let any = ScanOptions {
            itf: breader_core::ItfOptions {
                allowed_lengths: vec![],
                min_length: 6,
            },
            ..ScanOptions::default()
        };
        let found = scan(&img, &any);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].data.len(), 14);

        let bad = format!("{}{}", &text[..13], (text.as_bytes()[13] - b'0' + 1) % 10);
        let img = wide_picture(&itf_modules(&bad, 3), 2.6, 7.0);
        assert!(scan(&img, &any).is_empty());
    }

    #[test]
    fn itf_default_lengths_beyond_14_read_with_four_lines_and_a_clear_margin() {
        let defaults = ScanOptions::default();
        for digits in ["12345678", "1234567890", "123456789012", "1234567890123456"] {
            let img = wide_picture(&itf_modules(digits, 3), 2.6, 0.0);
            let found = scan(&img, &defaults);
            assert_eq!(found.len(), 1, "{digits}");
            assert_eq!(found[0].data, digits.as_bytes());
            assert!(linear_meta(&found[0]).lines_agreeing >= 4, "{digits}");
        }
        let img = wide_picture(&itf_modules("123456789012", 3), 2.6, 0.0);
        let own = ScanOptions {
            itf: breader_core::ItfOptions {
                allowed_lengths: vec![12],
                min_length: 6,
            },
            ..ScanOptions::default()
        };
        assert_eq!(scan(&img, &own).len(), 1);
        // 20 digits are not on the default list.
        let img = wide_picture(&itf_modules("12345678901234567890", 3), 2.6, 0.0);
        assert!(scan(&img, &defaults).is_empty());
        // A quiet zone below 10 modules rejects the symbol whatever the length.
        let tight = with_neighbours(&itf_modules("123456789012", 3), 8, 8);
        let img = wide_picture(&tight, 2.6, 0.0);
        assert!(scan(&img, &defaults).is_empty());
    }

    #[test]
    fn databar_variants_read_in_every_orientation_with_their_meta() {
        let cases = [
            (
                databar_omni_modules("2001234567890"),
                Symbology::DataBar,
                &b"0120012345678909"[..],
                None,
            ),
            (
                databar_limited_modules("1234567890123"),
                Symbology::DataBarLimited,
                &b"0112345678901231"[..],
                None,
            ),
            (
                databar_expanded_modules("01900123456789083103001750"),
                Symbology::DataBarExpanded,
                &b"01900123456789083103001750"[..],
                Some(6),
            ),
        ];
        for (modules, symbology, data, segments) in cases {
            for angle in [0.0, 90.0, 180.0, 270.0, 23.0, 211.0] {
                for invert in [false, true] {
                    let mut img = wide_picture(&modules, 3.0, angle);
                    if invert {
                        invert_image(&mut img);
                    }
                    let found = scan(&img, &ScanOptions::default());
                    let ours: Vec<_> = found.iter().filter(|s| s.symbology == symbology).collect();
                    assert_eq!(ours.len(), 1, "{symbology:?} {angle} {invert}: {found:?}");
                    assert_eq!(ours[0].data, data);
                    assert_eq!(&ours[0].aim_id, b"]e0");
                    let meta = linear_meta(ours[0]);
                    assert!(meta.checksum_verified && meta.lines_agreeing >= 2);
                    assert_eq!(meta.databar_segments, segments);
                    assert_eq!(found.len(), 1, "{found:?}");
                }
            }
        }
    }

    fn stacked_picture(
        kind: u8,
        text: &str,
        columns: usize,
        module_px: f32,
        data: f32,
        angle_deg: f32,
    ) -> ImageBuffer {
        use crate::stack_testutil::*;
        let grid = databar_stacked_grid(kind, text, columns);
        let heights = stacked_row_heights(kind, grid.len(), data);
        let mut img = blank(1000, 1000, 230);
        let p = GridPlacement {
            heights: &heights,
            module_px,
            center: (500.0, 500.0),
            angle_deg,
            dark: 30,
        };
        draw_grid(&mut img, &grid, p);
        img
    }

    #[test]
    fn databar_stacked_reads_in_every_orientation_with_its_meta() {
        let cases = [
            (0u8, "2001234567890", 1, 0u8, 2u8),
            (1, "4412345678900", 1, 0, 2),
            (1, "0000000000001", 1, 0, 2),
        ];
        for (kind, text, columns, segments, rows) in cases {
            let data = format!("01{text}{}", check_digit(text)).into_bytes();
            for angle in [0.0, 90.0, 180.0, 270.0, 23.0, 211.0, 337.0] {
                for invert in [false, true] {
                    let mut img = stacked_picture(kind, text, columns, 4.0, 24.0, angle);
                    if invert {
                        invert_image(&mut img);
                    }
                    let found = scan(&img, &ScanOptions::default());
                    assert_eq!(found.len(), 1, "{kind} {angle} {invert}: {found:?}");
                    assert_eq!(found[0].symbology, Symbology::DataBar);
                    assert_eq!(found[0].data, data, "{kind} {angle} {invert}");
                    assert_eq!(&found[0].aim_id, b"]e0");
                    let meta = linear_meta(&found[0]);
                    assert!(meta.checksum_verified && meta.lines_agreeing >= 2);
                    assert_eq!(meta.databar_segments.unwrap_or(0), segments);
                    assert_eq!(meta.databar_rows, Some(rows));
                    let c = found[0].quad.center();
                    assert!(c.distance(Point::new(500.0, 500.0)) < 40.0, "{c:?}");
                }
            }
        }
    }

    #[test]
    fn databar_expanded_stacked_reads_every_row_layout() {
        let texts = [
            "0100012345678905",
            "01900123456789083103001750",
            "1012A",
            "010001234567890510ABC123",
            "011234567890123110UNIVERSITY-OF-DEUSTO",
            "10123456A1234A\x1d15991231",
            "1012345678901234567890123456789012345678901234567890",
            "01988987654321061599123131030017501012A\x1d422123\x1d2123456\x1d423012345678901",
        ];
        for text in texts {
            for columns in 1..=6 {
                // Rows of three or more blocks are too long for a plan line to
                // stay inside them when the symbol is turned off the plan's
                // directions.
                for angle in [0.0, 180.0, 90.0, 17.0] {
                    if angle == 17.0 && columns > 2 {
                        continue;
                    }
                    let img = stacked_picture(2, text, columns, 2.4, 34.0, angle);
                    let found = scan(&img, &ScanOptions::default());
                    let got: Vec<_> = found.iter().map(|s| s.data.clone()).collect();
                    assert_eq!(
                        got,
                        [text.as_bytes().to_vec()],
                        "{text:?} {columns} {angle}"
                    );
                    let meta = linear_meta(&found[0]);
                    assert_eq!(found[0].symbology, Symbology::DataBarExpanded);
                    assert!(meta.databar_segments.is_some());
                }
            }
        }
    }

    #[test]
    fn databar_stacked_cropped_to_its_modules_is_read() {
        use crate::stack_testutil::*;
        // The picture is the symbol and nothing else: the guard bars at the
        // ends of the rows touch the border (the zxing-cpp samples are like
        // this).
        let cases: [(u8, &str, usize, f32); 4] = [
            (0, "2001234567890", 1, 8.0),
            (1, "4412345678900", 1, 4.0),
            (2, "01900123456789083103001750", 2, 2.0),
            (2, "010001234567890510ABC123", 3, 2.0),
        ];
        for (kind, text, columns, px) in cases {
            let grid = databar_stacked_grid(kind, text, columns);
            let heights = stacked_row_heights(kind, grid.len(), 20.0);
            let (w, h) = (
                (grid[0].len() as f32 * px) as usize,
                (heights.iter().sum::<f32>() * px) as usize,
            );
            let mut img = blank(w, h, 230);
            let mut y0 = 0.0;
            for (row, &rh) in grid.iter().zip(&heights) {
                for y in (y0 * px) as usize..(((y0 + rh) * px) as usize).min(h) {
                    for x in 0..w {
                        if row[(x as f32 / px) as usize] {
                            img.row_mut(y)[x] = 30;
                        }
                    }
                }
                y0 += rh;
            }
            let found = scan(&img, &ScanOptions::default());
            assert_eq!(found.len(), 1, "{kind} {text}: {found:?}");
            let want = if kind == 2 {
                text.as_bytes().to_vec()
            } else {
                format!("01{text}{}", check_digit(text)).into_bytes()
            };
            assert_eq!(found[0].data, want, "{kind} {text}");
            assert!(linear_meta(&found[0]).databar_rows.is_some());
        }
    }

    #[test]
    fn databar_is_off_when_not_asked_for() {
        let img = wide_picture(&databar_omni_modules("2001234567890"), 3.0, 0.0);
        let others = SymbologySet::all()
            .iter()
            .filter(|s| {
                !matches!(
                    s,
                    Symbology::DataBar | Symbology::DataBarLimited | Symbology::DataBarExpanded
                )
            })
            .collect();
        let off = ScanOptions {
            symbologies: others,
            ..ScanOptions::default()
        };
        assert!(scan(&img, &off).is_empty());
        let only = ScanOptions {
            symbologies: [Symbology::DataBarLimited].into_iter().collect(),
            ..ScanOptions::default()
        };
        assert!(scan(&img, &only).is_empty());
    }

    #[test]
    fn codabar_reads_without_start_and_stop_in_the_data() {
        let img = wide_picture(&codabar_modules('B', "40129/5", 'D', 3, 1), 2.6, 7.0);
        let found = scan(&img, &ScanOptions::default());
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].symbology, Symbology::Codabar);
        assert_eq!(found[0].data, b"40129/5");
        assert_eq!(&found[0].aim_id, b"]F0");
        let meta = linear_meta(&found[0]);
        assert_eq!(meta.codabar_start_stop, Some((b'B', b'D')));
        assert!(meta.lines_agreeing >= 3);
        let off = ScanOptions {
            symbologies: SymbologySet::all()
                .iter()
                .filter(|s| *s != Symbology::Codabar)
                .collect(),
            ..ScanOptions::default()
        };
        assert!(scan(&img, &off).is_empty());
        // No stop character: nothing, and nothing else either.
        let m = codabar_modules('B', "40129/5", 'D', 3, 1);
        let img = wide_picture(&m[..m.len() - 11], 2.6, 7.0);
        assert!(scan(&img, &ScanOptions::default()).is_empty());
    }

    #[test]
    fn other_symbologies_never_come_out_as_codabar() {
        for modules in [
            code39_modules("ABC-1234", 3, 1),
            itf_modules(&with_gs1_check("0614141999996"), 3),
            ean13_modules(TEXT),
        ] {
            let img = wide_picture(&modules, 2.6, 7.0);
            for s in scan(&img, &ScanOptions::default()) {
                assert_ne!(s.symbology, Symbology::Codabar, "{s:?}");
            }
        }
    }

    #[test]
    fn mixed_symbologies_in_one_image() {
        let mut img = blank(1000, 760, 228);
        let place = |cx, cy, angle_deg| Placement {
            module_px: 2.6,
            height_px: 100.0,
            center: (cx, cy),
            angle_deg,
        };
        let c128 = code128_modules(&[104, b('M'), b('i'), b('x'), b('1')]);
        let itf = itf_modules(&with_gs1_check("1234567890123"), 3);
        draw_symbol(&mut img, &ean13_modules(TEXT), place(260.0, 150.0, 0.0), 30);
        draw_symbol(&mut img, &c128, place(740.0, 150.0, 0.0), 30);
        draw_symbol(
            &mut img,
            &code39_modules("MIXED", 3, 1),
            place(260.0, 420.0, 0.0),
            30,
        );
        draw_symbol(&mut img, &itf, place(740.0, 420.0, 0.0), 30);
        let mut names: Vec<String> = scan(&img, &ScanOptions::default())
            .iter()
            .map(|s| {
                format!(
                    "{}:{}",
                    s.symbology.name(),
                    String::from_utf8_lossy(&s.data)
                )
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "Code 128:Mix1",
                "Code 39:MIXED",
                "EAN-13:5901234123457",
                "ITF:12345678901231"
            ]
        );
    }

    #[test]
    fn degenerate_images_do_not_panic() {
        for (w, h) in [(1, 1), (3, 200), (200, 3), (59, 59), (61, 61), (64, 1)] {
            let img = blank(w, h, 100);
            for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
                assert!(scan(&img, &ScanOptions::with_effort(effort)).is_empty());
            }
        }
    }
}
