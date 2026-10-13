//! Codabar (NW-7), following ANSI/AIM BC3 and EN 798.
//!
//! Like Interleaved 2 of 5, the symbology has no check character in general
//! and a small character set, so noise and other symbologies produce
//! plausible readings. The guards are structural: a start and a stop
//! character from A-D, none inside, a minimum length, clear wide/narrow
//! classification of every element, bounded gaps and large quiet zones. The
//! aggregator additionally requires more agreeing scanlines than usual.

use breader_core::Symbology;

use crate::common::{
    MIN_MODULE, Split, adjusted_width, adjusted_widths, quiet_after, quiet_after_open,
    quiet_before, quiet_before_open, split_wide,
};
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::runs::Runs;

/// Characters in the order of [`PATTERNS`].
const CHARSET: &[u8; 20] = b"0123456789-$:/.+ABCD";
/// Bar and space widths of each character, `w` for wide. Every character
/// starts with a bar and has four bars and three spaces.
const PATTERNS: [&str; 20] = [
    "nnnnnww", "nnnnwwn", "nnnwnnw", "wwnnnnn", "nnwnnwn", "wnnnnwn", "nwnnnnw", "nwnnwnn",
    "nwwnnnn", "wnnwnnn", "nnnwwnn", "nnwwnnn", "wnnnwnw", "wnwnnnw", "wnwnwnn", "nnwnwnw",
    "nnwwnwn", "nnnwnww", "nwnwnnw", "nnnwwwn",
];

/// Character for each 7-bit mask of wide elements, 0 where none exists.
static BY_MASK: [u8; 128] = {
    let mut t = [0u8; 128];
    let mut k = 0;
    while k < 20 {
        let p = PATTERNS[k].as_bytes();
        let mut mask = 0usize;
        let mut i = 0;
        while i < 7 {
            if p[i] == b'w' {
                mask |= 1 << i;
            }
            i += 1;
        }
        t[mask] = CHARSET[k];
        k += 1;
    }
    t
};

/// Shortest payload, start and stop characters excluded.
pub(crate) const MIN_DATA_LENGTH: usize = 4;
const MAX_CHARS: usize = 128;
/// Quiet zone on each side, in narrow element widths. A margin that ends at
/// the profile border counts as open but flags the hit `short_quiet`.
const QUIET: f32 = 10.0;
/// Cheap first screen on the start character: the quiet zone as a fraction
/// of its width (10 narrow elements are at least 0.5 of a character of up
/// to 20).
const QUIET_SCREEN: f32 = 0.45;
/// Inter-character gap, in narrow widths.
const GAP: (f32, f32) = (0.5, 3.0);
/// Accepted ratio between the mean wide and the mean narrow element.
const RATIO: (f32, f32) = (1.7, 3.6);
/// The smallest wide element must exceed the largest narrow one by this factor.
const MIN_SEPARATION: f32 = 1.4;
/// Allowed ratio between the narrow widths of neighboring characters.
const MODULE_DRIFT: f32 = 1.5;
/// Allowed ratio between the largest and smallest narrow width in a symbol.
const SYMBOL_DRIFT: f32 = 2.0;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CodabarDecoder;

struct CharRead {
    ch: u8,
    narrow: f32,
    separation: f32,
}

fn is_guard(ch: u8) -> bool {
    matches!(ch, b'A'..=b'D')
}

/// Splits seven widths into wide and narrow elements: two wide ones for
/// digits, - and $; three for the rest.
fn classify(w: &[f32; 7]) -> Option<Split> {
    split_wide(w, 2)
        .filter(|s| s.separation >= MIN_SEPARATION)
        .or_else(|| split_wide(w, 3).filter(|s| s.separation >= MIN_SEPARATION))
}

fn read_char(runs: &Runs, at: usize, bias: f32, prev_narrow: f32) -> Option<CharRead> {
    let w = adjusted_widths::<7>(runs, at, bias)?;
    let s = classify(&w)?;
    let ratio = s.wide / s.narrow;
    if s.narrow < MIN_MODULE
        || !(RATIO.0..=RATIO.1).contains(&ratio)
        || (prev_narrow > 0.0
            && (s.narrow < prev_narrow / MODULE_DRIFT || s.narrow > prev_narrow * MODULE_DRIFT))
    {
        return None;
    }
    let ch = BY_MASK[usize::from(s.wide_mask) & 0x7f];
    (ch != 0).then_some(CharRead {
        ch,
        narrow: s.narrow,
        separation: s.separation,
    })
}

