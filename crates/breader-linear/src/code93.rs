//! Code 93, following AIM USS Code 93 (ANSI/AIM BC5-1995).
//!
//! A character is nine modules wide: three bars and three spaces of one to
//! four modules each, starting with a bar. The symbol is the start character
//! `*`, the data, two check characters C and K (both modulo 47), the stop
//! character `*` and one termination bar. Four shift characters give the
//! whole ASCII set; they are decoded always, since they are distinct from the
//! characters `$ / + %` and cannot be mistaken for them.
//!
//! The guards are structural and arithmetic: the start and stop patterns with
//! the termination bar, every element of every character quantizing to a whole
//! number of modules that add up to nine, quiet zones measured (never taken as
//! open at the profile border) on both sides, a bounded module drift, both
//! check characters, and valid shift sequences. The aggregator additionally
//! asks for agreeing scanlines as for Code 39.

use breader_core::Symbology;

use crate::common::{
    MIN_MODULE, adjusted_width, adjusted_widths, full_ascii_pair, quiet_after, quiet_before,
};
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::runs::Runs;

/// Characters by value: the 43 of the basic set in check-digit order.
const CHARSET: &[u8; 43] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";
/// Values 43 to 46 are the shift characters ($), (%), (/) and (+); they take
/// the letter after them to the Full ASCII character ([`full_ascii_pair`]).
const SHIFT_LEADS: [u8; 4] = *b"$%/+";
const FIRST_SHIFT: u8 = 43;
const STAR: u8 = 47;
const NONE: u8 = 0xff;

/// Element widths in modules (bar, space, bar, space, bar, space) of the 47
/// characters by value, then of the start/stop character, as in the
/// specification's table.
const PATTERNS: [&str; 48] = [
    "131112", "111213", "111312", "111411", "121113", "121212", "121311", "111114", "131211",
    "141111", "211113", "211212", "211311", "221112", "221211", "231111", "112113", "112212",
    "112311", "122112", "132111", "111123", "111222", "111321", "121122", "131121", "212112",
    "212211", "211122", "211221", "221121", "222111", "112122", "112221", "122121", "123111",
    "121131", "311112", "311211", "321111", "112131", "113121", "211131", "121221", "312111",
    "311121", "122211", "111141",
];

/// Character value for each element-width key (two bits per element, the
/// width minus one), `NONE` where no character has that shape.
static BY_KEY: [u8; 4096] = {
    let mut t = [NONE; 4096];
    let mut v = 0;
    while v < 48 {
        let p = PATTERNS[v].as_bytes();
        let mut key = 0usize;
        let mut e = 0;
        while e < 6 {
            key |= ((p[e] - b'1') as usize) << (2 * e);
            e += 1;
        }
        t[key] = v as u8;
        v += 1;
    }
    t
};

/// Fewest characters between the start character and the check characters.
/// The specification has no minimum; a one-character symbol is guarded by a
/// check pair that random shapes pass one time in 2209 only, which is not
/// enough alone.
pub(crate) const MIN_DATA_LENGTH: usize = 2;
/// Most characters between start and stop, check characters included.
const MAX_CHARS: usize = 130;
/// Quiet zone on each side, in modules, measured: the part of the margin that
/// lies inside the profile counts, the profile border never makes it open.
/// The specification asks for 10; labels cropped to their symbol keep 4 to 6.
const QUIET: f32 = 4.0;
/// Cheap first screen on the start character: the quiet zone as a fraction
/// of the character's width (4 modules are 0.44 of a character of 9; 0.35
/// leaves room for the print gain).
const QUIET_SCREEN: f32 = 0.35;
/// The termination bar is one module wide, within this many modules.
const TERMINATION_TOLERANCE: f32 = 0.5;
/// The 4-module bar of the start character over the widest of its other
/// elements, as a first screen (the real ratio is 4 over 1 to 1.3).
const WIDE_SCREEN: f32 = 2.0;
/// Each inner edge of a character must lie within this many modules of a
/// whole number of modules from the character's first edge, and the five
/// distances must average no more than `MEAN_TOLERANCE`.
const TOLERANCE: f32 = 0.4;
const MEAN_TOLERANCE: f32 = 0.22;
/// Allowed ratio between the module widths of neighboring characters.
const MODULE_DRIFT: f32 = 1.4;
/// Allowed ratio between the largest and smallest module width in a symbol.
const SYMBOL_DRIFT: f32 = 1.8;
/// Largest print-gain bias of the start character, in modules.
const MAX_BIAS: f32 = 0.3;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Code93Decoder;

