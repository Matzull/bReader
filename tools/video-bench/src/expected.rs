use std::collections::BTreeMap;
use std::path::Path;

use benchlib::groundtruth::map_symbology;
use benchlib::matching::canonical;

use crate::metrics::Key;

#[derive(Debug, Default)]
pub struct Expected {
    /// Supported codes: key to number of physical copies.
    pub codes: BTreeMap<Key, usize>,
    pub unsupported: usize,
}

/// Splits one CSV line into at most two fields; the second may be quoted.
fn split(line: &str) -> Option<(&str, String)> {
    let (fmt, rest) = line.split_once(',')?;
    let rest = rest.trim();
    let value = match rest.strip_prefix('"') {
        Some(inner) => inner
            .strip_suffix('"')
            .unwrap_or(inner)
            .replace("\"\"", "\""),
        None => rest.to_owned(),
    };
    // Some labels carry literal (straight or typographic) quotes around the payload.
    Some((
        fmt,
        value
            .trim_matches(|c| matches!(c, '"' | '\u{201c}' | '\u{201d}'))
            .to_owned(),
    ))
}

pub fn parse(text: &str) -> Expected {
    let text = text.trim_start_matches('\u{feff}');
    let mut out = Expected::default();
    for line in text.lines().skip(1) {
        let Some((fmt, value)) = split(line.trim_end_matches('\r')) else {
            continue;
        };
        match map_symbology(fmt) {
            Some(sym) => {
                let (s, data) = canonical(sym, value.as_bytes());
                *out.codes.entry((s.name(), data)).or_default() += 1;
            }
            None => out.unsupported += 1,
        }
    }
    out
}

/// Label corrections: (video, listed value, symbology, actual payload).
/// The CSV of `wide_to_close_datamatrix_1080p` lists the UPC-A as `123457347`;
/// every engine reads the checksum-valid UPC-A `000123457347` there, so the
/// label lost its leading zeros (spreadsheet number coercion).
const OVERRIDES: &[(&str, &str, &str)] = &[(
    "wide_to_close_datamatrix_1080p",
    "123457347",
    "000123457347",
)];

/// Labels the CSV lacks: (video, format, payload, reason). Each entry was
/// verified visually on frame crops; the reason says how.
const ADDITIONS: &[(&str, &str, &str, &str)] = &[];

pub fn load(path: &Path) -> Result<Expected, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut text = text;
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    for (video, from, to) in OVERRIDES {
        if *video == stem {
            text = text.replace(from, to);
        }
    }
    let mut expected = parse(&text);
    add_labels(&mut expected, stem, ADDITIONS);
    Ok(expected)
}

fn add_labels(expected: &mut Expected, video: &str, additions: &[(&str, &str, &str, &str)]) {
    for (v, format, value, _reason) in additions {
        if *v == video
            && let Some(sym) = map_symbology(format)
        {
            let (s, data) = canonical(sym, value.as_bytes());
            *expected.codes.entry((s.name(), data)).or_default() += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bom_nbsp_quotes_and_unsupported() {
        let e = parse(
            "\u{feff}Format,Value\nEAN_13,\u{a0}2004567540106\nEAN_13,\"\"\"3605972842046\"\"\"\nMAXICODE\u{a0},x\nDATAMATRIX\u{a0},y\nUPC_A,\"123457347\"\nDataMatrix,\u{201c}ae-1534\u{201d}",
        );
        assert_eq!(e.unsupported, 1);
        assert_eq!(e.codes.len(), 5);
        assert!(e.codes.keys().any(|k| k.1 == b"ae-1534"));
        assert!(e.codes.keys().any(|k| k.1 == b"2004567540106"));
        assert!(e.codes.keys().any(|k| k.1 == b"3605972842046"));
    }

    #[test]
    fn additions_apply_to_their_video_only() {
        let extra = [("clip", "PDF417", "hello", "verified on a crop")];
        let mut e = parse("Format,Value\n");
        add_labels(&mut e, "other", &extra);
        assert!(e.codes.is_empty());
        add_labels(&mut e, "clip", &extra);
        assert_eq!(e.codes.len(), 1);
    }

    #[test]
    fn code_93_labels_are_supported_in_either_spelling() {
        let e = parse("Format,Value\nCODE_93,TEST93\nCode93,OTHER\n");
        assert_eq!((e.codes.len(), e.unsupported), (2, 0));
        assert!(e.codes.keys().all(|k| k.0 == "Code 93"));
    }

    #[test]
    fn repeated_values_count_copies() {
        let e = parse("Format,Value\nQR_CODE,a\nQR_CODE,a\n");
        assert_eq!(e.codes.values().copied().collect::<Vec<_>>(), vec![2]);
    }
}
