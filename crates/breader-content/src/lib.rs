//! Post-processing stages: de-duplication, charset decoding, GS1 parsing and
//! content classification.
//!
//! Implementation crate of breader: not covered by SemVer; depend on `breader`, which re-exports what is public (docs/API-STABILITY.md).

pub mod charset;
mod classify;
pub mod gs1;
mod text;

pub use classify::ContentClassifier;
pub use gs1::Gs1Error;
pub use text::TextDecoder;

use breader_core::{FrameContext, PostProcessor, Symbol};

/// Drops later symbols that repeat an earlier one at the same location.
#[derive(Debug, Clone, Copy, Default)]
pub struct Dedup;

impl PostProcessor for Dedup {
    fn id(&self) -> &'static str {
        "dedup"
    }

    fn process(&self, _ctx: &FrameContext<'_>, symbols: &mut Vec<Symbol>) {
        let mut kept: Vec<Symbol> = Vec::with_capacity(symbols.len());
        for symbol in symbols.drain(..) {
            let center = symbol.quad.center();
            let duplicate = kept.iter().any(|k| {
                k.symbology == symbol.symbology && k.data == symbol.data && k.quad.contains(center)
            });
            if !duplicate {
                kept.push(symbol);
            }
        }
        *symbols = kept;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use breader_core::{
        Charset, Content, DataSegment, Gs1Element, ImageView, LinearMeta, Point, Quad, ScanOptions,
        Scratch, SymbolMeta, Symbology,
    };

    fn symbol(data: &[u8], quad: Quad) -> Symbol {
        let mut symbol = Symbol::new(
            Symbology::Code128,
            data.to_vec(),
            quad,
            *b"]C0",
            SymbolMeta::Linear(LinearMeta::default()),
        );
        symbol.orientation_deg = 0.0;
        symbol
    }

    fn square(x: f32, y: f32, size: f32) -> Quad {
        Quad::new([
            Point::new(x, y),
            Point::new(x + size, y),
            Point::new(x + size, y + size),
            Point::new(x, y + size),
        ])
    }

    fn run(processor: &dyn PostProcessor, symbols: &mut Vec<Symbol>) {
        let pixels = [0u8; 4];
        let image = ImageView::from_packed(&pixels, 2, 2).unwrap();
        let options = ScanOptions::default();
        let mut scratch = Scratch::new();
        let ctx = FrameContext::new(image, &options, &[], &[], &mut scratch);
        processor.process(&ctx, symbols);
    }

    #[test]
    fn dedup_by_data_and_location() {
        let mut symbols = vec![
            symbol(b"a", square(0.0, 0.0, 10.0)),
            symbol(b"a", square(4.0, 4.0, 4.0)),
            symbol(b"a", square(50.0, 50.0, 10.0)),
            symbol(b"b", square(0.0, 0.0, 10.0)),
        ];
        run(&Dedup, &mut symbols);
        let kept: Vec<_> = symbols.iter().map(|s| s.data.clone()).collect();
        assert_eq!(kept, [b"a".to_vec(), b"a".to_vec(), b"b".to_vec()]);
        assert_eq!(symbols[1].quad, square(50.0, 50.0, 10.0));
    }

    fn with(symbology: Symbology, data: &[u8], aim: &[u8; 3]) -> Symbol {
        let mut s = symbol(data, square(0.0, 0.0, 10.0));
        s.symbology = symbology;
        s.aim_id = *aim;
        s
    }

    fn full_pipeline(mut s: Symbol) -> Symbol {
        let mut v = vec![s.clone()];
        run(&TextDecoder, &mut v);
        run(&ContentClassifier, &mut v);
        s = v.pop().unwrap();
        s
    }

    fn seg(start: usize, end: usize, charset: Charset) -> DataSegment {
        DataSegment {
            start,
            end,
            charset,
        }
    }

    #[test]
    fn text_defaults_per_symbology() {
        let c128 = full_pipeline(with(Symbology::Code128, b"caf\xe9", b"]C0"));
        assert_eq!(c128.text.as_deref(), Some("café"));
        let c39 = full_pipeline(with(Symbology::Code39, b"AB-1", b"]A0"));
        assert_eq!(c39.text.as_deref(), Some("AB-1"));
        let bad = full_pipeline(with(Symbology::Code39, b"\xe9", b"]A0"));
        assert_eq!(bad.text, None);
        assert_eq!(bad.content, Content::Binary);
        let qr = full_pipeline(with(Symbology::Qr, "héllo".as_bytes(), b"]Q1"));
        assert_eq!(qr.text.as_deref(), Some("héllo"));
        let qr = full_pipeline(with(Symbology::Qr, b"caf\xe9", b"]Q1"));
        assert_eq!(qr.text.as_deref(), Some("café"));
        let qr = full_pipeline(with(Symbology::Qr, &[0x93, 0xfa, 0x96, 0x7b], b"]Q1"));
        assert_eq!(qr.text.as_deref(), Some("日本"));
    }

    #[test]
    fn text_from_declared_segments() {
        let mut s = with(Symbology::Qr, b"AB", b"]Q1");
        s.data.extend_from_slice(&[0x93, 0xfa]);
        s.data.extend_from_slice("é".as_bytes());
        s.segments = vec![
            seg(0, 2, Charset::Ascii),
            seg(2, 4, Charset::ShiftJis),
            seg(4, 6, Charset::Eci(26)),
        ];
        assert_eq!(full_pipeline(s.clone()).text.as_deref(), Some("AB日é"));

        // Unknown ECI and binary ECI leave the text empty; data is untouched.
        for eci in [899, 12345] {
            let mut u = s.clone();
            u.segments[2].charset = Charset::Eci(eci);
            let u = full_pipeline(u);
            assert_eq!(
                (u.text, u.data.len(), u.content),
                (None, 6, Content::Binary)
            );
        }
        // Invalid sequence in a declared charset.
        let mut u = s.clone();
        u.segments[2].charset = Charset::Eci(27);
        assert_eq!(full_pipeline(u).text, None);
        // Malformed segment layouts never panic.
        for bad in [
            vec![seg(0, 9, Charset::Ascii)],
            vec![seg(1, 6, Charset::Latin1)],
            vec![seg(0, 3, Charset::Latin1)],
            vec![seg(4, 2, Charset::Latin1)],
        ] {
            let mut u = s.clone();
            u.segments = bad;
            assert_eq!(full_pipeline(u).text, None);
        }
    }

    #[test]
    fn text_keeps_existing_value() {
        let mut s = with(Symbology::Qr, b"abc", b"]Q1");
        s.text = Some("preset".into());
        assert_eq!(full_pipeline(s).text.as_deref(), Some("preset"));
    }

    #[test]
    fn classifies_plain_content() {
        let kind = |sym, data: &[u8], aim: &[u8; 3]| full_pipeline(with(sym, data, aim)).content;
        let q = Symbology::Qr;
        assert_eq!(kind(q, b"HTTPS://example.com/a?b=1", b"]Q1"), Content::Url);
        assert_eq!(kind(q, b"ftp://host/file", b"]Q1"), Content::Url);
        assert_eq!(kind(q, b"http://a b", b"]Q1"), Content::Text);
        assert_eq!(kind(q, b"http", b"]Q1"), Content::Text);
        assert_eq!(kind(q, b"hello\r\n\ttab", b"]Q1"), Content::Text);
        assert_eq!(kind(q, b"a\x1db\x1e\x04", b"]Q1"), Content::Text);
        assert_eq!(kind(q, b"bin\x00ary", b"]Q1"), Content::Binary);
        assert_eq!(kind(q, b"\x07", b"]Q1"), Content::Binary);
        assert_eq!(kind(Symbology::Code128, b"hello", b"]C0"), Content::Text);
        assert_eq!(kind(Symbology::Itf, b"1234567890", b"]I0"), Content::Text);
    }

    fn gs1(ai: &str, value: &str) -> Gs1Element {
        Gs1Element {
            ai: ai.into(),
            value: value.into(),
        }
    }

    #[test]
    fn classifies_gs1() {
        let c = full_pipeline(with(
            Symbology::Code128,
            b"0109506000134352\x1d10AB",
            b"]C1",
        ));
        assert_eq!(
            c.content,
            Content::Gs1(vec![gs1("01", "09506000134352"), gs1("10", "AB")])
        );
        // The same data without the GS1 identifier is plain text.
        let c = full_pipeline(with(Symbology::Code128, b"0109506000134352", b"]C0"));
        assert_eq!(c.content, Content::Text);
        // GS1 identifier but unparseable payload falls back.
        let c = full_pipeline(with(Symbology::Code128, b"0109506000134353", b"]C1"));
        assert_eq!(c.content, Content::Text);
        let c = full_pipeline(with(Symbology::Code128, b"99\x00", b"]C1"));
        assert_eq!(c.content, Content::Binary);
        for aim in [b"]Q3", b"]Q4"] {
            let c = full_pipeline(with(Symbology::Qr, b"17250131", aim));
            assert_eq!(c.content, Content::Gs1(vec![gs1("17", "250131")]));
        }
        for aim in [b"]d2", b"]d5"] {
            let c = full_pipeline(with(Symbology::DataMatrix, b"17250131", aim));
            assert_eq!(c.content, Content::Gs1(vec![gs1("17", "250131")]));
        }
        for aim in [b"]d1", b"]d3"] {
            let c = full_pipeline(with(Symbology::DataMatrix, b"17250131", aim));
            assert_eq!(c.content, Content::Text);
        }
        let c = full_pipeline(with(Symbology::Qr, b"17250131", b"]Q1"));
        assert_eq!(c.content, Content::Text);
    }

    #[test]
    fn retail_symbols_map_to_gtin14() {
        let c = |sym, data: &[u8]| full_pipeline(with(sym, data, b"]E0")).content;
        assert_eq!(
            c(Symbology::Ean13, b"5901234123457"),
            Content::Gs1(vec![gs1("01", "05901234123457")])
        );
        assert_eq!(
            c(Symbology::UpcA, b"036000291452"),
            Content::Gs1(vec![gs1("01", "00036000291452")])
        );
        assert_eq!(
            c(Symbology::Ean8, b"96385074"),
            Content::Gs1(vec![gs1("01", "00000096385074")])
        );
        // UPC-E 01234565 expands to UPC-A 012345000065.
        assert_eq!(
            c(Symbology::UpcE, b"01234565"),
            Content::Gs1(vec![gs1("01", "00012345000065")])
        );
        // Bad check digit: not GS1.
        assert_eq!(c(Symbology::Ean13, b"5901234123458"), Content::Text);
        assert_eq!(c(Symbology::UpcE, b"0123"), Content::Text);
    }
}
