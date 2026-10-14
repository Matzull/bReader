//! Code 39 with optional mod-43 check character and Full ASCII, following
//! ISO/IEC 16388.

use breader_core::Symbology;

use crate::common::{
    MIN_MODULE, adjusted_width, adjusted_widths, quiet_after, quiet_after_open, quiet_before,
    quiet_before_open, split_wide,
};
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::runs::Runs;

/// Characters in check-digit order: the position is the value used by the
/// mod-43 check character.
const CHARSET: &[u8; 43] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";

/// Order in which the 40 characters built from two wide elements are
/// listed in the specification.
const PAIRED: &[u8; 40] = b"1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ-. *";
/// The two wide elements of each character of a group of ten, as element
/// indices of the nine alternating bars and spaces.
const PAIRS: [(u8, u8); 10] = [
    (0, 8),
    (2, 8),
    (0, 2),
    (4, 8),
    (0, 4),
    (2, 4),
    (6, 8),
    (0, 6),
    (2, 6),
    (4, 6),
];
/// The third wide element shared by the characters of each group of ten.
const MARKERS: [u8; 4] = [3, 5, 7, 1];
/// `$`, `/`, `+` and `%` have three wide spaces.
const SPACE_TRIPLES: [(u8, [u8; 3]); 4] = [
    (b'$', [1, 3, 5]),
    (b'/', [1, 3, 7]),
    (b'+', [1, 5, 7]),
    (b'%', [3, 5, 7]),
];
const STAR: u8 = b'*';

/// Character for each 9-bit mask of wide elements, 0 where none exists.
static BY_MASK: [u8; 512] = {
    let mut t = [0u8; 512];
    let mut k = 0;
    while k < 40 {
        let (a, b) = PAIRS[k % 10];
        let mask = (1u16 << a) | (1 << b) | (1 << MARKERS[k / 10]);
        t[mask as usize] = PAIRED[k];
        k += 1;
    }
    let mut s = 0;
    while s < 4 {
        let (c, e) = SPACE_TRIPLES[s];
        t[((1u16 << e[0]) | (1 << e[1]) | (1 << e[2])) as usize] = c;
        s += 1;
    }
    t
};

const STAR_MASK: u16 = (1 << 1) | (1 << 4) | (1 << 6);

/// Minimum quiet zone on each side, in narrow element widths. The standard
/// asks for 10; printed labels often have 4 to 7 because of a cell border.
const QUIET: f32 = 4.0;
/// Below this many narrow widths the symbol is flagged `short_quiet` and the
/// aggregator wants an additional agreeing line.
const FULL_QUIET: f32 = 7.0;
/// Quiet zone of a symbol read with the relaxed rules, measured, never open.
const THIN_QUIET: f32 = 7.0;
/// Relaxed rules apply to symbols whose narrow element is at most this many
/// samples wide, and only where the normal ones failed.
const THIN_MAX_MODULE: f32 = 2.4;
/// Relaxed separation of the smallest wide over the largest narrow element.
const THIN_SEPARATION: f32 = 1.12;
/// Cheap first screen: the quiet zone as a fraction of the start character's
/// width (4 narrow elements are at least 0.22 of a character of up to 17.6).
const QUIET_SCREEN: f32 = 0.22;
/// Largest inter-character gap, in narrow widths; larger gaps split symbols.
const MAX_GAP: f32 = 3.0;
const MIN_GAP: f32 = 0.5;
/// Accepted ratio between the mean wide and the mean narrow element.
const RATIO: (f32, f32) = (1.6, 3.6);
/// The smallest wide element must exceed the largest narrow one by this factor.
const MIN_SEPARATION: f32 = 1.3;
/// Allowed ratio between the narrow widths of neighboring characters.
const MODULE_DRIFT: f32 = 1.7;
const MAX_CHARS: usize = 256;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Code39Decoder;

struct CharRead {
    ch: u8,
    narrow: f32,
    separation: f32,
}

