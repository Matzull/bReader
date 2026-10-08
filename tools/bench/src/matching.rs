use std::collections::BTreeMap;
use std::time::Duration;

use breader::Symbology;

use crate::engine::Decoded;
use crate::groundtruth::Expected;

/// Per-symbology counters; `expected`/`read` are keyed by the expected symbology,
/// `false_positives` by the reported one.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SymStats {
    pub expected: usize,
    pub read: usize,
    pub false_positives: usize,
}

#[derive(Debug, Default)]
pub struct Stats {
    pub images: usize,
    pub errors: usize,
    pub unsupported: usize,
    pub per_symbology: BTreeMap<&'static str, SymStats>,
    pub latencies: Vec<Duration>,
}

impl Stats {
    pub fn totals(&self) -> SymStats {
        self.per_symbology
            .values()
            .fold(SymStats::default(), |a, s| SymStats {
                expected: a.expected + s.expected,
                read: a.read + s.read,
                false_positives: a.false_positives + s.false_positives,
            })
    }

    pub fn merge(&mut self, other: &Stats) {
        self.images += other.images;
        self.errors += other.errors;
        self.unsupported += other.unsupported;
        self.latencies.extend_from_slice(&other.latencies);
        for (k, v) in &other.per_symbology {
            let e = self.per_symbology.entry(k).or_default();
            e.expected += v.expected;
            e.read += v.read;
            e.false_positives += v.false_positives;
        }
    }
}

/// Expands an 8-digit UPC-E (number system, six data digits, check digit) to 12-digit UPC-A.
fn upce_to_upca(e: &[u8]) -> Option<Vec<u8>> {
    if e.len() != 8 || !matches!(e[0], b'0' | b'1') {
        return None;
    }
    let d = &e[1..7];
    let mut v = vec![e[0]];
    match d[5] {
        b'0'..=b'2' => {
            v.extend_from_slice(&[d[0], d[1], d[5], b'0', b'0', b'0', b'0', d[2], d[3], d[4]]);
        }
        b'3' => v.extend_from_slice(&[d[0], d[1], d[2], b'0', b'0', b'0', b'0', b'0', d[3], d[4]]),
        b'4' => v.extend_from_slice(&[d[0], d[1], d[2], d[3], b'0', b'0', b'0', b'0', b'0', d[4]]),
        _ => v.extend_from_slice(&[d[0], d[1], d[2], d[3], d[4], b'0', b'0', b'0', b'0', d[5]]),
    }
    v.push(e[7]);
    Some(v)
}

type Key = (Symbology, Vec<u8>);

/// Canonical comparison key: whitespace trimmed, UPC-A and UPC-E (expanded to UPC-A) folded into
/// EAN-13 with a leading 0,
/// the dataset's `{GS}` notation turned into ASCII 29 and a leading GS1 FNC1 separator dropped.
/// Valid UTF-8 is compared as trimmed text, anything else as raw bytes.
pub fn canonical(sym: Symbology, data: &[u8]) -> Key {
    let trimmed: Vec<u8> = match std::str::from_utf8(data) {
        Ok(s) => {
            let s = s.trim().replace("{GS}", "\u{1d}");
            s.strip_prefix('\u{1d}').unwrap_or(&s).as_bytes().to_vec()
        }
        Err(_) => data.trim_ascii().to_vec(),
    };
    if matches!(sym, Symbology::UpcA | Symbology::UpcE) && trimmed.iter().all(u8::is_ascii_digit) {
        let upca = match (sym, trimmed.len()) {
            (Symbology::UpcE, 8) => upce_to_upca(&trimmed),
            (Symbology::UpcA, 12) => Some(trimmed.clone()),
            _ => None,
        };
        if let Some(mut v) = upca {
            v.insert(0, b'0');
            return (Symbology::Ean13, v);
        }
        // Some datasets spell UPC-A and expanded UPC-E in EAN-13 notation.
        if trimmed.len() == 13 && trimmed[0] == b'0' {
            return (Symbology::Ean13, trimmed);
        }
    }
    (sym, trimmed)
}

