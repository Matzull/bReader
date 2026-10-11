//! GS1 DataBar Expanded in one row.
//!
//! A symbol of `n` characters (4 to 22, the check character first) has
//! 17 modules per character and 15 per finder pattern; a finder follows
//! every character of even index. Characters of odd index are printed
//! reversed. The first character holds the check value
//! `211 * (n - 4) + checksum`, the checksum being the sum of the element
//! widths of the other characters times powers of 3 (mod 211); which powers
//! depends on the symbol size. The finder patterns used are fixed by `n`.

use breader_core::Symbology;

use super::quantize::{MAX_ELEMENTS, Segment, quantize};
use super::rank::rank;
use crate::common::{MIN_MODULE, quiet_after_open, quiet_before_open};
use crate::decoder::LineHit;
use crate::runs::Runs;

use super::message::{Bits, element_string};

const MIN_CHARS: usize = 4;
const MAX_CHARS: usize = 22;
const TOLERANCE: f32 = 0.5;
const PIECE_SLACK: f32 = 0.25;
const QUIET: f32 = 0.8;

/// The six finder patterns; characters of even position use them as
/// printed, the others reversed.
const FINDERS: [[u8; 5]; 6] = [
    [1, 8, 4, 1, 1],
    [3, 6, 4, 1, 1],
    [3, 4, 6, 1, 1],
    [3, 2, 8, 1, 1],
    [2, 6, 5, 1, 1],
    [2, 2, 9, 1, 1],
];

/// Finder patterns by symbol size, `n = 4 + 2 * group` and the next odd
/// number share a row.
const FINDER_SEQUENCES: [&[u8]; 10] = [
    &[0, 0],
    &[0, 1, 1],
    &[0, 2, 1, 3],
    &[0, 4, 1, 3, 2],
    &[0, 4, 1, 3, 3, 5],
    &[0, 4, 1, 3, 4, 5, 5],
    &[0, 0, 1, 1, 2, 2, 3, 3],
    &[0, 0, 1, 1, 2, 2, 3, 4, 4],
    &[0, 0, 1, 1, 2, 2, 3, 4, 5, 5],
    &[0, 0, 1, 1, 2, 3, 3, 4, 4, 5, 5],
];

/// Weight rows of the character pairs `(1, 2)`, `(3, 4)`... for each size
/// group; character 1 always uses row 0. Row `r` gives the element weights
/// `3^(8r + i)` mod 211; the first character of a pair is weighed as
/// printed, the second reversed.
const WEIGHT_ROWS: [&[u8]; 10] = [
    &[1],
    &[5, 3],
    &[9, 3, 13],
    &[17, 3, 13, 7],
    &[17, 3, 13, 11, 21],
    &[17, 3, 13, 15, 21, 19],
    &[1, 3, 5, 7, 9, 11, 13],
    &[1, 3, 5, 7, 9, 11, 17, 15],
    &[1, 3, 5, 7, 9, 11, 17, 19, 21],
    &[1, 3, 5, 7, 13, 11, 17, 15, 21, 19],
];

/// Per group: odd and even element sums, widest odd element, number of odd
/// combinations in use (a prefix of the combinations with a width-1
/// element), number of even combinations, first value.
const ODD_SUM: [u8; 5] = [12, 10, 8, 6, 4];
const ODD_MAX: [u8; 5] = [7, 5, 4, 3, 1];
const ODD_COUNT: [u16; 5] = [87, 52, 30, 10, 1];
const EVEN_COUNT: [u16; 5] = [4, 20, 52, 104, 204];
const BASE: [u16; 5] = [0, 348, 1388, 2948, 3988];

fn group_size(n: usize) -> usize {
    (n - 3) / 2
}

fn finder_count(n: usize) -> usize {
    n.div_ceil(2)
}

/// Value of one character, elements in the numbering of the specification.
pub(super) fn char_value(c: &[u8; 8]) -> Option<u16> {
    let odd = [c[0], c[2], c[4], c[6]];
    let even = [c[1], c[3], c[5], c[7]];
    let sum: u8 = odd.iter().sum();
    let g = ODD_SUM.iter().position(|&s| s == sum)?;
    let o = rank(&odd, ODD_MAX[g], true)?;
    if o >= u32::from(ODD_COUNT[g]) {
        return None;
    }
    let e = rank(&even, 9 - ODD_MAX[g], false)?;
    u16::try_from(o)
        .ok()?
        .checked_mul(EVEN_COUNT[g])?
        .checked_add(u16::try_from(e).ok()?)?
        .checked_add(BASE[g])
}

/// Elements of character `k` in specification order.
fn spec_order(c: &[u8; 8], k: usize) -> [u8; 8] {
    if k % 2 == 1 {
        let mut r = *c;
        r.reverse();
        r
    } else {
        *c
    }
}