/// Print gain of the symbol from a start character: bars minus spaces,
/// within the narrow and within the wide elements. The guards A-D all have
/// wide bars and wide spaces.
fn bias_of(raw: &[f32; 7], wide_mask: u16) -> Option<f32> {
    let mut sums = [[0.0f32; 2]; 2];
    let mut counts = [[0u8; 2]; 2];
    for (i, &w) in raw.iter().enumerate() {
        let (wide, space) = (usize::from(wide_mask >> i & 1), i % 2);
        sums[wide][space] += w;
        counts[wide][space] += 1;
    }
    if counts.iter().flatten().any(|&c| c == 0) {
        return None;
    }
    let mean = |wide: usize, space: usize| sums[wide][space] / f32::from(counts[wide][space]);
    Some(((mean(0, 0) - mean(0, 1)) + (mean(1, 0) - mean(1, 1))) / 4.0)
}

struct Read {
    chars: [u8; MAX_CHARS],
    len: usize,
    start: u8,
    stop: u8,
    separation_sum: f32,
    narrow_sum: f32,
    count: usize,
    last_bar: usize,
    short_quiet: bool,
}

/// Reads the start character at run `j`; returns it with the bar/space bias
/// and whether the quiet zone before it is short.
fn start_of(runs: &Runs, j: usize) -> Option<(CharRead, f32, bool)> {
    let raw = adjusted_widths::<7>(runs, j, 0.0)?;
    if runs.space_before_open(j) < QUIET_SCREEN * raw.iter().sum::<f32>() {
        return None;
    }
    let split = classify(&raw)?;
    let bias = bias_of(&raw, split.wide_mask)?;
    if bias.abs() > 0.45 * split.narrow {
        return None;
    }
    let start = read_char(runs, j, bias, 0.0)?;
    if !is_guard(start.ch) {
        return None;
    }
    let short = quiet_before(runs, j, bias) < QUIET * start.narrow;
    (quiet_before_open(runs, j, bias) >= QUIET * start.narrow).then_some((start, bias, short))
}

fn read_symbol(runs: &Runs, j: usize, started: &mut bool) -> Option<Read> {
    let (start, bias, short_start) = start_of(runs, j)?;
    *started = true;
    let mut read = Read {
        chars: [0; MAX_CHARS],
        len: 0,
        start: start.ch,
        stop: 0,
        separation_sum: start.separation,
        narrow_sum: start.narrow,
        count: 1,
        last_bar: 0,
        short_quiet: short_start,
    };
    let (mut narrow, mut lo, mut hi) = (start.narrow, start.narrow, start.narrow);
    let mut at = j + 8;
    loop {
        let gap = adjusted_width(runs, at - 1, bias)?;
        if gap > GAP.1 * narrow || gap < GAP.0 * narrow {
            return None;
        }
        let c = read_char(runs, at, bias, narrow)?;
        narrow = c.narrow;
        lo = lo.min(narrow);
        hi = hi.max(narrow);
        if hi > lo * SYMBOL_DRIFT {
            return None;
        }
        read.separation_sum += c.separation;
        read.narrow_sum += c.narrow;
        read.count += 1;
        if is_guard(c.ch) {
            if quiet_after_open(runs, at + 6, bias) < QUIET * narrow {
                return None;
            }
            read.short_quiet |= quiet_after(runs, at + 6, bias) < QUIET * narrow;
            read.stop = c.ch;
            read.last_bar = at + 6;
            return Some(read);
        }
        if read.len == MAX_CHARS {
            return None;
        }
        read.chars[read.len] = c.ch;
        read.len += 1;
        at += 8;
    }
}