struct CharRead {
    value: u8,
    module: f32,
    /// Largest distance of an element to a whole number of modules.
    worst: f32,
}

/// Reads the six runs from `at` as one character.
fn read_char(runs: &Runs, at: usize, bias: f32, prev_module: f32) -> Option<CharRead> {
    let w = adjusted_widths::<6>(runs, at, bias)?;
    let module = w.iter().sum::<f32>() / 9.0;
    if module < MIN_MODULE
        || (prev_module > 0.0
            && (module < prev_module / MODULE_DRIFT || module > prev_module * MODULE_DRIFT))
    {
        return None;
    }
    // The five inner edges, in modules from the character's first edge, must
    // each lie close to a whole number; the elements are the differences.
    let (mut key, mut edge, mut last, mut worst, mut total) =
        (0usize, 0.0f32, 0i32, 0.0f32, 0.0f32);
    for (e, &width) in w.iter().enumerate() {
        edge += width / module;
        let whole = if e == 5 { 9 } else { edge.round() as i32 };
        let q = whole - last;
        let err = (edge - whole as f32).abs();
        if !(1..=4).contains(&q) || err > TOLERANCE {
            return None;
        }
        worst = worst.max(err);
        total += err;
        key |= (q as usize - 1) << (2 * e);
        last = whole;
    }
    if total > MEAN_TOLERANCE * 5.0 {
        return None;
    }
    let value = BY_KEY[key];
    (value != NONE).then_some(CharRead {
        value,
        module,
        worst,
    })
}

struct Read {
    values: [u8; MAX_CHARS],
    len: usize,
    module_sum: f32,
    worst_sum: f32,
    count: usize,
    last_bar: usize,
}

/// Reads the start character at run `j`; returns it with the bar/space bias.
fn start_of(runs: &Runs, j: usize) -> Option<(CharRead, f32)> {
    let raw = adjusted_widths::<6>(runs, j, 0.0)?;
    let others = raw[0].max(raw[1]).max(raw[2]).max(raw[3]).max(raw[5]);
    if raw[4] < WIDE_SCREEN * others {
        return None;
    }
    let total: f32 = raw.iter().sum();
    if runs.space_before(j) < QUIET_SCREEN * total {
        return None;
    }
    // Bars 1 + 1 + 4 and spaces 1 + 1 + 1 modules: the print gain g widens
    // every bar and narrows every space by g, so bars - spaces = 3 m + 6 g.
    let module = total / 9.0;
    let bias = ((raw[0] + raw[2] + raw[4]) - (raw[1] + raw[3] + raw[5]) - total / 3.0) / 6.0;
    if module < MIN_MODULE || bias.abs() > MAX_BIAS * module {
        return None;
    }
    let start = read_char(runs, j, bias, 0.0)?;
    if start.value != STAR || quiet_before(runs, j, bias) < QUIET * start.module {
        return None;
    }
    Some((start, bias))
}

fn read_symbol(runs: &Runs, j: usize, started: &mut bool) -> Option<Read> {
    let (start, bias) = start_of(runs, j)?;
    *started = true;
    let mut read = Read {
        values: [0; MAX_CHARS],
        len: 0,
        module_sum: start.module,
        worst_sum: start.worst,
        count: 1,
        last_bar: 0,
    };
    let (mut prev, mut lo, mut hi) = (start.module, start.module, start.module);
    let mut at = j + 6;
    loop {
        let c = read_char(runs, at, bias, prev)?;
        prev = c.module;
        lo = lo.min(prev);
        hi = hi.max(prev);
        if hi > lo * SYMBOL_DRIFT {
            return None;
        }
        read.module_sum += c.module;
        read.worst_sum += c.worst;
        read.count += 1;
        if c.value == STAR {
            // The termination bar is one module wide and the quiet zone
            // follows it.
            let bar = at + 6;
            let width = adjusted_width(runs, bar, bias)?;
            if (width / c.module - 1.0).abs() > TERMINATION_TOLERANCE
                || quiet_after(runs, bar, bias) < QUIET * c.module
            {
                return None;
            }
            read.last_bar = bar;
            return Some(read);
        }
        if read.len == MAX_CHARS {
            return None;
        }
        read.values[read.len] = c.value;
        read.len += 1;
        at += 6;
    }
}

