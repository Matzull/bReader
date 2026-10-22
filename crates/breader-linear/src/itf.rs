//! Interleaved 2 of 5, following ISO/IEC 16390.
//!
//! Any pair of digits is a valid element pattern, so the checks that keep
//! this decoder from reading noise are structural: the start and stop
//! patterns, wide-to-narrow ratios, large quiet zones and the length list.
//! The reader additionally requires more agreeing scanlines than for other
//! symbologies.

use breader_core::Symbology;

use crate::common::{
    MIN_MODULE, adjusted_width, adjusted_widths, quiet_after, quiet_before, split_wide,
};
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::ean::check_digit_ok;
use crate::runs::Runs;

/// The two wide elements among the five of each digit.
const WIDE: [(u8, u8); 10] = [
    (2, 3),
    (0, 4),
    (1, 4),
    (0, 1),
    (2, 4),
    (0, 2),
    (1, 2),
    (3, 4),
    (0, 3),
    (1, 3),
];

const INVALID: u8 = 0xff;

static DIGIT_BY_MASK: [u8; 32] = {
    let mut t = [INVALID; 32];
    let mut d = 0;
    while d < 10 {
        t[((1u16 << WIDE[d].0) | (1 << WIDE[d].1)) as usize] = d as u8;
        d += 1;
    }
    t
};

/// Minimum quiet zone on each side, in narrow element widths.
const QUIET: f32 = 10.0;
/// Accepted ratio between the mean wide and the mean narrow element of
/// a digit.
const RATIO: (f32, f32) = (1.6, 3.4);
/// The smallest wide element must exceed the largest narrow one by this factor.
const MIN_SEPARATION: f32 = 1.3;
/// Allowed ratio between the narrow widths of neighboring digit groups.
const MODULE_DRIFT: f32 = 1.7;
/// Allowed range of start-pattern elements around their mean.
const START_SPREAD: (f32, f32) = (0.6, 1.6);
const MAX_DIGITS: usize = 128;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ItfDecoder;

struct Group {
    digits: (u8, u8),
    narrow: f32,
    separation: f32,
}

/// Reads five bars and five spaces starting at run `at` as two digits.
fn read_pair(runs: &Runs, at: usize, bias: f32, prev_narrow: f32) -> Option<Group> {
    let w = adjusted_widths::<10>(runs, at, bias)?;
    let mut bars = [0.0; 5];
    let mut spaces = [0.0; 5];
    for i in 0..5 {
        bars[i] = w[2 * i];
        spaces[i] = w[2 * i + 1];
    }
    let (b, s) = (split_wide(&bars, 2)?, split_wide(&spaces, 2)?);
    let narrow = (b.narrow + s.narrow) / 2.0;
    if narrow < MIN_MODULE
        || narrow < prev_narrow / MODULE_DRIFT
        || narrow > prev_narrow * MODULE_DRIFT
    {
        return None;
    }
    for part in [b, s] {
        if part.separation < MIN_SEPARATION
            || !(RATIO.0..=RATIO.1).contains(&(part.wide / part.narrow))
        {
            return None;
        }
    }
    let first = DIGIT_BY_MASK[usize::from(b.wide_mask) & 31];
    let second = DIGIT_BY_MASK[usize::from(s.wide_mask) & 31];
    (first != INVALID && second != INVALID).then_some(Group {
        digits: (first, second),
        narrow,
        separation: b.separation.min(s.separation),
    })
}

/// Wide bar, narrow space, narrow bar at run `at`, followed by a quiet zone.
fn stop_at(runs: &Runs, at: usize, bias: f32, narrow: f32) -> bool {
    let (Some(bar), Some(space), Some(last)) = (
        adjusted_width(runs, at, bias),
        adjusted_width(runs, at + 1, bias),
        adjusted_width(runs, at + 2, bias),
    ) else {
        return false;
    };
    let ratio = bar / narrow;
    (1.6..=3.8).contains(&ratio)
        && [space, last]
            .iter()
            .all(|&w| (START_SPREAD.0..=START_SPREAD.1).contains(&(w / narrow)))
        && quiet_after(runs, at + 2, bias) >= QUIET * narrow
}

struct Read {
    digits: [u8; MAX_DIGITS],
    len: usize,
    separation_sum: f32,
    narrow_sum: f32,
    groups: usize,
    last_bar: usize,
}