impl LinearDecoder for CodabarDecoder {
    fn decode(&self, runs: &Runs, opts: &LineOptions, out: &mut Vec<LineHit>) -> bool {
        if !opts.symbologies.contains(Symbology::Codabar) {
            return false;
        }
        let mut started = false;
        let n = runs.len();
        let mut j = usize::from(n > 0 && !runs.is_bar(0));
        while j + 7 + 8 * (MIN_DATA_LENGTH + 1) <= n {
            if let Some(read) = read_symbol(runs, j, &mut started)
                && read.len >= MIN_DATA_LENGTH
            {
                let count = read.count as f32;
                out.push(LineHit {
                    symbology: Symbology::Codabar,
                    text: read.chars[..read.len].to_vec(),
                    addon: None,
                    start: runs.start(j),
                    end: runs.end(read.last_bar),
                    module: read.narrow_sum / count,
                    quality: ((read.separation_sum / count - 1.0) / 2.0).clamp(0.0, 1.0),
                    aim: *b"]F0",
                    checksum_verified: false,
                    short_quiet: read.short_quiet,
                    inverted: false,
                    start_stop: Some((read.start, read.stop)),
                    segments: 0,
                    thin: false,
                });
            }
            j += 2;
        }
        started
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn opts() -> LineOptions {
        line_options(|_| {})
    }

    fn prof(start: char, body: &str, stop: char, wide: usize, px: f32) -> Vec<u8> {
        Render::px(px).profile(&codabar_modules(start, body, stop, wide, 1))
    }

    fn texts(hits: &[LineHit]) -> Vec<Vec<u8>> {
        hits.iter().map(|h| h.text.clone()).collect()
    }

    fn run_lengths(modules: &[bool]) -> Vec<usize> {
        let mut out: Vec<usize> = Vec::new();
        for (i, &b) in modules.iter().enumerate() {
            if i > 0 && modules[i - 1] == b {
                *out.last_mut().unwrap() += 1;
            } else {
                out.push(1);
            }
        }
        out
    }

    #[test]
    fn generated_table_matches_the_specification_table() {
        for (k, (ch, pattern)) in CODABAR_TABLE.iter().enumerate() {
            let mask = pattern
                .bytes()
                .enumerate()
                .fold(0usize, |m, (i, b)| m | usize::from(b == b'w') << i);
            assert_eq!(BY_MASK[mask], *ch as u8, "{ch}");
            assert_eq!(pattern.len(), 7);
            assert_eq!(CHARSET[k], *ch as u8);
            assert_eq!(PATTERNS[k], *pattern);
        }
        assert_eq!(BY_MASK.iter().filter(|&&c| c != 0).count(), 20);
    }

    #[test]
    fn table_agrees_with_an_independent_encoder() {
        use barcoders::sym::codabar::Codabar;
        for (ch, _) in CODABAR_TABLE {
            // The encoder draws '+' with five wide elements, which no
            // specification table has; the other 19 characters agree.
            if ch == '+' {
                continue;
            }
            let body = if is_guard(ch as u8) {
                "1234".to_string()
            } else {
                format!("{ch}{ch}{ch}{ch}")
            };
            let text = format!("A{body}B");
            let bars: Vec<bool> = Codabar::new(&text)
                .unwrap()
                .encode()
                .iter()
                .map(|&b| b != 0)
                .collect();
            let shape = |v: &[bool]| -> Vec<bool> {
                let r = run_lengths(v);
                let narrow = *r.iter().min().unwrap();
                r.iter().map(|&n| n > narrow).collect()
            };
            let ours = codabar_modules('A', &body, 'B', 3, 1);
            assert_eq!(shape(&bars), shape(&ours), "{ch}");
        }
        // The guards themselves.
        for g in "ABCD".chars() {
            let text = format!("{g}1234{g}");
            let bars: Vec<bool> = Codabar::new(&text)
                .unwrap()
                .encode()
                .iter()
                .map(|&b| b != 0)
                .collect();
            let ours = codabar_modules(g, "1234", g, 3, 1);
            let shape = |v: &[bool]| -> Vec<bool> {
                let r = run_lengths(v);
                let narrow = *r.iter().min().unwrap();
                r.iter().map(|&n| n > narrow).collect()
            };
            assert_eq!(shape(&bars), shape(&ours), "{g}");
        }
    }

    #[test]
    fn reads_every_character_in_both_directions_and_polarities() {
        for body in ["0123456789", "-$:/.+", "12-34", "$1.5"] {
            for (start, stop) in [('A', 'B'), ('C', 'D'), ('D', 'A'), ('B', 'B')] {
                for wide in [2, 3] {
                    for px in [2.5, 3.0] {
                        let p = prof(start, body, stop, wide, px);
                        let hits = decode_all(&CodabarDecoder, &p, &opts());
                        assert_eq!(
                            texts(&hits),
                            [body.as_bytes().to_vec()],
                            "{body} {start}{stop} {wide} {px}"
                        );
                        assert_eq!(&hits[0].aim, b"]F0");
                        assert_eq!(hits[0].start_stop, Some((start as u8, stop as u8)));
                        assert!(!hits[0].checksum_verified);
                        let mut rev = p.clone();
                        rev.reverse();
                        let hits = decode_all(&CodabarDecoder, &rev, &opts());
                        assert_eq!(hits.len(), 1, "rev {body}");
                        assert_eq!(hits[0].text, body.as_bytes(), "rev {body}");
                        let r = Render {
                            dark: 230.0,
                            light: 20.0,
                            ..Render::px(px)
                        };
                        let m = codabar_modules(start, body, stop, wide, 1);
                        let hits = decode_all(&CodabarDecoder, &r.profile(&m), &opts());
                        assert_eq!(texts(&hits), [body.as_bytes().to_vec()]);
                        assert!(hits[0].inverted);
                    }
                }
            }
        }
    }

    #[test]
    fn reads_through_noise_and_gain() {
        let mut rng = Rng(5);
        let modules = codabar_modules('A', "40129", 'B', 3, 1);
        let mut ok = 0;
        for i in 0..40 {
            let r = Render {
                sigma: 0.5 + (i % 4) as f32 * 0.15,
                gain: (i % 5) as f32 * 0.1 - 0.2,
                offset: rng.unit(),
                ..Render::px(2.5)
            };
            let mut p = r.profile(&modules);
            add_noise(&mut p, 6.0, &mut rng);
            let hits = decode_all(&CodabarDecoder, &p, &opts());
            assert!(hits.iter().all(|h| h.text == b"40129"), "{hits:?}");
            ok += usize::from(!hits.is_empty());
        }
        assert!(ok >= 30, "{ok}");
    }

    #[test]
    fn minimum_length_counts_the_data_only() {
        let short = prof('A', "123", 'B', 3, 3.0);
        assert!(decode_all(&CodabarDecoder, &short, &opts()).is_empty());
        let ok = prof('A', "1234", 'B', 3, 3.0);
        assert_eq!(decode_all(&CodabarDecoder, &ok, &opts()).len(), 1);
        let empty = prof('A', "", 'B', 3, 3.0);
        assert!(decode_all(&CodabarDecoder, &empty, &opts()).is_empty());
    }

    #[test]
    fn guard_characters_inside_the_data_reject_the_symbol() {
        for body in ["12A34", "1B234", "123C4", "D1234", "1234A"] {
            let p = prof('A', body, 'B', 3, 3.0);
            let hits = decode_all(&CodabarDecoder, &p, &opts());
            assert!(hits.is_empty(), "{body}: {hits:?}");
        }
        // A data character where a guard belongs.
        let m = codabar_modules('1', "2345", 'B', 3, 1);
        assert!(decode_all(&CodabarDecoder, &Render::px(3.0).profile(&m), &opts()).is_empty());
        let m = codabar_modules('A', "2345", '6', 3, 1);
        assert!(decode_all(&CodabarDecoder, &Render::px(3.0).profile(&m), &opts()).is_empty());
    }

    #[test]
    fn truncated_symbols_read_nothing() {
        let modules = codabar_modules('A', "123456", 'B', 3, 1);
        let p = Render::px(3.0).profile(&modules);
        assert_eq!(decode_all(&CodabarDecoder, &p, &opts()).len(), 1);
        // Without the stop character, without the start character.
        let head = &modules[..modules.len() - 11];
        let tail = &modules[11..];
        for part in [head, tail] {
            let p = Render::px(3.0).profile(part);
            assert!(decode_all(&CodabarDecoder, &p, &opts()).is_empty());
        }
        let r = Render {
            quiet_left: 4.0,
            quiet_right: 4.0,
            ..Render::px(3.0)
        };
        let p = r.profile(&modules);
        assert_eq!(decode_all(&CodabarDecoder, &p, &opts()).len(), 1);
        for (depth, cut) in border_cuts(&p, 12.0, modules.len() as f32 * 3.0) {
            for hit in decode_all(&CodabarDecoder, &cut, &opts()) {
                assert_eq!(hit.text, b"123456", "depth {depth}");
            }
            if depth > 9 {
                assert!(
                    decode_all(&CodabarDecoder, &cut, &opts()).is_empty(),
                    "depth {depth}"
                );
            }
        }
    }

    #[test]
    fn quiet_zone_rules() {
        let modules = codabar_modules('A', "12345", 'B', 3, 1);
        let near = |left: usize, right: usize| {
            let p = Render::px(3.0).profile(&with_neighbours(&modules, left, right));
            decode_all(&CodabarDecoder, &p, &opts())
        };
        assert_eq!(near(12, 12).len(), 1);
        assert_eq!(near(11, 11).len(), 1);
        for (left, right) in [(8, 12), (12, 8), (5, 5), (3, 12)] {
            assert!(near(left, right).is_empty(), "{left} {right}");
        }
        // A margin that ends at the profile border is open but short.
        for (left, right) in [(3.0, 12.0), (12.0, 3.0), (1.0, 1.0)] {
            let r = Render {
                quiet_left: left,
                quiet_right: right,
                ..Render::px(3.0)
            };
            let hits = decode_all(&CodabarDecoder, &r.profile(&modules), &opts());
            assert_eq!(hits.len(), 1, "{left} {right}");
            assert!(hits[0].short_quiet, "{left} {right}");
        }
        let r = Render {
            quiet_left: 12.0,
            quiet_right: 12.0,
            ..Render::px(3.0)
        };
        let hits = decode_all(&CodabarDecoder, &r.profile(&modules), &opts());
        assert!(!hits[0].short_quiet);
    }

    #[test]
    fn gaps_and_ratios_must_be_plausible() {
        for gap in [1, 2, 3] {
            let m = codabar_modules('A', "1234", 'B', 3, gap);
            assert_eq!(
                decode_all(&CodabarDecoder, &Render::px(3.0).profile(&m), &opts()).len(),
                1,
                "gap {gap}"
            );
        }
        for gap in [4, 6, 9] {
            let m = codabar_modules('A', "1234", 'B', 3, gap);
            assert!(
                decode_all(&CodabarDecoder, &Render::px(3.0).profile(&m), &opts()).is_empty(),
                "gap {gap}"
            );
        }
        for wide in [1, 4, 5] {
            let p = prof('A', "1234", 'B', wide, 2.0);
            assert!(
                decode_all(&CodabarDecoder, &p, &opts()).is_empty(),
                "{wide}"
            );
        }
        let flat = Render::px(2.0).profile(&runs_to_modules(&[1; 90]));
        assert!(decode_all(&CodabarDecoder, &flat, &opts()).is_empty());
    }

    #[test]
    fn other_symbologies_are_not_read_as_codabar() {
        let mut rng = Rng(77);
        let mut cases: Vec<Vec<bool>> = Vec::new();
        for text in ["CODE39", "ABC-123", "1234567", "A1B2C3D4", "-$./+"] {
            cases.push(code39_modules(text, 3, 1));
            cases.push(code39_modules(text, 2, 1));
        }
        for _ in 0..40 {
            let pairs = 2 + rng.below(6) as usize;
            let digits = rng.digits(2 * pairs);
            cases.push(itf_modules(&digits, 2 + rng.below(2) as usize));
            cases.push(ean13_modules(&with_gs1_check(&rng.digits(12))));
            let mut v = vec![104u8];
            v.extend((0..9).map(|_| rng.below(103) as u8));
            cases.push(code128_modules_raw(&v));
        }
        for m in cases {
            for px in [1.5, 2.0, 3.0] {
                let p = Render::px(px).profile(&m);
                let hits = decode_all(&CodabarDecoder, &p, &opts());
                assert!(hits.is_empty(), "{hits:?}");
            }
        }
    }

    #[test]
    fn random_stripes_read_nothing() {
        let mut rng = Rng(9);
        for _ in 0..2000 {
            let n = 60 + rng.below(80) as usize;
            let widths: Vec<usize> = (0..n).map(|_| 1 + rng.below(3) as usize).collect();
            let p = Render::px(2.5).profile(&runs_to_modules(&widths));
            assert!(decode_all(&CodabarDecoder, &p, &opts()).is_empty());
        }
    }

    #[test]
    fn the_symbology_can_be_switched_off() {
        let p = prof('A', "1234", 'B', 3, 3.0);
        let off = line_options(|o| {
            o.symbologies = Symbology::ALL
                .iter()
                .copied()
                .filter(|s| *s != Symbology::Codabar)
                .collect();
        });
        assert!(decode_all(&CodabarDecoder, &p, &off).is_empty());
    }
}
