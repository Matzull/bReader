//! GS1 DataBar Omnidirectional and Truncated (one row).
//!
//! 46 elements, 96 modules: a left guard (space, bar), four data characters
//! of 8 elements, two finder patterns of 5 elements and a right guard. The
//! first and last data characters of a half have 16 modules ("outside"), the
//! others 15 ("inside"). Printed left to right the pieces are
//! `1 f 2 4 f 3`, with the elements of characters 2 and 3 numbered from the
//! right. The GTIN body (13 digits) is split in `4537077` and `1597`
//! blocks into the four character values; the checksum over all element
//! widths (mod 79) selects the pair of finder patterns.

use breader_core::Symbology;

use super::quantize::{Segment, quantize};
use super::rank::rank;
use crate::common::MIN_MODULE;
use crate::decoder::LineHit;

use super::Row;

/// Modules between the first and the last bar edge.
const SPAN_MODULES: f32 = 95.0;
/// Largest distance of a scaled element to a whole number of modules.
const TOLERANCE: f32 = 0.5;
/// Allowed deviation of a piece from its module count before quantizing.
const PIECE_SLACK: f32 = 0.25;
/// Light area required outside the guards, in modules.
const QUIET: f32 = 0.8;

const SEGMENTS: [Segment; 9] = [
    Segment::new(0, 1, 1),
    Segment::new(1, 8, 16),
    Segment::new(9, 5, 15),
    Segment::new(14, 8, 15),
    Segment::new(22, 8, 15),
    Segment::new(30, 5, 15),
    Segment::new(35, 8, 16),
    Segment::new(43, 1, 1),
    Segment::new(44, 1, 1),
];

/// Widths of the nine finder patterns, from the left finder's point of view.
pub(super) const FINDERS: [[u8; 5]; 9] = [
    [3, 8, 2, 1, 1],
    [3, 5, 5, 1, 1],
    [3, 3, 7, 1, 1],
    [3, 1, 9, 1, 1],
    [2, 7, 4, 1, 1],
    [2, 5, 6, 1, 1],
    [2, 3, 8, 1, 1],
    [1, 5, 7, 1, 1],
    [1, 3, 9, 1, 1],
];

/// Outside characters by group: odd and even element sums, widest odd
/// element, first value, and the number of even combinations.
const OUT_ODD_SUM: [u8; 5] = [12, 10, 8, 6, 4];
const OUT_ODD_MAX: [u8; 5] = [8, 6, 4, 3, 1];
const OUT_BASE: [u16; 5] = [0, 161, 961, 2015, 2715];
const OUT_EVEN_COUNT: [u16; 5] = [1, 10, 34, 70, 126];
/// Inside characters: the odd combinations in use (a prefix of the full set
/// for the last two groups).
const IN_ODD_SUM: [u8; 4] = [5, 7, 9, 11];
const IN_ODD_MAX: [u8; 4] = [2, 4, 6, 8];
const IN_BASE: [u16; 4] = [0, 336, 1036, 1516];
const IN_ODD_COUNT: [u16; 4] = [4, 20, 48, 81];

/// Values per half: `2841 * 1597` half pairs of the GTIN body.
const PAIR: u64 = 4_537_077;
const INSIDE: u64 = 1597;
const BODY_LIMIT: u64 = 10_000_000_000_000;

/// Value of one data character, its elements in the numbering of the
/// specification (odd elements first, third, ...).
pub(super) fn char_value(c: &[u8; 8], outside: bool) -> Option<u16> {
    let odd = [c[0], c[2], c[4], c[6]];
    let even = [c[1], c[3], c[5], c[7]];
    let sum: u8 = odd.iter().sum();
    if outside {
        let g = OUT_ODD_SUM.iter().position(|&s| s == sum)?;
        let o = rank(&odd, OUT_ODD_MAX[g], false)?;
        let e = rank(&even, 9 - OUT_ODD_MAX[g], true)?;
        u16::try_from(o)
            .ok()?
            .checked_mul(OUT_EVEN_COUNT[g])?
            .checked_add(u16::try_from(e).ok()?)?
            .checked_add(OUT_BASE[g])
    } else {
        let g = IN_ODD_SUM.iter().position(|&s| s == sum)?;
        let o = rank(&odd, IN_ODD_MAX[g], true)?;
        if o >= u32::from(IN_ODD_COUNT[g]) {
            return None;
        }
        let e = rank(&even, 9 - IN_ODD_MAX[g], false)?;
        u16::try_from(e)
            .ok()?
            .checked_mul(IN_ODD_COUNT[g])?
            .checked_add(u16::try_from(o).ok()?)?
            .checked_add(IN_BASE[g])
    }
}

/// Checksum over the four characters in specification order, mod 79: the
/// weight of element `i` of character `k` is `3^i * 4^k`.
pub(super) fn checksum(chars: &[[u8; 8]; 4]) -> u32 {
    let mut sum = 0u32;
    let mut char_weight = 1u32;
    for c in chars {
        let mut w = char_weight;
        for &e in c {
            sum += w * u32::from(e);
            w = w * 3 % 79;
        }
        char_weight = char_weight * 4 % 79;
    }
    sum % 79
}

