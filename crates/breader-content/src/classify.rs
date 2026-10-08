use crate::gs1;
use breader_core::{Content, FrameContext, Gs1Element, PostProcessor, Symbol, Symbology};

/// Sets `Symbol::content`: GS1, URL, text or binary. A parse failure never
/// rejects a symbol, it only lowers the classification.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContentClassifier;

fn is_url(text: &str) -> bool {
    let starts = |prefix: &str| {
        text.get(..prefix.len())
            .is_some_and(|head| head.eq_ignore_ascii_case(prefix))
    };
    (starts("http://") || starts("https://") || starts("ftp://"))
        && !text.contains(char::is_whitespace)
}

fn is_binary(text: Option<&str>) -> bool {
    text.is_none_or(|t| {
        t.chars()
            .any(|c| c.is_control() && !matches!(c, '\t' | '\r' | '\n' | '\x1d' | '\x1e' | '\x04'))
    })
}

/// 12-digit UPC-A form of an 8-digit UPC-E.
fn upce_to_upca(d: &[u8]) -> Option<Vec<u8>> {
    let [ns, d @ .., check] = d else { return None };
    if d.len() != 6 {
        return None;
    }
    let mut a = vec![*ns];
    match d[5] {
        b'0'..=b'2' => {
            a.extend_from_slice(&d[..2]);
            a.push(d[5]);
            a.extend_from_slice(b"0000");
            a.extend_from_slice(&d[2..5]);
        }
        b'3' => {
            a.extend_from_slice(&d[..3]);
            a.extend_from_slice(b"00000");
            a.extend_from_slice(&d[3..5]);
        }
        b'4' => {
            a.extend_from_slice(&d[..4]);
            a.extend_from_slice(b"00000");
            a.push(d[4]);
        }
        _ => {
            a.extend_from_slice(&d[..5]);
            a.extend_from_slice(b"0000");
            a.push(d[5]);
        }
    }
    a.push(*check);
    Some(a)
}

fn retail_gtin(symbol: &Symbol) -> Option<Vec<Gs1Element>> {
    let digits = match symbol.symbology {
        Symbology::Ean13 | Symbology::Ean8 | Symbology::UpcA => symbol.data.clone(),
        Symbology::UpcE => upce_to_upca(&symbol.data)?,
        _ => return None,
    };
    if digits.len() > 14 {
        return None;
    }
    let mut s = String::from("01");
    for _ in digits.len()..14 {
        s.push('0');
    }
    s.push_str(std::str::from_utf8(&digits).ok()?);
    gs1::parse(&s).ok()
}

fn gs1_content(symbol: &Symbol) -> Option<Vec<Gs1Element>> {
    match &symbol.aim_id {
        b"]C1" | b"]Q3" | b"]Q4" | b"]d2" | b"]d5" => {
            gs1::parse(std::str::from_utf8(&symbol.data).ok()?).ok()
        }
        _ => retail_gtin(symbol),
    }
}

impl PostProcessor for ContentClassifier {
    fn id(&self) -> &'static str {
        "content"
    }

    fn process(&self, _ctx: &FrameContext<'_>, symbols: &mut Vec<Symbol>) {
        for symbol in symbols {
            if symbol.content != Content::Unparsed {
                continue;
            }
            symbol.content = if let Some(elements) = gs1_content(symbol) {
                Content::Gs1(elements)
            } else if is_binary(symbol.text.as_deref()) {
                Content::Binary
            } else if symbol.text.as_deref().is_some_and(is_url) {
                Content::Url
            } else {
                Content::Text
            };
        }
    }
}