/// Checks the start pattern at run `j`; returns the bar/space bias and the
/// module estimate.
///
/// The quiet zone is measured as is, also where it ends at the profile
/// border (`quiet_before`, not `quiet_before_open`). The other symbologies
/// treat such a margin as open, because a symbol cut by the border fails on
/// its start, stop or check character. Interleaved 2 of 5 has none of them
/// in the data: the start pattern also occurs inside a longer symbol, and
/// the cut symbol, with its stop pattern, is a valid shorter code.
fn start_of(runs: &Runs, j: usize) -> Option<(f32, f32)> {
    let raw = adjusted_widths::<4>(runs, j, 0.0)?;
    let mean = raw.iter().sum::<f32>() / 4.0;
    if mean < MIN_MODULE || runs.space_before(j) < 0.9 * QUIET * mean * START_SPREAD.0 {
        return None;
    }
    let bars = (raw[0] + raw[2]) / 2.0;
    let spaces = (raw[1] + raw[3]) / 2.0;
    let bias = (bars - spaces) / 2.0;
    let module = (bars + spaces) / 2.0;
    if bias.abs() > 0.45 * module
        || !raw
            .iter()
            .all(|&w| (START_SPREAD.0..=START_SPREAD.1).contains(&(w / mean)))
    {
        return None;
    }
    if quiet_before(runs, j, bias) < QUIET * module {
        return None;
    }
    Some((bias, module))
}

fn read_symbol(runs: &Runs, j: usize, started: &mut bool) -> Option<Read> {
    let (bias, module) = start_of(runs, j)?;
    let mut read = Read {
        digits: [0; MAX_DIGITS],
        len: 0,
        separation_sum: 0.0,
        narrow_sum: module,
        groups: 0,
        last_bar: 0,
    };
    let mut narrow = module;
    let mut at = j + 4;
    loop {
        if stop_at(runs, at, bias, narrow) {
            read.last_bar = at + 2;
            return Some(read);
        }
        if read.len + 2 > MAX_DIGITS {
            return None;
        }
        let g = read_pair(runs, at, bias, narrow)?;
        *started = true;
        narrow = g.narrow;
        read.digits[read.len] = g.digits.0;
        read.digits[read.len + 1] = g.digits.1;
        read.len += 2;
        read.separation_sum += g.separation;
        read.narrow_sum += g.narrow;
        read.groups += 1;
        if read.groups == 3 {
            runs.note_start(0.0);
        }
        at += 10;
    }
}

