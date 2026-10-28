#![no_main]

use breader_content::{ContentClassifier, Dedup, TextDecoder};
use breader_core::{
    Charset, Content, DataSegment, FrameContext, ImageView, LinearMeta, Point, PostProcessor,
    Quad, ScanOptions, Scratch, Symbol, SymbolMeta, Symbology,
};
use libfuzzer_sys::fuzz_target;

fn symbol(bytes: &[u8]) -> Option<Symbol> {
    let [sym, aim, nseg, rest @ ..] = bytes else {
        return None;
    };
    let symbology = Symbology::ALL[usize::from(*sym) % Symbology::ALL.len()];
    let data = rest.to_vec();
    let mut segments = Vec::new();
    // Segment boundaries come from the leading bytes; they may be invalid
    // on purpose (gaps, overlaps, out of range).
    let mut at = 0usize;
    for i in 0..usize::from(*nseg % 4) {
        let len = usize::from(*rest.get(i).unwrap_or(&0)) % (data.len() + 2);
        let end = at + len;
        let charset = match rest.get(i + 4).copied().unwrap_or(0) % 5 {
            0 => Charset::Ascii,
            1 => Charset::Latin1,
            2 => Charset::ShiftJis,
            3 => Charset::Eci(u32::from(*rest.get(i + 8).unwrap_or(&0))),
            _ => Charset::Undeclared,
        };
        segments.push(DataSegment {
            start: at,
            end,
            charset,
        });
        at = end;
    }
    let p = Point::new(0.0, 0.0);
    let mut symbol = Symbol::new(
        symbology,
        data,
        Quad::new([p; 4]),
        [b']', b'A' + aim % 26, b'0' + aim % 10],
        SymbolMeta::Linear(LinearMeta::default()),
    );
    symbol.segments = segments;
    symbol.orientation_deg = 0.0;
    symbol.quality = 0.5;
    Some(symbol)
}

fuzz_target!(|data: &[u8]| {
    let Some(first) = symbol(data) else { return };
    // A second symbol shares the payload at the same place to exercise Dedup.
    let mut symbols = vec![first.clone(), first];
    let pixels = [0u8; 64];
    let view = ImageView::from_packed(&pixels, 8, 8).unwrap();
    let options = ScanOptions::default();
    let mut scratch = Scratch::new();
    let ctx = FrameContext::new(view, &options, &[], &[], &mut scratch);
    Dedup.process(&ctx, &mut symbols);
    TextDecoder.process(&ctx, &mut symbols);
    ContentClassifier.process(&ctx, &mut symbols);
    for s in &symbols {
        if let Some(t) = &s.text {
            assert!(std::str::from_utf8(t.as_bytes()).is_ok());
        }
        if let Content::Gs1(el) = &s.content {
            assert!(!el.is_empty());
        }
    }
});