fn read_char(runs: &Runs, at: usize, bias: f32, prev_narrow: f32, thin: bool) -> Option<CharRead> {
    let w = adjusted_widths::<9>(runs, at, bias)?;
    let s = split_wide(&w, 3)?;
    let ratio = s.wide / s.narrow;
    let min_sep = if thin {
        THIN_SEPARATION
    } else {
        MIN_SEPARATION
    };
    if s.narrow < MIN_MODULE
        || s.separation < min_sep
        || !(RATIO.0..=RATIO.1).contains(&ratio)
        || (prev_narrow > 0.0
            && (s.narrow < prev_narrow / MODULE_DRIFT || s.narrow > prev_narrow * MODULE_DRIFT))
    {
        return None;
    }
    let ch = BY_MASK[usize::from(s.wide_mask) & 0x1ff];
    (ch != 0).then_some(CharRead {
        ch,
        narrow: s.narrow,
        separation: s.separation,
    })
}

struct Read {
    chars: [u8; MAX_CHARS],
    len: usize,
    separation_sum: f32,
    narrow_sum: f32,
    count: usize,
    last_bar: usize,
    short_quiet: bool,
}

/// Reads the start character at run `j` and returns it with the bar/space
/// bias and whether the quiet zone before it is short.
fn start_of(runs: &Runs, j: usize, thin: bool) -> Option<(CharRead, f32, bool)> {
    let raw = adjusted_widths::<9>(runs, j, 0.0)?;
    if runs.space_before_open(j) < QUIET_SCREEN * raw.iter().sum::<f32>() {
        return None;
    }
    if split_wide(&raw, 3)?.wide_mask != STAR_MASK {
        return None;
    }
    let bars = (raw[0] + raw[2] + raw[8]) / 3.0;
    let spaces = (raw[3] + raw[5] + raw[7]) / 3.0;
    let bias = (bars - spaces) / 2.0;
    let module = (bars + spaces) / 2.0;
    if module < MIN_MODULE || bias.abs() > 0.45 * module {
        return None;
    }
    if thin && module > THIN_MAX_MODULE {
        return None;
    }
    let start = read_char(runs, j, bias, 0.0, thin)?;
    if start.ch != STAR {
        return None;
    }
    if thin {
        return (quiet_before(runs, j, bias) >= THIN_QUIET * start.narrow)
            .then_some((start, bias, false));
    }
    // A margin that runs into the border is accepted whatever its length, but
    // its measured length still decides whether it is short.
    let short = quiet_before(runs, j, bias) < FULL_QUIET * start.narrow;
    (quiet_before_open(runs, j, bias) >= QUIET * start.narrow).then_some((start, bias, short))
}

fn read_symbol(runs: &Runs, j: usize, started: &mut bool, thin: bool) -> Option<Read> {
    let (start, bias, short_start) = start_of(runs, j, thin)?;
    *started = true;
    let mut read = Read {
        chars: [0; MAX_CHARS],
        len: 0,
        separation_sum: start.separation,
        narrow_sum: start.narrow,
        count: 1,
        last_bar: 0,
        short_quiet: short_start,
    };
    let mut narrow = start.narrow;
    let mut at = j + 10;
    loop {
        let gap = adjusted_width(runs, at - 1, bias)?;
        if gap > MAX_GAP * narrow || gap < MIN_GAP * narrow {
            return None;
        }
        let c = read_char(runs, at, bias, narrow, thin)?;
        narrow = c.narrow;
        read.separation_sum += c.separation;
        read.narrow_sum += c.narrow;
        read.count += 1;
        if c.ch == STAR {
            if thin {
                if quiet_after(runs, at + 8, bias) < THIN_QUIET * narrow {
                    return None;
                }
            } else if quiet_after_open(runs, at + 8, bias) < QUIET * narrow {
                return None;
            }
            read.short_quiet |= quiet_after(runs, at + 8, bias) < FULL_QUIET * narrow;
            read.last_bar = at + 8;
            return Some(read);
        }
        if read.len == MAX_CHARS {
            return None;
        }
        read.chars[read.len] = c.ch;
        read.len += 1;
        at += 10;
    }
}

fn value_of(ch: u8) -> Option<u32> {
    CHARSET.iter().position(|&c| c == ch).map(|p| p as u32)
}