/// Check character over `values`, weights 1 to `period` from the right,
/// modulo 47.
fn check_of(values: &[u8], period: usize) -> u8 {
    let sum: usize = values
        .iter()
        .rev()
        .enumerate()
        .map(|(k, &v)| usize::from(v) * (k % period + 1))
        .sum();
    (sum % 47) as u8
}

/// The payload of `values` (data characters without the check characters) as
/// ASCII bytes. `None` for a shift character that does not take a letter
/// whose pair is assigned.
fn expand(values: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(values.len());
    let mut it = values.iter().copied();
    while let Some(v) = it.next() {
        if v < FIRST_SHIFT {
            out.push(CHARSET[usize::from(v)]);
            continue;
        }
        let lead = *SHIFT_LEADS.get(usize::from(v - FIRST_SHIFT))?;
        let letter = match it.next() {
            Some(l @ 10..=35) => b'A' + (l - 10),
            _ => return None,
        };
        out.push(full_ascii_pair(lead, letter)?);
    }
    Some(out)
}

/// Run widths, in modules, of the symbol for the data `values` (each 0 to
/// 46) with its check characters, and the text it stands for. For fuzzing.
pub(crate) fn symbol_runs(values: &[u8]) -> (Option<Vec<u8>>, Vec<u8>) {
    let mut all = values.to_vec();
    all.push(check_of(values, 20));
    all.push(check_of(&all, 15));
    let mut widths = Vec::new();
    for &v in std::iter::once(&STAR).chain(&all).chain(&[STAR]) {
        widths.extend(PATTERNS[usize::from(v)].bytes().map(|b| b - b'0'));
    }
    widths.push(1);
    let text = if values.len() >= MIN_DATA_LENGTH {
        expand(values)
    } else {
        None
    };
    (text, widths)
}