pub fn score(found: &[Decoded], expected: &[Expected], stats: &mut Stats) {
    let found_keys: Vec<Key> = found
        .iter()
        .map(|d| canonical(d.symbology, &d.data))
        .collect();
    let alt_keys: Vec<Vec<Key>> = found
        .iter()
        .map(|d| d.alts.iter().map(|t| canonical(d.symbology, t)).collect())
        .collect();
    let exp_keys: Vec<Option<Key>> = expected
        .iter()
        .map(|e| e.symbology.map(|s| canonical(s, &e.data)))
        .collect();
    let is_match = |i: usize, key: &Key| found_keys[i] == *key || alt_keys[i].contains(key);
    let mut used = vec![false; found.len()];

    for (exp, key) in expected.iter().zip(&exp_keys) {
        let (Some(sym), Some(key)) = (exp.symbology, key) else {
            stats.unsupported += 1;
            continue;
        };
        let entry = stats.per_symbology.entry(sym.name()).or_default();
        entry.expected += 1;
        if let Some(i) = (0..found.len()).find(|&i| !used[i] && is_match(i, key)) {
            used[i] = true;
            entry.read += 1;
        } else if std::env::var_os("BENCH_SHOW_FP").is_some() {
            eprintln!(
                "missed: {} {:?}",
                sym.name(),
                String::from_utf8_lossy(&key.1)
            );
        }
    }

    for (i, key) in found_keys.iter().enumerate() {
        if used[i] || exp_keys.iter().flatten().any(|e| is_match(i, e)) {
            continue;
        }
        if std::env::var_os("BENCH_SHOW_FP").is_some() {
            eprintln!(
                "false positive: {} {:?}",
                found[i].symbology.name(),
                String::from_utf8_lossy(&key.1)
            );
        }
        stats
            .per_symbology
            .entry(found[i].symbology.name())
            .or_default()
            .false_positives += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exp(sym: Symbology, text: &str) -> Expected {
        Expected {
            symbology: Some(sym),
            label: String::new(),
            data: text.as_bytes().to_vec(),
        }
    }

    #[test]
    fn upca_equals_ean13_with_leading_zero() {
        let mut st = Stats::default();
        let found = vec![Decoded::new(Symbology::Ean13, b"0123456789012".to_vec())];
        score(&found, &[exp(Symbology::UpcA, "123456789012")], &mut st);
        assert_eq!(
            st.totals(),
            SymStats {
                expected: 1,
                read: 1,
                false_positives: 0
            }
        );

        let mut st = Stats::default();
        let found = vec![Decoded::new(Symbology::UpcA, b"123456789012".to_vec())];
        score(&found, &[exp(Symbology::Ean13, "0123456789012")], &mut st);
        assert_eq!(st.totals().read, 1);
        assert_eq!(st.totals().false_positives, 0);
    }

    #[test]
    fn upce_matches_its_expansion_in_ean13_notation() {
        let (s, d) = canonical(Symbology::UpcE, b"05096893");
        assert_eq!((s, d.as_slice()), (Symbology::Ean13, &b"0050968000093"[..]));
        for (e, a) in [
            ("04963406", "049000006346"),
            ("04124498", "041244000098"),
            ("01234531", "012300000451"),
            ("01264904", "012000006494"),
        ] {
            let up = canonical(Symbology::UpcE, e.as_bytes());
            assert_eq!(up, canonical(Symbology::UpcA, a.as_bytes()), "{e}");
            assert_eq!(up, canonical(Symbology::UpcE, format!("0{a}").as_bytes()));
        }
        assert_eq!(
            canonical(Symbology::UpcA, b"0456314319671"),
            canonical(Symbology::UpcA, b"456314319671")
        );
        // Not expandable: kept as is.
        assert_eq!(canonical(Symbology::UpcE, b"25096893").0, Symbology::UpcE);
    }

    #[test]
    fn whitespace_is_trimmed_and_bytes_fall_back() {
        let mut st = Stats::default();
        let found = vec![Decoded::new(Symbology::Code39, b" AB12\n".to_vec())];
        score(&found, &[exp(Symbology::Code39, "AB12")], &mut st);
        assert_eq!(st.totals().read, 1);

        let mut st = Stats::default();
        let found = vec![Decoded::new(Symbology::Qr, vec![0xff, 0xfe, b' '])];
        let e = Expected {
            symbology: Some(Symbology::Qr),
            label: String::new(),
            data: vec![0xff, 0xfe],
        };
        score(&found, &[e], &mut st);
        assert_eq!(st.totals().read, 1);
    }

    #[test]
    fn false_positives_and_duplicates() {
        let mut st = Stats::default();
        let found = vec![
            Decoded::new(Symbology::Qr, b"a".to_vec()),
            Decoded::new(Symbology::Qr, b"a".to_vec()),
            Decoded::new(Symbology::Qr, b"zzz".to_vec()),
            Decoded::new(Symbology::Code128, b"a".to_vec()),
        ];
        score(
            &found,
            &[exp(Symbology::Qr, "a"), exp(Symbology::Qr, "b")],
            &mut st,
        );
        let t = st.totals();
        assert_eq!((t.expected, t.read, t.false_positives), (2, 1, 2));
        assert_eq!(st.per_symbology[Symbology::Qr.name()].false_positives, 1);
    }

    #[test]
    fn charset_decoded_text_matches_too() {
        let sjis = vec![0x83, 0x65];
        let found = vec![Decoded {
            symbology: Symbology::Qr,
            data: sjis.clone(),
            alts: vec!["テ".as_bytes().to_vec()],
        }];
        for label in [exp(Symbology::Qr, "テ"), {
            let mut e = exp(Symbology::Qr, "");
            e.data = sjis.clone();
            e
        }] {
            let mut st = Stats::default();
            score(&found, &[label], &mut st);
            assert_eq!((st.totals().read, st.totals().false_positives), (1, 0));
        }
        let mut st = Stats::default();
        score(&found, &[exp(Symbology::Qr, "x")], &mut st);
        assert_eq!((st.totals().read, st.totals().false_positives), (0, 1));
    }

    #[test]
    fn unsupported_expected_is_excluded() {
        let mut st = Stats::default();
        let e = Expected {
            symbology: None,
            label: "DATAMATRIX".into(),
            data: b"x".to_vec(),
        };
        score(&[], &[e], &mut st);
        assert_eq!(st.unsupported, 1);
        assert_eq!(st.totals().expected, 0);
    }

    #[test]
    fn gs_notation_matches_fnc1() {
        let gt = canonical(Symbology::Code128, b"{GS}7829{GS}2798");
        let read = canonical(Symbology::Code128, b"7829\x1d2798");
        assert_eq!(gt, read);
    }
}