/// Index of the finder pair `9 * left + right` that checksum `cs` selects.
/// Two pairs of the 81 are unused; for checksum 71 either neighbour of the
/// gap is accepted, since the specification text was not available.
fn pair_matches(cs: u32, pair: u32) -> bool {
    let idx = cs + u32::from(cs >= 8) + u32::from(cs >= 71);
    pair == idx || (cs == 71 && pair == 72)
}

fn finder_of(widths: &[u8; 5]) -> Option<u32> {
    FINDERS.iter().position(|f| f == widths).map(|p| p as u32)
}

/// Reads an Omnidirectional symbol.
/// Returns true when the shape looked like one, whether or not it decoded.
pub(super) fn decode(row: &Row, out: &mut Vec<LineHit>) -> bool {
    let module = row.span / SPAN_MODULES;
    if module < MIN_MODULE {
        return false;
    }
    if row.quiet_before < QUIET * module || row.quiet_after < QUIET * module {
        return false;
    }
    let w = &row.w;
    for s in &SEGMENTS[1..7] {
        let sum: f32 = w[s.at..s.at + s.len].iter().sum();
        let expected = f32::from(s.modules) * module;
        if (sum - expected).abs() > PIECE_SLACK * expected {
            return false;
        }
    }
    let mut n = [0u8; super::ROW_RUNS];
    let Some(q) = quantize(w, true, &SEGMENTS, module, TOLERANCE, &mut n) else {
        return true;
    };
    let (Some(left), Some(right)) = (finder_of(&super::array(&n[9..14])), finder_of(&super::reversed(&n[30..35])))
    else {
        return true;
    };
    let chars: [[u8; 8]; 4] = [
        super::array(&n[1..9]),
        super::reversed(&n[14..22]),
        super::reversed(&n[35..43]),
        super::array(&n[22..30]),
    ];
    if !pair_matches(checksum(&chars), 9 * left + right) {
        return true;
    }
    let (Some(d1), Some(d2), Some(d3), Some(d4)) = (
        char_value(&chars[0], true),
        char_value(&chars[1], false),
        char_value(&chars[2], true),
        char_value(&chars[3], false),
    ) else {
        return true;
    };
    let half = |outside: u16, inside: u16| u64::from(outside) * INSIDE + u64::from(inside);
    let body = half(d1, d2) * PAIR + half(d3, d4);
    if body >= BODY_LIMIT {
        return true;
    }
    out.push(LineHit {
        symbology: Symbology::DataBar,
        text: super::element_string(body),
        addon: None,
        start: row.start,
        end: row.end,
        module,
        quality: (1.0 - q.deviation / 0.5).clamp(0.0, 1.0),
        aim: *b"]e0",
        checksum_verified: true,
        short_quiet: false,
        inverted: false,
        start_stop: None,
        segments: 0,
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databar::DataBarDecoder;
    use crate::databar::rank::count;
    use crate::decoder::LinearDecoder;
    use crate::testutil::*;

    fn opts() -> crate::decoder::LineOptions {
        line_options(|_| {})
    }

    fn element(body: &str) -> Vec<u8> {
        format!("01{body}{}", check_digit(body)).into_bytes()
    }

    fn read(modules: &[bool], render: Render) -> Vec<Vec<u8>> {
        decode_all(&DataBarDecoder, &render.profile(modules), &opts())
            .into_iter()
            .filter(|h| h.symbology == Symbology::DataBar)
            .map(|h| h.text)
            .collect()
    }

    #[test]
    fn group_tables_match_the_combination_counts() {
        let mut next = 0;
        for g in 0..5 {
            assert_eq!(OUT_BASE[g], next);
            let odd = count(4, i32::from(OUT_ODD_SUM[g]), i32::from(OUT_ODD_MAX[g]), false);
            let even_sum = 16 - i32::from(OUT_ODD_SUM[g]);
            let even = count(4, even_sum, 9 - i32::from(OUT_ODD_MAX[g]), true);
            assert_eq!(u32::from(OUT_EVEN_COUNT[g]), even, "group {g}");
            next += (odd * even) as u16;
        }
        assert_eq!(next, 2841);
        let mut next = 0;
        for g in 0..4 {
            assert_eq!(IN_BASE[g], next);
            let full = count(4, i32::from(IN_ODD_SUM[g]), i32::from(IN_ODD_MAX[g]), true);
            assert!(u32::from(IN_ODD_COUNT[g]) <= full);
            let even_sum = 15 - i32::from(IN_ODD_SUM[g]);
            let even = count(4, even_sum, 9 - i32::from(IN_ODD_MAX[g]), false);
            next += IN_ODD_COUNT[g] * even as u16;
        }
        assert_eq!(next, 1597);
    }

    #[test]
    fn finder_patterns_have_fifteen_modules() {
        for f in FINDERS {
            assert_eq!(f.iter().map(|&w| u32::from(w)).sum::<u32>(), 15);
        }
        assert_eq!(pair_matches(0, 0), true);
        assert!(!pair_matches(7, 8) && pair_matches(8, 9));
        assert!(pair_matches(70, 71) && pair_matches(78, 80));
    }

    #[test]
    fn reads_boundaries_and_random_bodies() {
        let mut bodies: Vec<String> = [
            "0000000000000",
            "0000000000001",
            "2001234567890",
            "9999999999999",
            "0000004537076",
            "0000004537077",
            "0000001596000",
            "4412345678900",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut rng = Rng(0xda7a_ba12);
        for _ in 0..400 {
            bodies.push(rng.digits(13));
        }
        for (i, body) in bodies.iter().enumerate() {
            let px = 1.8 + rng.unit() * 3.0;
            let modules = databar_omni_modules(body);
            assert_eq!(modules.len(), 96);
            let render = Render {
                gain: rng.unit() * 0.25,
                sigma: 0.5 + rng.unit() * 0.3,
                ..Render::px(px)
            };
            let got = read(&modules, render);
            assert!(got.contains(&element(body)), "{i} {body} at {px}: {got:?}");
        }
    }

    #[test]
    fn reads_the_turned_symbol() {
        let m = databar_omni_modules("2001234567890");
        let turned: Vec<bool> = m.iter().rev().copied().collect();
        let got = read(&turned, Render::px(3.0));
        assert_eq!(got, vec![b"0120012345678909".to_vec()]);
    }

    #[test]
    fn reads_inverted_and_without_quiet_zone() {
        let m = databar_omni_modules("2001234567890");
        let inverted: Vec<bool> = m.iter().map(|b| !b).collect();
        let hits = decode_all(
            &DataBarDecoder,
            &Render::px(3.0).profile(&inverted),
            &opts(),
        );
        assert!(hits.iter().any(|h| h.inverted && h.text == b"0120012345678909"));
        // The symbol fills the profile: guard bars cut by both borders.
        let render = Render {
            quiet_left: 0.0,
            quiet_right: 0.0,
            ..Render::px(3.0)
        };
        let got = read(&m[1..], render);
        assert!(got.contains(&b"0120012345678909".to_vec()), "{got:?}");
    }

    #[test]
    fn a_damaged_symbol_is_never_misread() {
        let mut rng = Rng(0x5eed);
        for _ in 0..40 {
            let body = rng.digits(13);
            let good = element(&body);
            let modules = databar_omni_modules(&body);
            for at in 0..modules.len() {
                let mut m = modules.clone();
                m[at] = !m[at];
                for text in read(&m, Render::px(3.0)) {
                    assert_eq!(text, good, "{body} flipped {at}");
                }
            }
        }
    }

    #[test]
    fn noise_and_other_symbologies_read_nothing() {
        let mut rng = Rng(0xbad5eed);
        for _ in 0..4000 {
            let n = 46 + rng.below(30) as usize;
            let mut widths = Vec::new();
            for _ in 0..n {
                widths.push((1 + rng.below(4)) as f32 * (0.8 + 0.4 * rng.unit()));
            }
            let runs = crate::runs::Runs::from_widths(true, &widths, 20.0, 20.0);
            let mut out = Vec::new();
            DataBarDecoder.decode(&runs, &opts(), &mut out);
            assert!(out.is_empty(), "{widths:?}");
        }
        for m in [
            ean13_modules("5901234123457"),
            code39_modules("ABC-123", 3, 1),
            itf_modules("00012345678905", 3),
        ] {
            assert!(read(&m, Render::px(3.0)).is_empty());
        }
    }

    #[test]
    fn a_symbol_with_a_wrong_finder_pair_is_rejected() {
        // Swap the right finder of a valid symbol for another pattern of the
        // same width: only the pair of finders tells the checksum.
        let m = databar_omni_modules("2001234567890");
        let mut widths = Vec::new();
        let mut i = 1;
        while i < m.len() {
            let s = i;
            while i < m.len() && m[i] == m[s] {
                i += 1;
            }
            widths.push((i - s) as f32 * 3.0);
        }
        let mut alt = widths.clone();
        for (k, f) in FINDERS[7].iter().enumerate() {
            alt[30 + k] = f32::from(*f) * 3.0;
        }
        let sum_a: f32 = widths.iter().sum();
        let sum_b: f32 = alt.iter().sum();
        assert_eq!(sum_a, sum_b);
        let runs = crate::runs::Runs::from_widths(true, &alt, 20.0, 20.0);
        let mut out = Vec::new();
        DataBarDecoder.decode(&runs, &opts(), &mut out);
        assert!(out.is_empty(), "{out:?}");
        let runs = crate::runs::Runs::from_widths(true, &widths, 20.0, 20.0);
        DataBarDecoder.decode(&runs, &opts(), &mut out);
        assert_eq!(out.len(), 1);
    }
}