impl LinearDecoder for Code93Decoder {
    fn decode(&self, runs: &Runs, opts: &LineOptions, out: &mut Vec<LineHit>) -> bool {
        if !opts.symbologies.contains(Symbology::Code93) {
            return false;
        }
        let mut started = false;
        let n = runs.len();
        let mut j = usize::from(n > 0 && !runs.is_bar(0));
        // Start, one data and two check characters, stop, termination bar.
        while j + 6 * (3 + MIN_DATA_LENGTH) + 7 <= n {
            if let Some(read) = read_symbol(runs, j, &mut started)
                && read.len >= MIN_DATA_LENGTH + 2
            {
                let all = &read.values[..read.len];
                let (data, checks) = all.split_at(read.len - 2);
                let c_ok = check_of(data, 20) == checks[0];
                let k_ok = c_ok && check_of(&all[..read.len - 1], 15) == checks[1];
                if k_ok && let Some(text) = expand(data) {
                    let count = read.count as f32;
                    out.push(LineHit {
                        symbology: Symbology::Code93,
                        text,
                        addon: None,
                        start: runs.start(j),
                        end: runs.end(read.last_bar),
                        module: read.module_sum / count,
                        quality: (1.0 - 2.0 * read.worst_sum / count).clamp(0.0, 1.0),
                        aim: *b"]G0",
                        checksum_verified: true,
                        short_quiet: false,
                        inverted: false,
                        start_stop: None,
                        segments: 0,
                        thin: false,
                        part: false,
                    });
                }
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

    fn texts(hits: &[LineHit]) -> Vec<Vec<u8>> {
        hits.iter().map(|h| h.text.clone()).collect()
    }

    /// Profile of a symbol with the given data values and correct checks.
    fn prof(values: &[u8], px: f32) -> Vec<u8> {
        Render::px(px).profile(&code93_modules(values))
    }

    fn text_values(text: &str) -> Vec<u8> {
        text.bytes()
            .map(|b| CHARSET.iter().position(|&c| c == b).unwrap() as u8)
            .collect()
    }

    fn decode(p: &[u8]) -> Vec<LineHit> {
        decode_all(&Code93Decoder, p, &opts())
    }

    #[test]
    fn table_matches_the_specification_table_in_module_form() {
        for (v, bits) in CODE93_BITS.iter().enumerate() {
            assert_eq!(bits.len(), 9);
            let mut widths = String::new();
            let b = bits.as_bytes();
            let mut i = 0;
            while i < 9 {
                let mut k = i;
                while k < 9 && b[k] == b[i] {
                    k += 1;
                }
                widths.push(char::from(b'0' + (k - i) as u8));
                i = k;
            }
            assert_eq!(widths, PATTERNS[v], "value {v}");
            assert_eq!(b[0], b'1', "a character starts with a bar");
            assert_eq!(b[8], b'0', "and ends with a space");
            assert_eq!(BY_KEY.iter().filter(|&&x| usize::from(x) == v).count(), 1);
        }
        assert_eq!(BY_KEY.iter().filter(|&&x| x != NONE).count(), 48);
    }

    #[test]
    fn symbols_agree_with_an_independent_encoder() {
        // The 43 basic characters, doubled so that C and K take many values,
        // and the whole ASCII set through the shift characters.
        for (v, &ch) in CHARSET.iter().enumerate() {
            let body = [ch, ch];
            assert_eq!(
                code93_oracle_modules(&body, false),
                code93_modules(&[v as u8, v as u8]),
                "{}",
                ch as char
            );
        }
        for c in 0u8..128 {
            let text = [b'A', c, b'7'];
            assert_eq!(
                code93_oracle_modules(&text, true),
                code93_modules(&code93_ascii_values(&text)),
                "ASCII {c}"
            );
        }
        let mut rng = Rng(9393);
        for _ in 0..300 {
            let text: Vec<u8> = (0..2 + rng.below(30))
                .map(|_| rng.below(128) as u8)
                .collect();
            assert_eq!(
                code93_oracle_modules(&text, true),
                code93_modules(&code93_ascii_values(&text))
            );
        }
    }

    #[test]
    fn reads_every_character_in_both_directions_and_polarities() {
        let all = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";
        for text in ["CODE 93", "1234567890", all, "-. $/+%", "Z9"] {
            let values = text_values(text);
            for px in [1.75, 2.0, 3.0] {
                let p = prof(&values, px);
                let hits = decode(&p);
                assert_eq!(texts(&hits), [text.as_bytes().to_vec()], "{text} {px}");
                assert_eq!(&hits[0].aim, b"]G0");
                assert!(hits[0].checksum_verified && !hits[0].short_quiet);
                let mut rev = p.clone();
                rev.reverse();
                assert_eq!(texts(&decode(&rev)), [text.as_bytes().to_vec()], "rev");
                let r = Render {
                    dark: 230.0,
                    light: 20.0,
                    ..Render::px(px)
                };
                let hits = decode(&r.profile(&code93_modules(&values)));
                assert_eq!(texts(&hits), [text.as_bytes().to_vec()]);
                assert!(hits[0].inverted);
            }
        }
    }

    #[test]
    fn reads_through_noise_and_gain() {
        let mut rng = Rng(93);
        let modules = code93_modules(&text_values("NOISE42"));
        let mut ok = 0;
        for i in 0..40 {
            let r = Render {
                sigma: 0.45 + (i % 4) as f32 * 0.1,
                gain: (i % 5) as f32 * 0.07 - 0.14,
                offset: rng.unit(),
                ..Render::px(2.5)
            };
            let mut p = r.profile(&modules);
            add_noise(&mut p, 6.0, &mut rng);
            let hits = decode(&p);
            assert!(hits.iter().all(|h| h.text == b"NOISE42"), "{hits:?}");
            ok += usize::from(!hits.is_empty());
        }
        assert!(ok >= 34, "{ok}");
    }

    #[test]
    fn check_characters_use_weights_one_to_twenty_and_one_to_fifteen() {
        // 25 data characters: both weight sequences wrap.
        let values: Vec<u8> = (0..25).map(|i| (i * 7 % 43) as u8).collect();
        let (c, k) = code93_checks(&values);
        assert_eq!(check_of(&values, 20), c);
        let mut with_c = values.clone();
        with_c.push(c);
        assert_eq!(check_of(&with_c, 15), k);
        let hits = decode(&prof(&values, 3.0));
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].text.len(), 25);
        // A published reference: TEST93 has the checks "+" and "6".
        let (c, k) = code93_checks(&text_values("TEST93"));
        assert_eq!(
            (CHARSET[usize::from(c)], CHARSET[usize::from(k)]),
            (b'+', b'6')
        );
    }

    #[test]
    fn a_wrong_check_character_rejects_the_symbol() {
        let data = text_values("CHECKED");
        let (c, k) = code93_checks(&data);
        let render = |c: u8, k: u8| {
            let mut v = data.clone();
            v.extend([c, k]);
            Render::px(3.0).profile(&code93_modules_raw(&v))
        };
        assert_eq!(decode(&render(c, k)).len(), 1);
        assert!(decode(&render((c + 1) % 47, k)).is_empty(), "wrong C");
        assert!(decode(&render(c, (k + 1) % 47)).is_empty(), "wrong K");
        assert!(decode(&render(k, c)).is_empty(), "swapped");
        // K is computed over C: the K of a symbol with a wrong C fits the data alone only by chance.
        let mut data_c = data.clone();
        data_c.push(c);
        assert_eq!(check_of(&data_c, 15), k);
        // No check characters at all.
        assert!(decode(&Render::px(3.0).profile(&code93_modules_raw(&data))).is_empty());
        // Only C.
        let mut only_c = data.clone();
        only_c.push(c);
        assert!(decode(&Render::px(3.0).profile(&code93_modules_raw(&only_c))).is_empty());
    }

    #[test]
    fn full_ascii_through_the_shift_characters_round_trips_all_128_characters() {
        for c in 0u8..128 {
            // One plain character around it, so the text is long enough.
            let mut text = b"A".to_vec();
            text.push(c);
            let values = code93_ascii_values(&text);
            let hits = decode(&prof(&values, 3.0));
            assert_eq!(texts(&hits), [text], "ASCII {c}");
        }
        for chunk in (0u8..128).collect::<Vec<_>>().chunks(40) {
            let hits = decode(&prof(&code93_ascii_values(chunk), 2.0));
            assert_eq!(texts(&hits), [chunk.to_vec()]);
        }
        // The plain `$ / + %` stay themselves.
        let hits = decode(&prof(&text_values("A$/+%B"), 3.0));
        assert_eq!(texts(&hits), [b"A$/+%B".to_vec()]);
    }

    #[test]
    fn invalid_shift_sequences_reject_the_symbol() {
        const DOLLAR: u8 = 43;
        const PERCENT: u8 = 44;
        const SLASH: u8 = 45;
        const PLUS: u8 = 46;
        let a = 10;
        let cases: [(&str, Vec<u8>); 9] = [
            ("shift last", vec![a, a, PLUS]),
            ("shift then shift", vec![a, PLUS, PLUS, a]),
            ("shift then digit", vec![a, DOLLAR, 1, a]),
            ("shift then space", vec![a, SLASH, 38, a]),
            ("%X alias of DEL", vec![a, PERCENT, 33, a]),
            ("%Y alias of DEL", vec![a, PERCENT, 34, a]),
            ("/M", vec![a, SLASH, 22, a]),
            ("/N", vec![a, SLASH, 23, a]),
            ("/P", vec![a, SLASH, 25, a]),
        ];
        for (name, v) in cases {
            assert!(decode(&prof(&v, 3.0)).is_empty(), "{name}");
        }
        // The same shifts with an assigned letter read.
        for v in [
            vec![a, PERCENT, 29, a],
            vec![a, SLASH, 35, a],
            vec![a, DOLLAR, 10, a],
        ] {
            assert_eq!(decode(&prof(&v, 3.0)).len(), 1, "{v:?}");
        }
    }

    #[test]
    fn length_rule() {
        let one = text_values("7");
        assert!(decode(&prof(&one, 3.0)).is_empty(), "one data character");
        let two = text_values("78");
        assert_eq!(decode(&prof(&two, 3.0)).len(), 1);
        let long: Vec<u8> = (0..MAX_CHARS as u8 - 2).map(|i| i % 43).collect();
        assert_eq!(decode(&prof(&long, 2.0)).len(), 1, "longest accepted");
        let longer: Vec<u8> = (0..MAX_CHARS as u8 - 1).map(|i| i % 43).collect();
        assert!(decode(&prof(&longer, 2.0)).is_empty(), "one more");
    }

    #[test]
    fn quiet_zones_are_measured_on_both_sides_and_never_open() {
        let modules = code93_modules(&text_values("QUIET"));
        let at = |left: f32, right: f32| {
            let r = Render {
                quiet_left: left,
                quiet_right: right,
                ..Render::px(3.0)
            };
            decode(&r.profile(&modules))
        };
        for (l, r) in [(10.0, 10.0), (4.6, 4.6), (12.0, 4.6), (4.6, 12.0)] {
            assert_eq!(at(l, r).len(), 1, "{l} {r}");
        }
        // Below 4 modules on either side, or the profile ending at the symbol.
        for (l, r) in [
            (3.2, 12.0),
            (12.0, 3.2),
            (2.0, 2.0),
            (1.0, 12.0),
            (12.0, 1.0),
            (0.0, 12.0),
        ] {
            assert!(at(l, r).is_empty(), "{l} {r}");
        }
        // A neighbouring bar limits the quiet zone by content.
        let near = |left: usize, right: usize| {
            let p = Render::px(3.0).profile(&with_neighbours(&modules, left, right));
            decode(&p)
        };
        assert_eq!(near(12, 12).len(), 1);
        assert_eq!(near(5, 5).len(), 1);
        for (l, r) in [(3, 12), (12, 3), (2, 2)] {
            assert!(near(l, r).is_empty(), "{l} {r}");
        }
    }

    #[test]
    fn a_symbol_cut_by_the_border_is_not_read() {
        let modules = code93_modules(&text_values("CUT BY THE BORDER"));
        let r = Render {
            quiet_left: 8.0,
            quiet_right: 8.0,
            ..Render::px(3.0)
        };
        let p = r.profile(&modules);
        assert_eq!(decode(&p).len(), 1);
        for (depth, cut) in border_cuts(&p, 24.0, modules.len() as f32 * 3.0) {
            assert!(decode(&cut).is_empty(), "depth {depth}");
        }
    }

    #[test]
    fn start_stop_and_termination_bar_are_all_required() {
        let values = text_values("PARTS");
        let full = code93_modules(&values);
        assert_eq!(decode(&Render::px(3.0).profile(&full)).len(), 1);
        // Without the termination bar.
        let no_bar = &full[..full.len() - 1];
        assert!(decode(&Render::px(3.0).profile(no_bar)).is_empty());
        // Without the stop character (and its bar), without the start.
        let no_stop = &full[..full.len() - 10];
        assert!(decode(&Render::px(3.0).profile(no_stop)).is_empty());
        let no_start = &full[9..];
        assert!(decode(&Render::px(3.0).profile(no_start)).is_empty());
        // A termination bar of two modules, or of three.
        for extra in [2, 3] {
            let mut wide = full.clone();
            wide.extend(vec![true; extra - 1]);
            assert!(
                decode(&Render::px(3.0).profile(&wide)).is_empty(),
                "{extra}"
            );
        }
        // A bare data run is not a symbol.
        let mut bare = Vec::new();
        for &v in values.iter().chain(&[0u8, 1]) {
            bare.extend(CODE93_BITS[usize::from(v)].bytes().map(|b| b == b'1'));
        }
        assert!(decode(&Render::px(3.0).profile(&bare)).is_empty());
    }

    #[test]
    fn a_star_inside_a_symbol_ends_it() {
        // `*` in the middle is a stop character: what follows is not part of
        // the symbol, and the part before has no valid checks.
        let mut v = text_values("AB");
        v.push(STAR);
        v.extend(text_values("CD"));
        let (c, k) = code93_checks(&v);
        v.extend([c, k]);
        let p = Render::px(3.0).profile(&code93_modules_raw(&v));
        assert!(decode(&p).is_empty());
    }

    #[test]
    fn element_widths_must_quantize_to_the_nine_module_structure() {
        // A character drawn with a one-module bar replaced by a one and a
        // half module bar, the space after it one half short: the pattern
        // is a shape no character has.
        let bits = code93_modules(&text_values("SHAPE"));
        let runs = |b: &[bool]| {
            let mut w = Vec::new();
            for (i, &x) in b.iter().enumerate() {
                if i > 0 && b[i - 1] == x {
                    *w.last_mut().unwrap() += 1.0f32;
                } else {
                    w.push(1.0);
                }
            }
            w
        };
        let clean = runs(&bits);
        let widths = |edit: &dyn Fn(&mut Vec<f32>)| {
            let mut w: Vec<f32> = clean.iter().map(|x| x * 3.0).collect();
            edit(&mut w);
            Runs::from_widths(true, &w, 40.0, 40.0)
        };
        let decode_runs = |r: &Runs| {
            let mut out = Vec::new();
            Code93Decoder.decode(r, &opts(), &mut out);
            out
        };
        assert_eq!(decode_runs(&widths(&|_| {})).len(), 1);
        // Elements of the first two data characters moved by half a module.
        for (i, shift) in [(8usize, 1.5f32), (9, -1.5), (12, 1.5), (13, -1.5)] {
            let r = widths(&|w| {
                w[i] += shift;
                w[i + 1] -= shift;
            });
            assert!(decode_runs(&r).is_empty(), "{i} {shift}");
        }
        // Moved by 0.2 modules: read.
        let r = widths(&|w| {
            w[8] += 0.6;
            w[9] -= 0.6;
        });
        assert_eq!(decode_runs(&r).len(), 1);
    }

    #[test]
    fn module_drift_is_bounded() {
        // The right half of the symbol is 1.9 times wider than the left.
        let bits = code93_modules(&text_values("DRIFTING"));
        let mut w = Vec::new();
        for (i, &x) in bits.iter().enumerate() {
            if i > 0 && bits[i - 1] == x {
                *w.last_mut().unwrap() += 1.0f32;
            } else {
                w.push(1.0);
            }
        }
        let scaled = |f: f32| {
            let half = w.len() / 2;
            let ws: Vec<f32> = w
                .iter()
                .enumerate()
                .map(|(i, x)| x * 3.0 * if i >= half { f } else { 1.0 })
                .collect();
            let r = Runs::from_widths(true, &ws, 80.0, 80.0);
            let mut out = Vec::new();
            Code93Decoder.decode(&r, &opts(), &mut out);
            out.len()
        };
        assert_eq!(scaled(1.0), 1);
        assert_eq!(scaled(2.0), 0);
    }

    #[test]
    fn code_128_code_39_and_ean_are_not_read_as_code_93() {
        for p in [
            Render::px(3.0).profile(&code128_modules(&[104, b('9'), b('3'), b('A')])),
            Render::px(3.0).profile(&code39_modules("CODE93", 3, 1)),
            Render::px(3.0).profile(&ean13_modules("5901234123457")),
            Render::px(2.0).profile(&itf_modules("12345678901231", 3)),
        ] {
            assert!(decode(&p).is_empty());
        }
    }

    #[test]
    fn perturbed_widths_are_read_correctly_or_not_at_all() {
        let mut rng = Rng(77);
        let text = "AMBIGUOUS93";
        let modules = code93_modules(&text_values(text));
        let mut w = Vec::new();
        for (i, &x) in modules.iter().enumerate() {
            if i > 0 && modules[i - 1] == x {
                *w.last_mut().unwrap() += 1.0f32;
            } else {
                w.push(1.0);
            }
        }
        let (mut read, mut missed) = (0, 0);
        for _ in 0..4000 {
            let jitter = 0.05 + rng.unit() * 0.4;
            let ws: Vec<f32> = w
                .iter()
                .map(|x| (x * 3.0 + (rng.unit() - 0.5) * 2.0 * jitter * 3.0).max(0.5))
                .collect();
            let r = Runs::from_widths(true, &ws, 60.0, 60.0);
            let mut out = Vec::new();
            Code93Decoder.decode(&r, &opts(), &mut out);
            assert!(out.iter().all(|h| h.text == text.as_bytes()), "{out:?}");
            if out.is_empty() {
                missed += 1;
            } else {
                read += 1;
            }
        }
        assert!(read > 1500 && missed > 0, "{read} {missed}");
    }

    #[test]
    fn disabled_symbology_is_not_decoded() {
        let p = prof(&text_values("OFF"), 3.0);
        let off = line_options(|o| {
            o.symbologies = o
                .symbologies
                .iter()
                .filter(|s| *s != Symbology::Code93)
                .collect();
        });
        assert!(decode_all(&Code93Decoder, &p, &off).is_empty());
    }

    #[test]
    fn hostile_run_sequences_do_not_panic() {
        let mut rng = Rng(5);
        for _ in 0..2000 {
            let n = rng.below(200) as usize;
            let ws: Vec<f32> = (0..n)
                .map(|_| match rng.below(6) {
                    0 => 0.0,
                    1 => f32::NAN,
                    2 => f32::INFINITY,
                    3 => -1.0,
                    _ => rng.unit() * 12.0,
                })
                .collect();
            for first in [true, false] {
                let r = Runs::from_widths(first, &ws, rng.unit() * 40.0, rng.unit() * 40.0);
                let mut out = Vec::new();
                Code93Decoder.decode(&r, &opts(), &mut out);
            }
        }
    }
}