/// True when the last character is the mod-43 check of the ones before it.
fn check_ok(chars: &[u8]) -> bool {
    let Some((&check, body)) = chars.split_last() else {
        return false;
    };
    let sum: u32 = body.iter().filter_map(|&c| value_of(c)).sum();
    value_of(check) == Some(sum % 43)
}

/// Full ASCII: `$`, `%`, `/` and `+` combine with a letter into one character.
fn expand_full_ascii(chars: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(chars.len());
    let mut it = chars.iter().copied();
    while let Some(c) = it.next() {
        if !matches!(c, b'$' | b'%' | b'/' | b'+') {
            out.push(c);
            continue;
        }
        let x = it.next()?;
        if !x.is_ascii_uppercase() {
            return None;
        }
        let k = x - b'A';
        out.push(match c {
            b'$' => k + 1,
            b'+' => b'a' + k,
            b'/' => match x {
                b'A'..=b'L' | b'O' => 33 + k,
                b'Z' => b':',
                _ => return None,
            },
            _ => match x {
                b'A'..=b'E' => 27 + k,
                b'F'..=b'J' => 59 + (k - 5),
                b'K'..=b'O' => 91 + (k - 10),
                b'P'..=b'S' => 123 + (k - 15),
                b'T' => 127,
                b'U' => 0,
                b'V' => b'@',
                b'W' => b'`',
                _ => return None,
            },
        });
    }
    Some(out)
}

