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

/// Canonical comparison key: whitespace trimmed, UPC-A folded into EAN-13 with a leading 0,
/// the dataset's `{GS}` notation turned into ASCII 29 and a leading GS1 FNC1 separator dropped.
/// Valid UTF-8 is compared as trimmed text, anything else as raw bytes.
pub fn canonical(sym: Symbology, data: &[u8]) -> (Symbology, Vec<u8>) {
    let trimmed: Vec<u8> = match std::str::from_utf8(data) {
        Ok(s) => {
            let s = s.trim().replace("{GS}", "\u{1d}");
            s.strip_prefix('\u{1d}').unwrap_or(&s).as_bytes().to_vec()
        }
        Err(_) => data.trim_ascii().to_vec(),
    };
    if sym == Symbology::UpcA && trimmed.len() == 12 && trimmed.iter().all(u8::is_ascii_digit) {
        let mut v = Vec::with_capacity(13);
        v.push(b'0');
        v.extend_from_slice(&trimmed);
        return (Symbology::Ean13, v);
    }
    (sym, trimmed)
}

pub fn score(found: &[Decoded], expected: &[Expected], stats: &mut Stats) {
    let found_keys: Vec<(Symbology, Vec<u8>)> =
        found.iter().map(|(s, d)| canonical(*s, d)).collect();
    let exp_keys: Vec<Option<(Symbology, Vec<u8>)>> = expected
        .iter()
        .map(|e| e.symbology.map(|s| canonical(s, &e.data)))
        .collect();
    let mut used = vec![false; found.len()];

    for (exp, key) in expected.iter().zip(&exp_keys) {
        let (Some(sym), Some(key)) = (exp.symbology, key) else {
            stats.unsupported += 1;
            continue;
        };
        let entry = stats.per_symbology.entry(sym.name()).or_default();
        entry.expected += 1;
        if let Some(i) = (0..found.len()).find(|&i| !used[i] && found_keys[i] == *key) {
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
        if used[i] || exp_keys.iter().flatten().any(|e| e == key) {
            continue;
        }
        if std::env::var_os("BENCH_SHOW_FP").is_some() {
            eprintln!(
                "false positive: {} {:?}",
                found[i].0.name(),
                String::from_utf8_lossy(&key.1)
            );
        }
        stats
            .per_symbology
            .entry(found[i].0.name())
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
        let found = vec![(Symbology::Ean13, b"0123456789012".to_vec())];
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
        let found = vec![(Symbology::UpcA, b"123456789012".to_vec())];
        score(&found, &[exp(Symbology::Ean13, "0123456789012")], &mut st);
        assert_eq!(st.totals().read, 1);
        assert_eq!(st.totals().false_positives, 0);
    }

    #[test]
    fn whitespace_is_trimmed_and_bytes_fall_back() {
        let mut st = Stats::default();
        let found = vec![(Symbology::Code39, b" AB12\n".to_vec())];
        score(&found, &[exp(Symbology::Code39, "AB12")], &mut st);
        assert_eq!(st.totals().read, 1);

        let mut st = Stats::default();
        let found = vec![(Symbology::Qr, vec![0xff, 0xfe, b' '])];
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
            (Symbology::Qr, b"a".to_vec()),
            (Symbology::Qr, b"a".to_vec()),
            (Symbology::Qr, b"zzz".to_vec()),
            (Symbology::Code128, b"a".to_vec()),
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
