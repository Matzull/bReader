use std::time::Instant;

use crate::{
    Binarizer, Effort, Error, Escalation, FrameContext, ImageView, Pass, Point, PostProcessor,
    Preprocessor, ScanOptions, Scratch, Symbol, SymbologyReader,
};

/// Collects the modules of a [`Pipeline`].
#[derive(Default)]
pub struct PipelineBuilder {
    preprocessors: Vec<Box<dyn Preprocessor>>,
    binarizers: Vec<Box<dyn Binarizer>>,
    readers: Vec<Box<dyn SymbologyReader>>,
    post: Vec<Box<dyn PostProcessor>>,
    upscale: Option<Box<dyn Preprocessor>>,
}

impl std::fmt::Debug for PipelineBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineBuilder")
            .field(
                "preprocessors",
                &self
                    .preprocessors
                    .iter()
                    .map(|p| p.id())
                    .collect::<Vec<_>>(),
            )
            .field(
                "binarizers",
                &self.binarizers.iter().map(|b| b.id()).collect::<Vec<_>>(),
            )
            .field(
                "readers",
                &self.readers.iter().map(|r| r.name()).collect::<Vec<_>>(),
            )
            .field(
                "post",
                &self.post.iter().map(|p| p.id()).collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl PipelineBuilder {
    /// Creates an empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Binarizers are tried in registration order; the first is the primary one.
    pub fn binarizer(mut self, b: impl Binarizer + 'static) -> Self {
        self.binarizers.push(Box::new(b));
        self
    }

    /// Adds a preprocessor; used by `Balanced` (first only) and `Exhaustive` (all).
    pub fn preprocessor(mut self, p: impl Preprocessor + 'static) -> Self {
        self.preprocessors.push(Box::new(p));
        self
    }

    /// Sets the preprocessor of the up-scale retry: when no regular pass finds
    /// a symbol in a small frame (see [`ScanOptions::upscale_max_side`]), the
    /// frame is enlarged with it and scanned once more. It is not part of the
    /// regular pass list.
    pub fn upscale(mut self, p: impl Preprocessor + 'static) -> Self {
        self.upscale = Some(Box::new(p));
        self
    }

    /// Adds a symbology reader; readers run in registration order on every pass.
    pub fn reader(mut self, r: impl SymbologyReader + 'static) -> Self {
        self.readers.push(Box::new(r));
        self
    }

    /// Adds a post-processor; they run in registration order after the last pass.
    pub fn post_processor(mut self, p: impl PostProcessor + 'static) -> Self {
        self.post.push(Box::new(p));
        self
    }

    /// Finishes the pipeline. Fails with [`Error::NoBinarizer`] without a binarizer.
    pub fn build(self) -> Result<Pipeline, Error> {
        if self.binarizers.is_empty() {
            return Err(Error::NoBinarizer);
        }
        Ok(Pipeline {
            preprocessors: self.preprocessors,
            binarizers: self.binarizers,
            readers: self.readers,
            post: self.post,
            upscale: self.upscale,
        })
    }
}

/// Immutable set of modules that turns an image into symbols.
/// It holds no per-scan state, so one pipeline can serve many threads.
pub struct Pipeline {
    preprocessors: Vec<Box<dyn Preprocessor>>,
    binarizers: Vec<Box<dyn Binarizer>>,
    readers: Vec<Box<dyn SymbologyReader>>,
    post: Vec<Box<dyn PostProcessor>>,
    upscale: Option<Box<dyn Preprocessor>>,
}

impl std::fmt::Debug for Pipeline {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pipeline")
            .field(
                "readers",
                &self.readers.iter().map(|r| r.name()).collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// Whether the time budget of `options` is spent; `start` is set exactly when a budget is.
fn over_budget(options: &ScanOptions, start: Option<Instant>) -> bool {
    match (options.time_budget, start) {
        (Some(budget), Some(start)) => start.elapsed() >= budget,
        _ => false,
    }
}

impl Pipeline {
    /// Same as [`PipelineBuilder::new`].
    pub fn builder() -> PipelineBuilder {
        PipelineBuilder::new()
    }

    /// Ordered passes for the given options. Cheap passes first so the
    /// common case exits early.
    pub fn passes(&self, options: &ScanOptions) -> Vec<Pass> {
        self.pass_iter(options).collect()
    }

    /// The passes of [`Pipeline::passes`] without collecting them.
    fn pass_iter(&self, options: &ScanOptions) -> impl Iterator<Item = Pass> + use<> {
        let invert = options.invert_enabled();
        let (binarizers, preprocessors) = match options.effort {
            Effort::Fast => (1, 0),
            Effort::Balanced => (
                self.binarizers.len().min(2),
                self.preprocessors.len().min(1),
            ),
            Effort::Exhaustive => (self.binarizers.len(), self.preprocessors.len()),
        };
        (0..=preprocessors).flat_map(move |image| {
            (0..binarizers).flat_map(move |binarizer| {
                [false, true]
                    .into_iter()
                    .filter(move |&inverted| !inverted || invert)
                    .map(move |inverted| Pass {
                        preprocessor: image.checked_sub(1),
                        binarizer,
                        inverted,
                    })
            })
        })
    }

    /// Scans `image` and returns the symbols found, in discovery order.
    /// `scratch` carries reusable buffers between calls.
    ///
    /// When every regular pass comes back empty and the frame is small
    /// enough, one final conditional pass rescans an up-scaled copy; the
    /// symbols it finds are mapped back to the coordinates of `image`.
    pub fn scan(
        &self,
        image: ImageView<'_>,
        options: &ScanOptions,
        scratch: &mut Scratch,
    ) -> Vec<Symbol> {
        // The clock is only read when a budget is set (std's `Instant` panics on wasm32-unknown-unknown).
        let start = options.time_budget.map(|_| Instant::now());
        let mut upscaled = std::mem::take(&mut scratch.upscaled);
        let mut inner = scratch.retry.take().unwrap_or_default();
        let mut ctx = FrameContext::new(
            image,
            options,
            &self.preprocessors,
            &self.binarizers,
            scratch,
        );
        let mut symbols = Vec::new();
        self.run_passes(&mut ctx, start, &mut symbols);

        if let Some(up) = &self.upscale {
            let limit = options.upscale_max_side();
            let small = image.width().max(image.height()) <= limit;
            let in_budget = !over_budget(options, start);
            if small && in_budget && symbols.is_empty() {
                up.apply(image, &mut upscaled);
                let (sx, sy) = (
                    image.width() as f32 / upscaled.width() as f32,
                    image.height() as f32 / upscaled.height() as f32,
                );
                let mut big = FrameContext::new(
                    upscaled.view(),
                    options,
                    &self.preprocessors,
                    &self.binarizers,
                    &mut inner,
                );
                self.run_passes(&mut big, start, &mut symbols);
                for s in &mut symbols {
                    for c in &mut s.quad.corners {
                        *c = Point::new(c.x * sx, c.y * sy);
                    }
                }
            }
        }

        for p in &self.post {
            p.process(&ctx, &mut symbols);
        }
        scratch.upscaled = upscaled;
        scratch.retry = Some(inner);
        symbols
    }

    /// Runs the regular passes of `ctx`, appending to `symbols`.
    fn run_passes(
        &self,
        ctx: &mut FrameContext<'_>,
        start: Option<Instant>,
        symbols: &mut Vec<Symbol>,
    ) {
        let options = ctx.options();
        let mut found = 0;
        for pass in self.pass_iter(options) {
            if over_budget(options, start) {
                break;
            }
            ctx.set_pass(pass);
            // Passes that differ only in the binarizer share the gray image,
            // and the first binarizer comes first for every image variant.
            let gray_new = pass.binarizer == 0;
            for reader in &self.readers {
                if !reader.symbologies().intersects(options.symbologies)
                    || (reader.handles_inversion() && pass.inverted)
                {
                    continue;
                }
                if reader.uses_binary() || gray_new {
                    reader.scan(ctx, symbols);
                }
            }
            symbols.retain(|s| options.symbologies.contains(s.symbology));
            if options.max_symbols != 0 && symbols.len() >= options.max_symbols {
                symbols.truncate(options.max_symbols);
                break;
            }
            let found_before = std::mem::replace(&mut found, symbols.len());
            let stop = match options.escalation() {
                Escalation::All => false,
                Escalation::UntilFound => found > 0,
                Escalation::UntilStable => found > 0 && found == found_before,
            };
            if stop {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BitMatrix, Content, LinearMeta, Quad, SymbolMeta, Symbology, SymbologySet};

    struct Threshold;
    impl Binarizer for Threshold {
        fn id(&self) -> &'static str {
            "threshold"
        }
        fn binarize(&self, src: ImageView<'_>, out: &mut BitMatrix) {
            for y in 0..src.height() {
                for (x, &v) in src.row(y).iter().enumerate() {
                    out.set(x, y, v < 128);
                }
            }
        }
    }

    /// Reports a symbol when pixel (0, 0) is dark in the current pass.
    struct Probe;
    impl SymbologyReader for Probe {
        fn name(&self) -> &'static str {
            "probe"
        }
        fn symbologies(&self) -> SymbologySet {
            SymbologySet::EMPTY.with(Symbology::Code39)
        }
        fn scan(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
            let inverted = ctx.pass().inverted;
            if ctx.binary().get(0, 0) {
                out.push(Symbol {
                    symbology: Symbology::Code39,
                    data: vec![inverted as u8],
                    text: None,
                    segments: Vec::new(),
                    content: Content::Unparsed,
                    quad: Quad::default(),
                    orientation_deg: 0.0,
                    aim_id: *b"]A0",
                    meta: SymbolMeta::Linear(LinearMeta::default()),
                    quality: 1.0,
                });
            }
        }
    }

    struct GrayCounter(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl SymbologyReader for GrayCounter {
        fn name(&self) -> &'static str {
            "gray"
        }
        fn symbologies(&self) -> SymbologySet {
            SymbologySet::EMPTY.with(Symbology::Ean13)
        }
        fn uses_binary(&self) -> bool {
            false
        }
        fn scan(&self, _ctx: &mut FrameContext<'_>, _out: &mut Vec<Symbol>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[test]
    fn gray_reader_skips_binarizer_only_passes() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pipeline = Pipeline::builder()
            .binarizer(Threshold)
            .binarizer(Threshold)
            .reader(GrayCounter(calls.clone()))
            .build()
            .unwrap();
        let pixels = [0u8; 4];
        let img = ImageView::from_packed(&pixels, 2, 2).unwrap();
        let options = ScanOptions::with_effort(Effort::Exhaustive);
        assert_eq!(pipeline.passes(&options).len(), 4);
        pipeline.scan(img, &options, &mut Scratch::new());
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    struct BothPolarities(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl SymbologyReader for BothPolarities {
        fn name(&self) -> &'static str {
            "both"
        }
        fn symbologies(&self) -> SymbologySet {
            SymbologySet::EMPTY.with(Symbology::Ean13)
        }
        fn handles_inversion(&self) -> bool {
            true
        }
        fn scan(&self, ctx: &mut FrameContext<'_>, _out: &mut Vec<Symbol>) {
            assert!(!ctx.pass().inverted);
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    #[test]
    fn inversion_handling_reader_skips_inverted_passes() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pipeline = Pipeline::builder()
            .binarizer(Threshold)
            .reader(BothPolarities(calls.clone()))
            .build()
            .unwrap();
        let pixels = [0u8; 4];
        let img = ImageView::from_packed(&pixels, 2, 2).unwrap();
        pipeline.scan(img, &ScanOptions::default(), &mut Scratch::new());
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn requires_binarizer() {
        assert_eq!(Pipeline::builder().build().unwrap_err(), Error::NoBinarizer);
    }

    #[test]
    fn inverted_pass_runs_when_enabled() {
        let pipeline = Pipeline::builder()
            .binarizer(Threshold)
            .reader(Probe)
            .build()
            .unwrap();
        let pixels = [255u8; 4];
        let img = ImageView::from_packed(&pixels, 2, 2).unwrap();
        let mut scratch = Scratch::new();

        let fast = ScanOptions::with_effort(Effort::Fast);
        assert!(pipeline.scan(img, &fast, &mut scratch).is_empty());

        let balanced = ScanOptions::with_effort(Effort::Balanced);
        let found = pipeline.scan(img, &balanced, &mut scratch);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].data, vec![1]);
    }

    #[test]
    fn disabled_symbology_skips_reader() {
        let pipeline = Pipeline::builder()
            .binarizer(Threshold)
            .reader(Probe)
            .build()
            .unwrap();
        let pixels = [0u8; 4];
        let img = ImageView::from_packed(&pixels, 2, 2).unwrap();
        let options = ScanOptions {
            symbologies: SymbologySet::EMPTY.with(Symbology::Qr),
            ..ScanOptions::default()
        };
        assert!(pipeline.scan(img, &options, &mut Scratch::new()).is_empty());
    }

    /// Triples every pixel.
    struct Triple;
    impl Preprocessor for Triple {
        fn id(&self) -> &'static str {
            "triple"
        }
        fn apply(&self, src: ImageView<'_>, dst: &mut crate::ImageBuffer) {
            dst.resize(src.width() * 3, src.height() * 3);
            for y in 0..dst.height() {
                for x in 0..dst.width() {
                    dst.row_mut(y)[x] = src.get(x / 3, y / 3);
                }
            }
        }
    }

    /// Reports a symbol only on frames at least 60 px wide, with its corners
    /// at (30, 30)..(60, 60) of the frame it was given.
    struct OnlyBig(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl SymbologyReader for OnlyBig {
        fn name(&self) -> &'static str {
            "only-big"
        }
        fn symbologies(&self) -> SymbologySet {
            SymbologySet::EMPTY.with(Symbology::Code39)
        }
        fn scan(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if ctx.image().width() < 60 || !out.is_empty() {
                return;
            }
            out.push(Symbol {
                symbology: Symbology::Code39,
                data: vec![1],
                text: None,
                segments: Vec::new(),
                content: Content::Unparsed,
                quad: Quad::new([
                    Point::new(30.0, 30.0),
                    Point::new(60.0, 30.0),
                    Point::new(60.0, 60.0),
                    Point::new(30.0, 60.0),
                ]),
                orientation_deg: 0.0,
                aim_id: *b"]A0",
                meta: SymbolMeta::Linear(LinearMeta::default()),
                quality: 1.0,
            });
        }
    }

    fn retry_pipeline() -> (Pipeline, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let pipeline = Pipeline::builder()
            .binarizer(Threshold)
            .upscale(Triple)
            .reader(OnlyBig(calls.clone()))
            .build()
            .unwrap();
        (pipeline, calls)
    }

    #[test]
    fn upscale_retry_maps_geometry_back() {
        let (pipeline, _) = retry_pipeline();
        let pixels = [255u8; 24 * 20];
        let img = ImageView::from_packed(&pixels, 24, 20).unwrap();
        let mut scratch = Scratch::new();
        for _ in 0..2 {
            let found = pipeline.scan(img, &ScanOptions::default(), &mut scratch);
            assert_eq!(found.len(), 1);
            let c = found[0].quad.corners;
            assert_eq!((c[0].x, c[0].y), (10.0, 10.0));
            assert_eq!((c[2].x, c[2].y), (20.0, 20.0));
        }
    }

    #[test]
    fn upscale_retry_respects_option_and_size() {
        let (pipeline, calls) = retry_pipeline();
        let pixels = [255u8; 24 * 20];
        let img = ImageView::from_packed(&pixels, 24, 20).unwrap();
        let mut scratch = Scratch::new();
        let with = |upscale_retry| ScanOptions {
            upscale_retry,
            ..ScanOptions::default()
        };
        assert!(pipeline.scan(img, &with(Some(0)), &mut scratch).is_empty());
        assert!(pipeline.scan(img, &with(Some(23)), &mut scratch).is_empty());
        assert_eq!(pipeline.scan(img, &with(Some(24)), &mut scratch).len(), 1);
        let fast = ScanOptions::with_effort(Effort::Fast);
        assert!(pipeline.scan(img, &fast, &mut scratch).is_empty());
        let fast_on = ScanOptions {
            upscale_retry: Some(600),
            ..fast
        };
        assert_eq!(pipeline.scan(img, &fast_on, &mut scratch).len(), 1);

        // A frame that already yields a symbol is not rescanned.
        let wide = [255u8; 60 * 20];
        let img = ImageView::from_packed(&wide, 60, 20).unwrap();
        calls.store(0, std::sync::atomic::Ordering::Relaxed);
        let found = pipeline.scan(img, &ScanOptions::default(), &mut scratch);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].quad.corners[0].x, 30.0);
        // Default effort: one binarizer, two polarities, one reader call each.
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }
}