fn checksum(chars: &[[u8; 8]], n: usize) -> u32 {
    let pairs = WEIGHT_ROWS[group_size(n)];
    let mut sum = 0u32;
    for (k, c) in chars.iter().enumerate().skip(1) {
        let row = if k == 1 {
            0
        } else {
            u32::from(pairs[(k - 2) / 2]) + ((k - 2) % 2) as u32
        };
        let mut w = 1u32;
        for _ in 0..8 * row {
            w = w * 3 % 211;
        }
        for &e in &spec_order(c, k) {
            sum += w * u32::from(e);
            w = w * 3 % 211;
        }
    }
    sum % 211
}

/// Runs of the right guard: space and bar when the number of finders is
/// even, only the bar when it is odd (the space then merges with the
/// quiet zone).
fn tail_runs(n: usize) -> usize {
    if finder_count(n) % 2 == 0 { 2 } else { 1 }
}

/// Runs from the first bar guard to the last bar: the guards, the
/// characters and the finders.
fn row_runs(n: usize) -> usize {
    8 * n + 5 * finder_count(n) + tail_runs(n) + 1
}

/// Modules from the start of the first bar to the end of the last.
fn row_modules(n: usize) -> usize {
    17 * n + 15 * finder_count(n) + tail_runs(n) + 1
}

/// Scans `runs` for Expanded symbols; true when a symbol start was seen.
/// A bar guard cut by a border of the profile has no edge: run -1 and run
/// `runs.len()` stand for it.
pub(super) fn decode(runs: &Runs, out: &mut Vec<LineHit>) -> bool {
    let n_runs = runs.len() as isize;
    let w = runs.widths();
    let mut started = false;
    let mut j: isize = if n_runs > 0 && !runs.is_bar(0) { -1 } else { 0 };
    while j + row_runs(MIN_CHARS) as isize <= n_runs + 1 {
        // First character and first finder: 17 + 15 modules.
        let first = (j + 1) as usize;
        let head: f32 = w[first..first + 13].iter().sum();
        let module = head / 32.0;
        let guard_ok = j < 0 || (0.5 * module..=2.2 * module).contains(&w[j as usize]);
        let quiet_ok = j <= 0 || quiet_before_open(runs, j as usize, 0.0) >= QUIET * module;
        if module >= MIN_MODULE
            && guard_ok
            && (j >= 0 || runs.lead() >= 1.0)
            && quiet_ok
            && head_matches(&w[first..], module)
        {
            for n in MIN_CHARS..=MAX_CHARS {
                if j + row_runs(n) as isize > n_runs + 1 {
                    break;
                }
                started |= try_symbol(runs, j, n, module, out);
            }
        }
        j += 2;
    }
    started
}

/// The first character and finder have their module counts.
fn head_matches(w: &[f32], module: f32) -> bool {
    let sum = |a: usize, b: usize| -> f32 { w[a..b].iter().sum() };
    let within =
        |s: f32, modules: f32| (s - modules * module).abs() <= PIECE_SLACK * modules * module;
    within(sum(0, 8), 17.0) && within(sum(8, 13), 15.0)
}