impl LinearDecoder for ItfDecoder {
    fn decode(&self, runs: &Runs, opts: &LineOptions, out: &mut Vec<LineHit>) -> bool {
        if !opts.symbologies.contains(Symbology::Itf) {
            return false;
        }
        let mut started = false;
        let n = runs.len();
        let mut j = usize::from(n > 0 && !runs.is_bar(0));
        while j + 4 + 10 + 3 <= n {
            if let Some(read) = read_symbol(runs, j, &mut started) {
                let digits = &read.digits[..read.len];
                let length_ok =
                    digits.len() >= opts.itf_min_length && opts.itf_length_ok(digits.len());
                let checked = digits.len() == 14;
                if length_ok && (!checked || check_digit_ok(digits)) {
                    let groups = read.groups.max(1) as f32;
                    out.push(LineHit {
                        symbology: Symbology::Itf,
                        text: digits.iter().map(|d| b'0' + d).collect(),
                        addon: None,
                        start: runs.start(j),
                        end: runs.end(read.last_bar),
                        module: read.narrow_sum / (groups + 1.0),
                        quality: ((read.separation_sum / groups - 1.0) / 2.0).clamp(0.0, 1.0),
                        aim: if checked { *b"]I1" } else { *b"]I0" },
                        checksum_verified: checked,
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

    const ITF14: &str = "0001234567890";

    fn opts() -> LineOptions {
        line_options(|_| {})
    }

    fn any_length() -> LineOptions {
        line_options(|o| o.itf.allowed_lengths.clear())
    }

    fn texts(hits: &[LineHit]) -> Vec<Vec<u8>> {
        hits.iter().map(|h| h.text.clone()).collect()
    }

    fn prof(digits: &str, wide: usize, px: f32) -> Vec<u8> {
        Render::px(px).profile(&itf_modules(digits, wide))
    }

    #[test]
    fn digit_table_has_every_two_of_five_combination_once() {
        assert_eq!(DIGIT_BY_MASK.iter().filter(|&&d| d != INVALID).count(), 10);
        for (d, p) in [
            "nnwwn", "wnnnw", "nwnnw", "wwnnn", "nnwnw", "wnwnn", "nwwnn", "nnnww", "wnnwn",
            "nwnwn",
        ]
        .iter()
        .enumerate()
        {
            let mask = p
                .bytes()
                .enumerate()
                .fold(0usize, |m, (i, b)| m | usize::from(b == b'w') << i);
            assert_eq!(usize::from(DIGIT_BY_MASK[mask]), d);
        }
    }

    #[test]
    fn itf14_reads_in_both_directions_and_polarities() {
        let text = with_gs1_check(ITF14);
        assert_eq!(text.len(), 14);
        for wide in [2, 3] {
            for px in [1.5, 2.0, 3.0] {
                if wide == 2 && px < 2.0 {
                    continue;
                }
                let p = prof(&text, wide, px);
                let hits = decode_all(&ItfDecoder, &p, &opts());
                assert_eq!(
                    texts(&hits),
                    [text.as_bytes().to_vec()],
                    "wide {wide} px {px}"
                );
                assert_eq!(&hits[0].aim, b"]I1");
                assert!(hits[0].checksum_verified);
                let mut rev = p.clone();
                rev.reverse();
                assert_eq!(decode_all(&ItfDecoder, &rev, &opts()).len(), 1, "reversed");
                let r = Render {
                    dark: 225.0,
                    light: 25.0,
                    ..Render::px(px)
                };
                let hits = decode_all(&ItfDecoder, &r.profile(&itf_modules(&text, wide)), &opts());
                assert_eq!(hits.len(), 1);
                assert!(hits[0].inverted);
            }
        }
    }

    #[test]
    fn length_rules() {
        let t12 = "123456789012";
        let t14 = with_gs1_check("1234567890123");
        for n in [8, 10, 12, 14, 16, 18] {
            let t = if n == 14 {
                t14.clone()
            } else {
                "12345678901234567890"[..n].to_string()
            };
            assert_eq!(
                decode_all(&ItfDecoder, &prof(&t, 3, 3.0), &opts()).len(),
                1,
                "default list, {n} digits"
            );
        }
        for n in [6, 20] {
            let t = &"12345678901234567890"[..n];
            assert!(decode_all(&ItfDecoder, &prof(t, 3, 3.0), &opts()).is_empty());
        }
        let any = decode_all(&ItfDecoder, &prof(t12, 3, 3.0), &any_length());
        assert_eq!(texts(&any), [t12.as_bytes().to_vec()]);
        assert_eq!(&any[0].aim, b"]I0");
        assert!(!any[0].checksum_verified);
        let list = line_options(|o| o.itf.allowed_lengths = vec![12, 14]);
        assert_eq!(decode_all(&ItfDecoder, &prof(t12, 3, 3.0), &list).len(), 1);
        assert_eq!(decode_all(&ItfDecoder, &prof(&t14, 3, 3.0), &list).len(), 1);
        let min = line_options(|o| {
            o.itf.allowed_lengths.clear();
            o.itf.min_length = 8;
        });
        assert!(decode_all(&ItfDecoder, &prof("123456", 3, 3.0), &min).is_empty());
        assert_eq!(
            decode_all(&ItfDecoder, &prof("12345678", 3, 3.0), &min).len(),
            1
        );
    }

    #[test]
    fn itf14_needs_its_check_digit_even_in_the_any_length_mode() {
        let good = with_gs1_check("1234567890123");
        for o in [opts(), any_length()] {
            assert_eq!(decode_all(&ItfDecoder, &prof(&good, 3, 3.0), &o).len(), 1);
            for wrong in (0..10).filter(|&d| d != good.as_bytes()[13] - b'0') {
                let bad = format!("{}{wrong}", &good[..13]);
                assert!(
                    decode_all(&ItfDecoder, &prof(&bad, 3, 3.0), &o).is_empty(),
                    "{bad}"
                );
            }
        }
    }

    #[test]
    fn quiet_zone_of_ten_narrow_widths_is_required() {
        let text = with_gs1_check("1234567890123");
        let modules = itf_modules(&text, 3);
        let at = |left: f32, right: f32| {
            let r = Render {
                quiet_left: left,
                quiet_right: right,
                ..Render::px(3.0)
            };
            decode_all(&ItfDecoder, &r.profile(&modules), &opts()).len()
        };
        assert_eq!(at(11.0, 11.0), 1);
        assert_eq!(at(8.0, 14.0), 0);
        assert_eq!(at(14.0, 8.0), 0);
    }

    /// Unlike the other symbologies, ITF keeps its full quiet zone when the
    /// margin ends at the profile border: a start pattern inside a longer
    /// symbol, or a symbol cut short, reads as a shorter valid code, and only
    /// the quiet zone tells these apart.
    #[test]
    fn a_margin_ending_at_the_border_does_not_open_the_quiet_zone() {
        let text = with_gs1_check("1234567890123");
        let modules = itf_modules(&text, 3);
        for (left, right) in [(1.0, 14.0), (14.0, 1.0), (4.0, 4.0), (9.0, 9.0)] {
            let r = Render {
                quiet_left: left,
                quiet_right: right,
                ..Render::px(3.0)
            };
            let p = r.profile(&modules);
            assert!(
                decode_all(&ItfDecoder, &p, &opts()).is_empty(),
                "{left} {right}"
            );
        }
    }

    #[test]
    fn a_symbol_cut_by_the_border_is_not_read_shorter() {
        let digits = "12345678901234567890";
        let modules = itf_modules(digits, 3);
        let r = Render {
            quiet_left: 11.0,
            quiet_right: 11.0,
            ..Render::px(3.0)
        };
        let p = r.profile(&modules);
        let any = any_length();
        assert_eq!(decode_all(&ItfDecoder, &p, &any).len(), 1);
        for (depth, cut) in border_cuts(&p, 33.0, modules.len() as f32 * 3.0) {
            assert!(
                decode_all(&ItfDecoder, &cut, &any).is_empty(),
                "depth {depth}"
            );
        }
    }

    #[test]
    fn a_partial_read_of_a_longer_symbol_is_never_returned() {
        // Whatever the length list, only the whole 14-digit symbol reads:
        // inner start-like patterns have no quiet zone in front of them.
        let t14 = with_gs1_check("1234567890123");
        for o in [
            opts(),
            any_length(),
            line_options(|o| o.itf.allowed_lengths = vec![12, 14]),
        ] {
            let hits = decode_all(&ItfDecoder, &prof(&t14, 3, 3.0), &o);
            assert_eq!(texts(&hits), [t14.as_bytes().to_vec()]);
        }
        // A symbol with an end cut off is no symbol.
        let m = itf_modules(&t14, 3);
        let mut cut = m[..m.len() - 40].to_vec();
        cut.extend(std::iter::repeat_n(false, 40));
        assert!(decode_all(&ItfDecoder, &Render::px(3.0).profile(&cut), &any_length()).is_empty());
        let tail = &m[40..];
        assert!(decode_all(&ItfDecoder, &Render::px(3.0).profile(tail), &any_length()).is_empty());
    }

    #[test]
    fn ratio_outside_the_accepted_range_is_rejected() {
        let t = with_gs1_check("1234567890123");
        assert!(decode_all(&ItfDecoder, &prof(&t, 4, 2.0), &opts()).is_empty());
        let flat = Render::px(2.0).profile(&runs_to_modules(&[1; 90]));
        assert!(decode_all(&ItfDecoder, &flat, &any_length()).is_empty());
    }

    #[test]
    fn perturbed_widths_are_read_correctly_or_not_at_all() {
        let mut rng = Rng(21);
        let text = with_gs1_check("4006381333931");
        let modules = itf_modules(&text, 2);
        let (mut right, mut none) = (0, 0);
        for i in 0..400 {
            let amount = 0.1 + (i % 8) as f32 * 0.1;
            let mut dark = Vec::new();
            let mut x = 60.0f32;
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
            let p = render_intervals(&dark, x as usize + 80, 25.0, 225.0, 0.6);
            for o in [opts(), any_length()] {
                for h in decode_all(&ItfDecoder, &p, &o) {
                    assert_eq!(h.text, text.as_bytes(), "misread at {amount}");
                }
            }
            if decode_all(&ItfDecoder, &p, &opts()).is_empty() {
                none += 1
            } else {
                right += 1
            }
        }
        assert!(right > 100 && none > 0, "{right} {none}");
    }
}
