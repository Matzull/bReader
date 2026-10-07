//! Builds a custom pipeline with user-written modules.
//!
//! - `FixedThreshold`: a [`Binarizer`] with a constant threshold.
//! - `MinLength`: a [`PostProcessor`] that drops short payloads.
//! - `DarkCorner`: a toy [`SymbologyReader`] that reports a Code 39 symbol
//!   named "DARK" when the top-left pixel is dark. A real reader would detect
//!   and decode a pattern from `ctx.binary()` or `ctx.image()`.
//!
//! The ready-made QR reader runs next to them in the same pipeline.
//!
//! cargo run --example custom_module

use breader::modules::{QrReader, TextDecoder};
use breader::{
    Binarizer, BitMatrix, Content, FrameContext, ImageBuffer, ImageView, LinearMeta, Pipeline,
    PostProcessor, Quad, Reader, ScanOptions, Symbol, SymbolMeta, Symbology, SymbologyReader,
    SymbologySet,
};
use qrcode::{Color, EcLevel, QrCode};

struct FixedThreshold(u8);

impl Binarizer for FixedThreshold {
    fn id(&self) -> &'static str {
        "fixed-threshold"
    }
    fn binarize(&self, src: ImageView<'_>, out: &mut BitMatrix) {
        for y in 0..src.height() {
            for (x, &v) in src.row(y).iter().enumerate() {
                out.set(x, y, v < self.0);
            }
        }
    }
}

struct MinLength(usize);

impl PostProcessor for MinLength {
    fn id(&self) -> &'static str {
        "min-length"
    }
    fn process(&self, _ctx: &FrameContext<'_>, symbols: &mut Vec<Symbol>) {
        symbols.retain(|s| s.data.len() >= self.0);
    }
}

struct DarkCorner;

impl SymbologyReader for DarkCorner {
    fn name(&self) -> &'static str {
        "dark-corner"
    }
    fn symbologies(&self) -> SymbologySet {
        SymbologySet::EMPTY.with(Symbology::Code39)
    }
    fn scan(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>) {
        if ctx.pass().inverted || !ctx.binary().get(0, 0) {
            return;
        }
        out.push(Symbol {
            symbology: Symbology::Code39,
            data: b"DARK".to_vec(),
            text: None,
            segments: Vec::new(),
            content: Content::Unparsed,
            quad: Quad::default(),
            orientation_deg: 0.0,
            aim_id: *b"]A0",
            meta: SymbolMeta::Linear(LinearMeta::default()),
            quality: 0.0,
        });
    }
}

fn qr_frame(text: &str) -> ImageBuffer {
    let code = QrCode::with_error_correction_level(text, EcLevel::M).expect("fits in a QR code");
    let (dim, scale, quiet) = (code.width(), 4, 16);
    let side = dim * scale + 2 * quiet;
    let mut img = ImageBuffer::new(side, side);
    img.as_mut_slice().fill(235);
    let colors = code.to_colors();
    for y in 0..dim * scale {
        for x in 0..dim * scale {
            if colors[(y / scale) * dim + x / scale] == Color::Dark {
                img.row_mut(quiet + y)[quiet + x] = 20;
            }
        }
    }
    img
}

fn main() {
    let pipeline = Pipeline::builder()
        .binarizer(FixedThreshold(128))
        .reader(QrReader)
        .reader(DarkCorner)
        .post_processor(TextDecoder)
        .post_processor(MinLength(3))
        .build()
        .expect("a binarizer is registered");
    let mut reader = Reader::from_pipeline(pipeline, ScanOptions::default());

    let mut frame = qr_frame("custom pipeline");
    for symbol in reader.scan(frame.view()) {
        println!("{}: {:?}", symbol.symbology.name(), symbol.text);
    }

    // Darken the corner so the toy reader fires too; the QR code is unaffected.
    frame.row_mut(0)[..4].fill(0);
    println!("with a dark corner:");
    for symbol in reader.scan(frame.view()) {
        println!("{}: {:?}", symbol.symbology.name(), symbol.text);
    }
}
