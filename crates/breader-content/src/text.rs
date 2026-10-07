use crate::charset;
use breader_core::{Charset, FrameContext, PostProcessor, Symbol, Symbology};

/// Sets `Symbol::text` from the payload bytes and their declared charsets.
#[derive(Debug, Clone, Copy, Default)]
pub struct TextDecoder;

fn default_charset(symbology: Symbology) -> Charset {
    match symbology {
        Symbology::Code128 => Charset::Latin1,
        Symbology::Qr => Charset::Undeclared,
        Symbology::Code39
        | Symbology::Itf
        | Symbology::Ean13
        | Symbology::Ean8
        | Symbology::UpcA
        | Symbology::UpcE => Charset::Ascii,
        _ => Charset::Undeclared,
    }
}

fn decode_symbol(symbol: &Symbol) -> Option<String> {
    if symbol.segments.is_empty() {
        return charset::decode(default_charset(symbol.symbology), &symbol.data);
    }
    let mut text = String::with_capacity(symbol.data.len());
    let mut at = 0;
    for seg in &symbol.segments {
        if seg.start != at || seg.end < seg.start {
            return None;
        }
        text.push_str(&charset::decode(
            seg.charset,
            symbol.data.get(seg.start..seg.end)?,
        )?);
        at = seg.end;
    }
    (at == symbol.data.len()).then_some(text)
}

impl PostProcessor for TextDecoder {
    fn id(&self) -> &'static str {
        "text"
    }

    fn process(&self, _ctx: &FrameContext<'_>, symbols: &mut Vec<Symbol>) {
        for symbol in symbols {
            if symbol.text.is_none() {
                symbol.text = decode_symbol(symbol);
            }
        }
    }
}