fn try_symbol(runs: &Runs, j: isize, n: usize, head_module: f32, out: &mut Vec<LineHit>) -> bool {
    let len = row_runs(n);
    let n_runs = runs.len() as isize;
    let last = j + len as isize - 1;
    let (first_cut, last_cut) = (j < 0, last >= n_runs);
    if last_cut && (runs.trail() < 1.0 || runs.is_bar(runs.len() - 1)) {
        return false;
    }
    let real = |k: isize| runs.widths()[k as usize];
    // The cut guards take the width of the other one, or the module.
    let guard = match (first_cut, last_cut) {
        (false, _) => real(j),
        (true, false) => real(last),
        (true, true) => head_module,
    };
    let mut widths = [0f32; MAX_ELEMENTS];
    for (i, v) in widths[..len].iter_mut().enumerate() {
        let k = j + i as isize;
        *v = if k < 0 || k >= n_runs { guard } else { real(k) };
    }
    let widths = &widths[..len];
    // Positions decrease along reversed runs.
    let sign = if runs.end(0) >= runs.start(0) { 1.0 } else { -1.0 };
    let start = if first_cut {
        runs.start(0) - sign * guard
    } else {
        runs.start(j as usize)
    };
    let end = if last_cut {
        runs.end(runs.len() - 1) + sign * guard
    } else {
        runs.end(last as usize)
    };
    let module = (end - start).abs() / row_modules(n) as f32;
    let quiet_after = if last_cut || last + 1 == n_runs {
        f32::INFINITY
    } else {
        quiet_after_open(runs, last as usize, 0.0)
    };
    if !(module >= MIN_MODULE) || quiet_after < QUIET * module {
        return false;
    }
    // Guards at the end: the last bar and, for an even number of finders,
    // the space before.
    let tail = tail_runs(n);
    let guards_ok = widths[len - tail..]
        .iter()
        .chain(&widths[..1])
        .all(|&g| (0.5 * module..=2.2 * module).contains(&g));
    if !guards_ok {
        return false;
    }
    // Segments: guard, characters and finders, end guards.
    let mut segments = [Segment::new(0, 0, 0); 1 + MAX_CHARS + MAX_CHARS / 2 + 1 + 2];
    let mut count = 0;
    let mut push = |s: Segment| {
        segments[count] = s;
        count += 1;
    };
    push(Segment::new(0, 1, 1));
    let mut at = 1;
    for k in 0..n {
        push(Segment::new(at, 8, 17));
        at += 8;
        if k % 2 == 0 {
            push(Segment::new(at, 5, 15));
            at += 5;
        }
    }
    for g in at..len {
        push(Segment::new(g, 1, 1));
    }
    for s in &segments[..count] {
        if s.len > 1 {
            let sum: f32 = widths[s.at..s.at + s.len].iter().sum();
            let expected = f32::from(s.modules) * module;
            if (sum - expected).abs() > PIECE_SLACK * expected {
                return true;
            }
        }
    }
    let mut mods = [0u8; MAX_ELEMENTS];
    let mods = &mut mods[..len];
    let Some(q) = quantize(widths, true, &segments[..count], module, TOLERANCE, mods) else {
        return true;
    };
    // Finder patterns.
    let sequence = FINDER_SEQUENCES[group_size(n)];
    let mut chars = [[0u8; 8]; MAX_CHARS];
    let mut at = 1;
    for k in 0..n {
        chars[k].copy_from_slice(&mods[at..at + 8]);
        at += 8;
        if k % 2 == 0 {
            let p = k / 2;
            let mut f = [0u8; 5];
            f.copy_from_slice(&mods[at..at + 5]);
            if p % 2 == 1 {
                f.reverse();
            }
            if FINDERS[usize::from(sequence[p])] != f {
                return true;
            }
            at += 5;
        }
    }
    let chars = &chars[..n];
    let Some(check) = char_value(&spec_order(&chars[0], 0)) else {
        return true;
    };
    if usize::from(check) != 211 * (n - MIN_CHARS) + checksum(chars, n) as usize {
        return true;
    }
    let mut bits = [0u8; 12 * (MAX_CHARS - 1)];
    for (k, c) in chars.iter().enumerate().skip(1) {
        let Some(v) = char_value(&spec_order(c, k)) else {
            return true;
        };
        for b in 0..12 {
            bits[12 * (k - 1) + b] = ((v >> (11 - b)) & 1) as u8;
        }
    }
    let Some(text) = element_string(&Bits(&bits[..12 * (n - 1)]), n) else {
        return true;
    };
    out.push(LineHit {
        symbology: Symbology::DataBarExpanded,
        text,
        addon: None,
        start,
        end,
        module,
        quality: (1.0 - q.deviation / 0.5).clamp(0.0, 1.0),
        aim: *b"]e0",
        checksum_verified: true,
        short_quiet: false,
        inverted: false,
        start_stop: None,
        segments: n as u8,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databar::DataBarDecoder;
    use crate::decoder::LinearDecoder;
    use crate::testutil::*;

    fn opts() -> crate::decoder::LineOptions {
        line_options(|_| {})
    }

    fn read(text: &str, render: Render) -> Vec<(Vec<u8>, u8)> {
        let m = databar_expanded_modules(text);
        decode_all(&DataBarDecoder, &render.profile(&m), &opts())
            .into_iter()
            .filter(|h| h.symbology == Symbology::DataBarExpanded)
            .map(|h| (h.text, h.segments))
            .collect()
    }

    fn readable(text: &str) -> Vec<u8> {
        // The reader keeps the separator as GS, like the input.
        text.as_bytes().to_vec()
    }

    #[test]
    fn tables_are_consistent() {
        for (g, seq) in FINDER_SEQUENCES.iter().enumerate() {
            let n = if g == 0 { 4 } else { 2 * g + 3 };
            assert_eq!(seq.len(), finder_count(n));
            assert_eq!(group_size(n), g);
            if g > 0 {
                assert_eq!(seq.len(), finder_count(n + 1));
            }
            assert!(seq.iter().all(|&f| usize::from(f) < FINDERS.len()));
        }
        for f in FINDERS {
            assert_eq!(f.iter().map(|&w| u32::from(w)).sum::<u32>(), 15);
        }
        for n in MIN_CHARS..=MAX_CHARS {
            let pairs = WEIGHT_ROWS[group_size(n)];
            assert_eq!(pairs.len(), (n - 2).div_ceil(2), "{n}");
            assert!(pairs.iter().all(|&r| r % 2 == 1 && r + 1 <= 22));
            assert_eq!(row_runs(n) - 1 - tail_runs(n), 8 * n + 5 * finder_count(n));
        }
        // Value ranges of the five groups.
        let mut next = 0u32;
        for g in 0..5 {
            assert_eq!(u32::from(BASE[g]), next);
            next += u32::from(ODD_COUNT[g]) * u32::from(EVEN_COUNT[g]);
        }
        assert!(next >= 4096);
    }

    #[test]
    fn reads_every_symbol_size_and_method() {
        let strings = [
            "1012A",
            "10123",
            "101",
            "01900123456789083103001750",
            "01900123456789083202000156",
            "01900123456789083203032767",
            "0190012345678908310200175011100312",
            "0190012345678908320200175017100312",
            "01900123456789083922795",
            "019001234567890839320401234",
            "01900123456789083932040EUR",
            "0100000000000000",
            "010001234567890510ABC123",
            "011234567890123110UNIVERSITY-OF-DEUSTO",
            "10567811010101",
            "101098/1234",
            "101098a1234",
            "10123456A1234A\x1d15991231",
            "2112345678901234567\x1d240ABCDEFG",
            "01988987654321061599123131030017501012A\x1d422123\x1d2123456\x1d423012345678901",
            "1012345678901234567890123456789012345678901234567890",
        ];
        let mut sizes = std::collections::BTreeSet::new();
        for t in strings {
            let t = t.replace("\\x1d", "\x1d");
            let got = read(&t, Render::px(2.6));
            assert!(
                got.iter().any(|(s, _)| *s == readable(&t)),
                "{t:?}: {got:?}"
            );
            sizes.extend(got.iter().map(|(_, n)| *n));
        }
        assert!(sizes.contains(&4) && sizes.contains(&5) && sizes.len() > 8, "{sizes:?}");
    }

    #[test]
    fn random_payloads_round_trip_in_every_size() {
        let alnum = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-./";
        let mut rng = Rng(0xe9a2_d3d);
        let mut sizes = std::collections::BTreeSet::new();
        for _ in 0..300 {
            let k = 1 + rng.below(30) as usize;
            let value: String = (0..k)
                .map(|_| char::from(alnum[rng.below(alnum.len() as u64) as usize]))
                .collect();
            let t = format!("10{value}");
            let render = Render {
                gain: rng.unit() * 0.2,
                sigma: 0.5 + rng.unit() * 0.3,
                ..Render::px(1.8 + rng.unit() * 2.5)
            };
            let got = read(&t, render);
            assert!(got.iter().any(|(s, _)| *s == readable(&t)), "{t:?}: {got:?}");
            sizes.extend(got.iter().map(|(_, n)| *n));
        }
        assert!(sizes.len() >= 12, "{sizes:?}");
    }

    #[test]
    fn a_damaged_symbol_is_never_misread() {
        let mut rng = Rng(0xf11b);
        for t in [
            "1012A",
            "01900123456789083103001750",
            "10123456789012345678",
            "01988987654321061599123131030017501012A",
        ] {
            let good = readable(t);
            let modules = databar_expanded_modules(t);
            for at in 0..modules.len() {
                for _ in 0..1 {
                    let mut m = modules.clone();
                    m[at] = !m[at];
                    let render = Render::px(3.0 + rng.unit());
                    for h in decode_all(&DataBarDecoder, &render.profile(&m), &opts()) {
                        assert_eq!(h.text, good, "{t} flipped {at}");
                    }
                }
            }
        }
    }

    #[test]
    fn symbol_cut_by_the_borders_is_read() {
        let t = "1012A";
        let m = databar_expanded_modules(t);
        let render = Render {
            quiet_left: 0.0,
            quiet_right: 0.0,
            ..Render::px(3.0)
        };
        let got = decode_all(&DataBarDecoder, &render.profile(&m[1..]), &opts());
        assert!(got.iter().any(|h| h.text == t.as_bytes()), "{got:?}");
    }

    #[test]
    fn noise_reads_nothing() {
        let mut rng = Rng(0xfeed_5eed);
        for _ in 0..3000 {
            let n = 46 + rng.below(120) as usize;
            let widths: Vec<f32> = (0..n)
                .map(|_| (1 + rng.below(5)) as f32 * (0.8 + 0.4 * rng.unit()))
                .collect();
            let runs = crate::runs::Runs::from_widths(true, &widths, 20.0, 20.0);
            let mut out = Vec::new();
            DataBarDecoder.decode(&runs, &opts(), &mut out);
            assert!(out.is_empty(), "{widths:?}");
        }
    }
}
