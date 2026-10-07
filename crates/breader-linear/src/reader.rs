use breader_core::{
    Effort, FrameContext, ImageView, Point, Symbol, Symbology, SymbologyReader, SymbologySet,
};

use crate::aggregate::{Aggregator, PlacedHit, Settings};
use crate::code39::Code39Decoder;
use crate::code128::Code128Decoder;
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::ean::EanDecoder;
use crate::edges::{EdgeDetector, EdgeParams, Edges};
use crate::itf::ItfDecoder;
use crate::plan::{ScanPlan, Scanline, clip_line};
use crate::profile::{self, Oblique};
use crate::runs::Runs;

/// Decoders tried on every scanline, composed statically.
type Decoders = (EanDecoder, Code128Decoder, Code39Decoder, ItfDecoder);

const DECODERS: Decoders = (EanDecoder, Code128Decoder, Code39Decoder, ItfDecoder);

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
    stage: Stage,
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
                module: hit.module,
                quality: hit.quality,
                aim: hit.aim,
                checksum_verified: hit.checksum_verified,
                inverted: hit.inverted,
            });
        }
    }
}

/// Pre-sampled profiles of the column and oblique lines of a plan.
struct Sources<'a> {
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
    if line.from.x == line.to.x {
        let h = sources.height;
        let column = &sources.columns[cursor.column * h..(cursor.column + 1) * h];
        scan_profile(&DECODERS, column, line, index, sources.opts, stage);
    } else if Oblique::is_oblique(line) {
        let range = sources.offsets[cursor.slanted]..sources.offsets[cursor.slanted + 1];
        scan_profile(
            &DECODERS,
            &sources.oblique[range],
            line,
            index,
            sources.opts,
            stage,
        );
    } else {
        profile::sample(img, line, false, profile);
        scan_profile(&DECODERS, profile, line, index, sources.opts, stage);
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
) -> Vec<PlacedHit> {
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
    let per_chunk: Vec<Vec<PlacedHit>> = lines
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
            stage.aggregator.take_hits()
        })
        .collect();
    per_chunk.into_iter().flatten().collect()
}

/// Parallel lines on both sides of `seed`, stopping per side once the
/// symbol stops being read.
fn refine<D: LinearDecoder>(
    decoders: &D,
    img: &ImageView<'_>,
    seed: &PlacedHit,
    pass: PassImage,
    opts: &LineOptions,
    work: &mut Work,
    next_index: &mut u32,
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
                opts,
                &mut work.stage,
            );
            *next_index += 1;
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
        let mut work = Work::default();
        work.stage
            .edge_params
            .extend_from_slice(edge_variants(effort));
        plan.lines(&mut work.lines);

        let columns: Vec<usize> = work
            .lines
            .iter()
            .filter(|l| l.from.x == l.to.x)
            .map(|l| l.from.x as usize)
            .collect();
        profile::sample_columns(&img, &columns, false, &mut work.columns);

        let lines = std::mem::take(&mut work.lines);
        let oblique: Vec<Scanline> = lines
            .iter()
            .filter(|l| Oblique::is_oblique(l))
            .copied()
            .collect();
        profile::sample_oblique_batch(&img, &oblique, false, &mut work.offsets, &mut work.oblique);
        let sources = Sources {
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
            for hit in scan_lines_parallel(&img, &lines, &sources, &work.stage) {
                work.stage.aggregator.push(hit);
            }
        } else {
            scan_lines(&img, &lines, &sources, &mut work.profile, &mut work.stage);
        }
        let plan_lines = lines.len() as u32;
        let settings = Settings {
            min_agreement: min_line_agreement,
            plan_lines,
            addon,
            across_pad: plan.spacing(0.0).min(plan.spacing(90.0)) / 2.0,
            scale,
        };
        let mut next_index = plan_lines;
        let mut seeds = std::mem::take(&mut work.seeds);
        work.stage.aggregator.seeds(&settings, &mut seeds);
        for seed in seeds.iter().take(REFINE_MAX_SEEDS) {
            refine(
                &DECODERS,
                &img,
                seed,
                pass,
                &opts,
                &mut work,
                &mut next_index,
            );
        }
        work.seeds = seeds;
        work.stage.aggregator.finish(&settings, out);
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
