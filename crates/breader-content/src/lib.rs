//! Post-processing stages: payload classification and de-duplication.

use breader_core::{Content, FrameContext, PostProcessor, Symbol};

#[derive(Debug, Clone, Copy, Default)]
pub struct ContentClassifier;

fn is_url(text: &str) -> bool {
    let starts = |prefix: &str| {
        text.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    starts("http://") || starts("https://")
}

impl PostProcessor for ContentClassifier {
    fn id(&self) -> &'static str {
        "content"
    }

    fn process(&self, _ctx: &FrameContext<'_>, symbols: &mut Vec<Symbol>) {
        for symbol in symbols {
            if symbol.text.is_none() {
                symbol.text = std::str::from_utf8(&symbol.data).ok().map(str::to_owned);
            }
            if symbol.content != Content::Unparsed {
                continue;
            }
            symbol.content = match symbol.text.as_deref() {
                Some(text) if is_url(text) => Content::Url,
                Some(_) => Content::Text,
                None => Content::Binary,
            };
        }
    }
}

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
        ImageView, LinearMeta, Point, Quad, ScanOptions, Scratch, SymbolMeta, Symbology,
    };

    fn symbol(data: &[u8], quad: Quad) -> Symbol {
        Symbol {
            symbology: Symbology::Code128,
            data: data.to_vec(),
            text: None,
            content: Content::Unparsed,
            quad,
            orientation_deg: 0.0,
            aim_id: *b"]C0",
            meta: SymbolMeta::Linear(LinearMeta::default()),
            quality: 1.0,
        }
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
    fn classifies_content() {
        let q = square(0.0, 0.0, 10.0);
        let mut symbols = vec![
            symbol(b"HTTPS://example.com", q),
            symbol(b"hello", q),
            symbol(&[0xff, 0xfe], q),
            symbol(b"http", q),
            symbol("héllo".as_bytes(), q),
        ];
        let mut gs1 = symbol(b"(01)1", q);
        gs1.content = Content::Gs1(Vec::new());
        symbols.push(gs1);
        let mut preset = symbol(b"http://x", q);
        preset.content = Content::Text;
        symbols.push(preset);

        run(&ContentClassifier, &mut symbols);

        assert_eq!(symbols[0].content, Content::Url);
        assert_eq!(symbols[0].text.as_deref(), Some("HTTPS://example.com"));
        assert_eq!(symbols[1].content, Content::Text);
        assert_eq!(symbols[2].content, Content::Binary);
        assert_eq!(symbols[2].text, None);
        assert_eq!(symbols[3].content, Content::Text);
        assert_eq!(symbols[4].content, Content::Text);
        assert_eq!(symbols[5].content, Content::Gs1(Vec::new()));
        assert_eq!(symbols[6].content, Content::Text);
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
}
