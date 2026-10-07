use std::time::Instant;

use crate::{
    Binarizer, Effort, Error, Escalation, FrameContext, ImageView, Pass, PostProcessor,
    Preprocessor, ScanOptions, Scratch, Symbol, SymbologyReader,
};

/// Collects the modules of a [`Pipeline`].
#[derive(Default)]
pub struct PipelineBuilder {
    preprocessors: Vec<Box<dyn Preprocessor>>,
    binarizers: Vec<Box<dyn Binarizer>>,
    readers: Vec<Box<dyn SymbologyReader>>,
    post: Vec<Box<dyn PostProcessor>>,
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

impl Pipeline {
    /// Same as [`PipelineBuilder::new`].
    pub fn builder() -> PipelineBuilder {
        PipelineBuilder::new()
    }

    /// Ordered passes for the given options. Cheap passes first so the
    /// common case exits early.
    pub fn passes(&self, options: &ScanOptions) -> Vec<Pass> {
        let invert = options.invert_enabled();
        let (binarizers, preprocessors) = match options.effort {
            Effort::Fast => (1, 0),
            Effort::Balanced => (
                self.binarizers.len().min(2),
                self.preprocessors.len().min(1),
            ),
            Effort::Exhaustive => (self.binarizers.len(), self.preprocessors.len()),
        };
        let images = std::iter::once(None).chain((0..preprocessors).map(Some));
        let mut passes = Vec::new();
        for preprocessor in images {
            for binarizer in 0..binarizers {
                for inverted in [false, true] {
                    if inverted && !invert {
                        continue;
                    }
                    passes.push(Pass {
                        preprocessor,
                        binarizer,
                        inverted,
                    });
                }
            }
        }
        passes
    }

    /// Scans `image` and returns the symbols found, in discovery order.
    /// `scratch` carries reusable buffers between calls.
    pub fn scan(
        &self,
        image: ImageView<'_>,
        options: &ScanOptions,
        scratch: &mut Scratch,
    ) -> Vec<Symbol> {
        let start = Instant::now();
        let mut ctx = FrameContext::new(
            image,
            options,
            &self.preprocessors,
            &self.binarizers,
            scratch,
        );
        let mut symbols = Vec::new();
        let readers: Vec<&dyn SymbologyReader> = self
            .readers
            .iter()
            .map(|r| r.as_ref())
            .filter(|r| r.symbologies().intersects(options.symbologies))
            .collect();

        let mut gray_done: Vec<(Option<usize>, bool)> = Vec::new();
        let mut found = 0;
        for pass in self.passes(options) {
            if options.time_budget.is_some_and(|b| start.elapsed() >= b) {
                break;
            }
            ctx.set_pass(pass);
            let gray_key = (pass.preprocessor, pass.inverted);
            let gray_new = !gray_done.contains(&gray_key);
            if gray_new {
                gray_done.push(gray_key);
            }
            for reader in &readers {
                if reader.handles_inversion() && pass.inverted {
                    continue;
                }
                if reader.uses_binary() || gray_new {
                    reader.scan(&mut ctx, &mut symbols);
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

        for p in &self.post {
            p.process(&ctx, &mut symbols);
        }
        symbols
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
}