impl LinearDecoder for Code39Decoder {
    fn decode(&self, runs: &Runs, opts: &LineOptions, out: &mut Vec<LineHit>) -> bool {
        if !opts.symbologies.contains(Symbology::Code39) {
            return false;
        }
        let mut started = false;
        let n = runs.len();
        let mut j = usize::from(n > 0 && !runs.is_bar(0));
        while j + 9 + 10 + 9 <= n {
            let mut thin = false;
            let mut read = read_symbol(runs, j, &mut started, false);
            if read.is_none() {
                read = read_symbol(runs, j, &mut false, true);
                thin = read.is_some();
            }
            if let Some(read) = read {
                let mut data = &read.chars[..read.len];
                let checked = opts.code39_checksum;
                if checked {
                    if !check_ok(data) {
                        j += 2;
                        continue;
                    }
                    data = &data[..data.len() - 1];
                }
                let text = if opts.code39_full_ascii {
                    expand_full_ascii(data)
                } else {
                    Some(data.to_vec())
                };
                if let Some(text) = text
                    && text.len() >= opts.code39_min_length
                    && !text.is_empty()
                {
                    let count = read.count as f32;
                    out.push(LineHit {
                        symbology: Symbology::Code39,
                        text,
                        addon: None,
                        start: runs.start(j),
                        end: runs.end(read.last_bar),
                        module: read.narrow_sum / count,
                        quality: ((read.separation_sum / count - 1.0) / 2.0).clamp(0.0, 1.0),
                        aim: [
                            b']',
                            b'A',
                            b'0' + [0, 3, 4, 7]
                                [usize::from(opts.code39_full_ascii) * 2 + usize::from(checked)],
                        ],
                        checksum_verified: checked,
                        short_quiet: read.short_quiet,
                        inverted: false,
                        start_stop: None,
                        segments: 0,
                        thin,
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

    fn prof(body: &str, wide: usize, px: f32) -> Vec<u8> {
        Render::px(px).profile(&code39_modules(body, wide, 1))
    }

    #[test]
    fn generated_table_matches_the_specification_table() {
        for (ch, pattern) in CODE39_TABLE {
            let mask = pattern
                .bytes()
                .enumerate()
                .fold(0usize, |m, (i, b)| m | usize::from(b == b'w') << i);
            assert_eq!(BY_MASK[mask], ch as u8, "{ch}");
            assert_eq!(pattern.bytes().filter(|&b| b == b'w').count(), 3);
        }
        assert_eq!(BY_MASK.iter().filter(|&&c| c != 0).count(), 44);
    }

    #[test]
    fn reads_every_character_in_both_directions_and_polarities() {
        for text in [
            "0123456789",
            "ABCDEFGHIJKLMNOPQRSTUVWXYZ",
            "-. $/+%",
            "CODE-39",
        ] {
            for wide in [2, 3] {
                for px in [1.5, 2.0, 3.0] {
                    if wide == 2 && px < 2.0 {
                        continue;
                    }
                    let p = prof(text, wide, px);
                    let hits = decode_all(&Code39Decoder, &p, &opts());
                    assert_eq!(
                        texts(&hits),
                        [text.as_bytes().to_vec()],
                        "{text} {wide} {px}"
                    );
                    assert_eq!(&hits[0].aim, b"]A0");
                    assert!(!hits[0].checksum_verified);
                    let mut rev = p.clone();
                    rev.reverse();
                    let hits = decode_all(&Code39Decoder, &rev, &opts());
                    assert_eq!(texts(&hits), [text.as_bytes().to_vec()], "rev {text}");
                    let r = Render {
                        dark: 230.0,
                        light: 20.0,
                        ..Render::px(px)
                    };
                    let hits = decode_all(
                        &Code39Decoder,
                        &r.profile(&code39_modules(text, wide, 1)),
                        &opts(),
                    );
                    assert_eq!(texts(&hits), [text.as_bytes().to_vec()]);
                    assert!(hits[0].inverted);
                }
            }
        }
    }

    #[test]
    fn reads_through_noise_and_gain() {
        let mut rng = Rng(11);
        let modules = code39_modules("NOISE42", 3, 1);
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
            let hits = decode_all(&Code39Decoder, &p, &opts());
            assert!(hits.iter().all(|h| h.text == b"NOISE42"), "{hits:?}");
            ok += usize::from(!hits.is_empty());
        }
        assert!(ok >= 36, "{ok}");
    }

    #[test]
    fn length_and_quiet_zone_rules() {
        let p = prof("AB", 3, 3.0);
        assert!(
            decode_all(&Code39Decoder, &p, &opts()).is_empty(),
            "min length 3"
        );
        let short_ok = line_options(|o| o.code39.min_length = 2);
        assert_eq!(decode_all(&Code39Decoder, &p, &short_ok).len(), 1);
        let long = line_options(|o| o.code39.min_length = 8);
        assert!(decode_all(&Code39Decoder, &prof("ABCDEFG", 3, 3.0), &long).is_empty());

        let modules = code39_modules("QUIET", 3, 1);
        let at = |left: f32, right: f32| {
            let r = Render {
                quiet_left: left,
                quiet_right: right,
                ..Render::px(3.0)
            };
            decode_all(&Code39Decoder, &r.profile(&modules), &opts())
        };
        // Narrow elements are one module wide here. A neighboring bar limits
        // the quiet zone by content.
        let near = |left: usize, right: usize| {
            let p = Render::px(3.0).profile(&with_neighbours(&modules, left, right));
            decode_all(&Code39Decoder, &p, &opts())
        };
        let full = at(8.0, 8.0);
        assert_eq!((full.len(), full[0].short_quiet), (1, false));
        assert_eq!(
            (near(12, 12).len(), near(12, 12)[0].short_quiet),
            (1, false)
        );
        for (left, right) in [(5, 12), (12, 5), (5, 5)] {
            let hits = near(left, right);
            assert_eq!(hits.len(), 1, "{left} {right}");
            assert!(hits[0].short_quiet, "{left} {right}");
        }
        for (left, right) in [(3, 12), (12, 3), (3, 3)] {
            assert!(near(left, right).is_empty(), "{left} {right}");
        }
    }

    #[test]
    fn a_quiet_zone_ending_at_the_profile_border_is_open_but_flagged_short() {
        let modules = code39_modules("QUIET", 3, 1);
        for (left, right) in [(3.0, 12.0), (12.0, 3.0), (1.0, 1.0), (2.0, 4.0)] {
            let r = Render {
                quiet_left: left,
                quiet_right: right,
                ..Render::px(3.0)
            };
            let hits = decode_all(&Code39Decoder, &r.profile(&modules), &opts());
            assert_eq!(texts(&hits), [b"QUIET".to_vec()], "{left} {right}");
            assert!(hits[0].short_quiet, "{left} {right}");
        }
    }

    #[test]
    fn a_symbol_cut_by_the_border_is_not_read_shorter() {
        let modules = code39_modules("CUT BY THE BORDER", 3, 1);
        let r = Render {
            quiet_left: 4.0,
            quiet_right: 4.0,
            ..Render::px(3.0)
        };
        let p = r.profile(&modules);
        assert_eq!(decode_all(&Code39Decoder, &p, &opts()).len(), 1);
        for (depth, cut) in border_cuts(&p, 12.0, modules.len() as f32 * 3.0) {
            for hit in decode_all(&Code39Decoder, &cut, &opts()) {
                assert_eq!(hit.text, b"CUT BY THE BORDER", "depth {depth}");
            }
            if depth > 9 {
                assert!(
                    decode_all(&Code39Decoder, &cut, &opts()).is_empty(),
                    "depth {depth}"
                );
            }
        }
    }

    #[test]
    fn a_star_inside_a_symbol_does_not_start_a_second_symbol() {
        // The widest inter-character gap is below the shortest quiet zone.
        for gap in [1, 2, 3] {
            let p = Render::px(3.0).profile(&code39_modules("AB*CD*EF", 3, gap));
            assert!(
                decode_all(&Code39Decoder, &p, &opts()).is_empty(),
                "gap {gap}"
            );
        }
    }

    #[test]
    fn wide_gaps_between_characters_split_the_symbol() {
        for gap in [1, 2, 3] {
            let p = Render::px(3.0).profile(&code39_modules("GAPS", 3, gap));
            assert_eq!(
                decode_all(&Code39Decoder, &p, &opts()).len(),
                1,
                "gap {gap}"
            );
        }
        for gap in [4, 6, 12] {
            let p = Render::px(3.0).profile(&code39_modules("GAPS", 3, gap));
            assert!(
                decode_all(&Code39Decoder, &p, &opts()).is_empty(),
                "gap {gap}"
            );
        }
    }

    #[test]
    fn wide_to_narrow_ratio_must_be_plausible() {
        // A ratio of 4 is outside the accepted range.
        let p = prof("RATIO", 4, 2.0);
        assert!(decode_all(&Code39Decoder, &p, &opts()).is_empty());
        let flat = Render::px(2.0).profile(&runs_to_modules(&[1; 80]));
        assert!(decode_all(&Code39Decoder, &flat, &opts()).is_empty());
    }

    #[test]
    fn mod43_checksum() {
        let body = "CODE39";
        let sum: usize = body
            .chars()
            .map(|c| CHARSET.iter().position(|&x| x as char == c).unwrap())
            .sum();
        let check = CHARSET[sum % 43] as char;
        let with = format!("{body}{check}");
        let need = line_options(|o| o.code39.require_checksum = true);
        let hits = decode_all(&Code39Decoder, &prof(&with, 3, 3.0), &need);
        assert_eq!(texts(&hits), [body.as_bytes().to_vec()]);
        assert_eq!(&hits[0].aim, b"]A3");
        assert!(hits[0].checksum_verified);
        let bad = CHARSET[(sum + 1) % 43] as char;
        let wrong = format!("{body}{bad}");
        assert!(decode_all(&Code39Decoder, &prof(&wrong, 3, 3.0), &need).is_empty());
        // Without the option the check character is plain data.
        let hits = decode_all(&Code39Decoder, &prof(&with, 3, 3.0), &opts());
        assert_eq!(texts(&hits), [with.as_bytes().to_vec()]);
    }

    #[test]
    fn full_ascii_expansion() {
        let full = line_options(|o| o.code39.full_ascii = true);
        let hits = decode_all(&Code39Decoder, &prof("+A+B$M/L%U", 3, 3.0), &full);
        assert_eq!(texts(&hits), [b"ab\r,\x00".to_vec()]);
        assert_eq!(&hits[0].aim, b"]A4");
        let both = line_options(|o| {
            o.code39.min_length = 2;
            o.code39.full_ascii = true;
            o.code39.require_checksum = true;
        });
        let body = "+AB";
        let sum: usize = body
            .chars()
            .map(|c| CHARSET.iter().position(|&x| x as char == c).unwrap())
            .sum();
        let with = format!("{body}{}", CHARSET[sum % 43] as char);
        let hits = decode_all(&Code39Decoder, &prof(&with, 3, 3.0), &both);
        assert_eq!(texts(&hits), [b"aB".to_vec()]);
        assert_eq!(&hits[0].aim, b"]A7");
        // Invalid pairs reject the whole symbol.
        for bad in ["A$1B", "AB+", "A/MB", "A%XB", "AB$-"] {
            assert!(
                decode_all(&Code39Decoder, &prof(bad, 3, 3.0), &full).is_empty(),
                "{bad}"
            );
        }
        // Not applied when disabled.
        let hits = decode_all(&Code39Decoder, &prof("A$1B", 3, 3.0), &opts());
        assert_eq!(texts(&hits), [b"A$1B".to_vec()]);
    }

    #[test]
    fn full_ascii_covers_all_128_characters_exactly_once() {
        let mut seen = [0u8; 128];
        for &c in b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. " {
            seen[usize::from(c)] += 1;
        }
        for lead in *b"$%/+" {
            for x in b'A'..=b'Z' {
                if let Some(out) = expand_full_ascii(&[lead, x]) {
                    assert_eq!(out.len(), 1);
                    seen[usize::from(out[0])] += 1;
                }
            }
        }
        assert!(seen.iter().all(|&n| n == 1), "{seen:?}");
    }

    #[test]
    fn bare_characters_without_start_and_stop_and_substrings_are_not_read() {
        let text = "ABC123";
        let modules = code39_modules(text, 3, 1);
        // Drop the start character: the rest must not be taken for a symbol.
        let tail = &modules[10..];
        assert!(decode_all(&Code39Decoder, &Render::px(3.0).profile(tail), &opts()).is_empty());
        // Drop the stop character.
        let head = &modules[..modules.len() - 10];
        assert!(decode_all(&Code39Decoder, &Render::px(3.0).profile(head), &opts()).is_empty());
        let hits = decode_all(&Code39Decoder, &Render::px(3.0).profile(&modules), &opts());
        assert_eq!(
            texts(&hits),
            [text.as_bytes().to_vec()],
            "only the full symbol"
        );
    }

    #[test]
    fn perturbed_widths_are_read_correctly_or_not_at_all() {
        let mut rng = Rng(7);
        let text = "AMBIGUOUS9";
        let modules = code39_modules(text, 2, 1);
        let (mut right, mut none) = (0, 0);
        for i in 0..400 {
            let amount = 0.1 + (i % 8) as f32 * 0.1;
            let mut dark = Vec::new();
            let mut x = 40.0f32;
            let mut j = 0;
            while j < modules.len() {
                let start = j;
                while j < modules.len() && modules[j] == modules[start] {
                    j += 1;
                }
                let w = ((j - start) as f32 + rng.gauss() * amount / 2.0).max(0.3) * 3.0;
                if modules[start] {
                    dark.push((x, x + w));
                }
                x += w;
            }
            let p = render_intervals(&dark, x as usize + 60, 25.0, 225.0, 0.6);
            let hits = decode_all(&Code39Decoder, &p, &opts());
            for h in &hits {
                assert_eq!(h.text, text.as_bytes(), "misread at {amount}");
            }
            if hits.is_empty() {
                none += 1
            } else {
                right += 1
            }
        }
        assert!(right > 50 && none > 0, "{right} {none}");
    }

    /// Narrow 1.4 to 1.6 px, wide twice the narrow: the third-largest element
    /// is not 1.3 times the fourth, so only the relaxed rules read it.
    fn thin_profile(text: &str, px: f32, sigma: f32, quiet: f32, rng: &mut Rng) -> Vec<u8> {
        let r = Render {
            sigma,
            quiet_left: quiet,
            quiet_right: quiet,
            offset: rng.unit(),
            ..Render::px(px)
        };
        let mut p = r.profile(&code39_modules(text, 2, 1));
        add_noise(&mut p, 4.0, rng);
        p
    }

    #[test]
    fn thin_symbols_are_read_with_the_relaxed_rules_and_flagged() {
        let mut rng = Rng(5);
        let (mut thin, mut plain) = (0, 0);
        for _ in 0..60 {
            let text = rng.digits(7);
            let p = thin_profile(&text, 1.6, 0.7, 12.0, &mut rng);
            for h in decode_all(&Code39Decoder, &p, &opts()) {
                assert_eq!(h.text, text.as_bytes());
                assert!(!h.short_quiet);
                if h.thin { thin += 1 } else { plain += 1 }
            }
        }
        assert!(thin >= 10 && plain > 0, "{thin} {plain}");
    }

    #[test]
    fn thin_symbols_need_seven_narrow_widths_of_quiet_zone_on_both_sides() {
        let mut rng = Rng(8);
        let text = "7654321";
        let reads = |left: f32, right: f32, rng: &mut Rng| {
            let mut n = 0;
            for _ in 0..40 {
                let r = Render {
                    sigma: 0.5,
                    quiet_left: left,
                    quiet_right: right,
                    offset: rng.unit(),
                    ..Render::px(1.4)
                };
                let mut p = r.profile(&code39_modules(text, 2, 1));
                add_noise(&mut p, 4.0, rng);
                n += decode_all(&Code39Decoder, &p, &opts())
                    .iter()
                    .filter(|h| h.thin)
                    .count();
            }
            n
        };
        assert!(reads(12.0, 12.0, &mut rng) > 3);
        assert_eq!(reads(5.0, 12.0, &mut rng), 0);
        assert_eq!(reads(12.0, 5.0, &mut rng), 0);
        assert_eq!(reads(6.0, 6.0, &mut rng), 0);
    }

    #[test]
    fn thin_rules_do_not_apply_above_the_module_limit() {
        // Wide 1.5 modules at 3 px: ambiguous, and too coarse for the relaxed rules.
        let mut rng = Rng(3);
        for _ in 0..40 {
            let text = rng.digits(7);
            let r = Render {
                sigma: 1.4,
                ..Render::px(3.2)
            };
            let p = r.profile(&code39_modules(&text, 2, 1));
            assert!(
                decode_all(&Code39Decoder, &p, &opts())
                    .iter()
                    .all(|h| !h.thin)
            );
        }
    }

    #[test]
    fn thin_symbols_cut_by_the_border_or_truncated_are_never_read() {
        let mut rng = Rng(13);
        let text = "1234567";
        let m = code39_modules(text, 2, 1);
        for sigma in [0.5, 0.7] {
            let r = Render {
                sigma,
                ..Render::px(1.5)
            };
            let p = r.profile(&m);
            for (depth, cut) in border_cuts(&p, 18.0, m.len() as f32 * 1.5) {
                assert!(
                    decode_all(&Code39Decoder, &cut, &opts()).is_empty(),
                    "depth {depth}"
                );
            }
            // Stop character blanked out.
            let mut broken = m.clone();
            let n = broken.len();
            broken[n - 12..].fill(false);
            let mut p = Render {
                sigma,
                ..Render::px(1.5)
            }
            .profile(&broken);
            add_noise(&mut p, 3.0, &mut rng);
            assert!(decode_all(&Code39Decoder, &p, &opts()).is_empty());
        }
    }

    #[test]
    fn thin_rules_never_read_noise_stripes_or_other_symbologies() {
        let mut rng = Rng(77);
        for _ in 0..3000 {
            let mut widths = Vec::new();
            while widths.iter().sum::<usize>() < 400 {
                widths.push(1 + rng.below(3) as usize);
            }
            let r = Render {
                sigma: 0.6,
                ..Render::px(1.5)
            };
            let mut p = r.profile(&runs_to_modules(&widths));
            add_noise(&mut p, 5.0, &mut rng);
            assert!(decode_all(&Code39Decoder, &p, &opts()).is_empty());
        }
        let others: [Vec<bool>; 4] = [
            ean13_modules("5901234123457"),
            itf_modules("1234567890", 2),
            code128_modules(&[104, 33, 34, 35, 36, 37, 38]),
            codabar_modules('A', "1234567", 'B', 2, 1),
        ];
        for modules in &others {
            for px in [1.3, 1.5, 1.8] {
                for sigma in [0.5, 0.8] {
                    let r = Render {
                        sigma,
                        ..Render::px(px)
                    };
                    let p = r.profile(modules);
                    assert!(decode_all(&Code39Decoder, &p, &opts()).is_empty());
                }
            }
        }
    }
}
