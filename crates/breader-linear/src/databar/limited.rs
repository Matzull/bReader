//! GS1 DataBar Limited.
//!
//! 46 elements, 74 modules: a left guard (space, bar), two data characters
//! of 14 elements and 26 modules around a 14-element check pattern of 18
//! modules, and a right guard. The 13-digit body (first digit 0 or 1) is
//! split at `2013571` into the two character values. The checksum is
//! `sum(3^i * e_i)` mod 89 over the 28 data elements left to right; it
//! selects one of 89 check patterns.

use breader_core::Symbology;

use super::quantize::{Segment, quantize};
use super::rank::rank;
use crate::common::MIN_MODULE;
use crate::decoder::LineHit;

use super::Row;

/// Modules between the first and the last bar edge.
const SPAN_MODULES: f32 = 73.0;
const TOLERANCE: f32 = 0.5;
const PIECE_SLACK: f32 = 0.25;
/// Light areas required before the left and after the right guard, in modules.
const QUIET_BEFORE: f32 = 0.8;
const QUIET_AFTER: f32 = 4.0;

const SEGMENTS: [Segment; 6] = [
    Segment::new(0, 1, 1),
    Segment::new(1, 14, 26),
    Segment::new(15, 14, 18),
    Segment::new(29, 14, 26),
    Segment::new(43, 1, 1),
    Segment::new(44, 1, 1),
];

/// Per group: odd element sum, widest odd element, first value and the
/// number of even combinations. The widest even element is `9 - odd widest`;
/// even combinations always contain a width-1 element.
const ODD_SUM: [u8; 7] = [17, 13, 9, 15, 11, 19, 7];
const ODD_MAX: [u8; 7] = [6, 5, 3, 5, 4, 8, 1];
const BASE: [u32; 7] = [
    0, 183_064, 820_064, 1_000_776, 1_491_021, 1_979_845, 1_996_939,
];
const EVEN_COUNT: [u32; 7] = [28, 728, 6454, 203, 2408, 1, 16_632];
const HALF: u64 = 2_013_571;
const BODY_LIMIT: u64 = 2_000_000_000_000;

/// Check patterns by checksum, element widths from the first space.
const CHECK_PATTERNS: [&str; 89] = [
    "11111111113311",
    "11111111123211",
    "11111111133111",
    "11111112113211",
    "11111112123111",
    "11111113113111",
    "11111211113211",
    "11111211123111",
    "11111212113111",
    "11111311113111",
    "11121111113211",
    "11121111123111",
    "11121112113111",
    "11121211113111",
    "11131111113111",
    "12111111113211",
    "12111111123111",
    "12111112113111",
    "12111211113111",
    "12121111113111",
    "13111111113111",
    "11111111212311",
    "11111111222211",
    "11111111232111",
    "11111112212211",
    "11111112222111",
    "11111113212111",
    "11111211212211",
    "11111211222111",
    "11111212212111",
    "11111311212111",
    "11121111212211",
    "11121111222111",
    "11121112212111",
    "11121211212111",
    "11131111212111",
    "12111111212211",
    "12111111222111",
    "12111112212111",
    "12111211212111",
    "12121111212111",
    "13111111212111",
    "11111111311311",
    "11111111321211",
    "11111112311211",
    "11121111311211",
    "12111111311211",
    "11111121112311",
    "11111121122211",
    "11111121132111",
    "11111122112211",
    "11121121112211",
    "11121121122111",
    "11121122112111",
    "11121221112111",
    "11131121112111",
    "12111121112211",
    "12111121122111",
    "12121121112111",
    "11112111112311",
    "11112111122211",
    "11112111132111",
    "11112112112211",
    "11112112122111",
    "11112211112211",
    "12112111112211",
    "12112111122111",
    "12112112112111",
    "12112211112111",
    "12122111112111",
    "13112111112111",
    "11211111112311",
    "11211111122211",
    "11211111132111",
    "11211112112211",
    "11211112122111",
    "11211113112111",
    "11211211112211",
    "11211211122111",
    "11221111112211",
    "21111111122211",
    "21111111132111",
    "21111112112211",
    "21111112122111",
    "21111113112111",
    "21111211122111",
    "21111212112111",
    "21121111122111",
    "21111111221211",
];

/// Value of one half, its 14 elements left to right.
pub(super) fn half_value(c: &[u8; 14]) -> Option<u32> {
    let mut odd = [0u8; 7];
    let mut even = [0u8; 7];
    for i in 0..7 {
        odd[i] = c[2 * i];
        even[i] = c[2 * i + 1];
    }
    let sum: u8 = odd.iter().sum();
    let g = ODD_SUM.iter().position(|&s| s == sum)?;
    let o = rank(&odd, ODD_MAX[g], false)?;
    let e = rank(&even, 9 - ODD_MAX[g], true)?;
    BASE[g]
        .checked_add(o.checked_mul(EVEN_COUNT[g])?)?
        .checked_add(e)
}

/// Checksum over the 28 data elements, mod 89.
pub(super) fn checksum(left: &[u8; 14], right: &[u8; 14]) -> usize {
    let mut w = 1u32;
    let mut sum = 0u32;
    for &e in left.iter().chain(right) {
        sum += w * u32::from(e);
        w = w * 3 % 89;
    }
    (sum % 89) as usize
}

pub(super) fn check_pattern(checksum: usize) -> &'static [u8] {
    CHECK_PATTERNS[checksum].as_bytes()
}

/// Reads a Limited symbol.
pub(super) fn decode(row: &Row, out: &mut Vec<LineHit>) -> bool {
    let module = row.span / SPAN_MODULES;
    if module < MIN_MODULE {
        return false;
    }
    if row.quiet_before < QUIET_BEFORE * module || row.quiet_after < QUIET_AFTER * module {
        return false;
    }
    let w = &row.w;
    for s in &SEGMENTS[1..4] {
        let sum: f32 = w[s.at..s.at + s.len].iter().sum();
        let expected = f32::from(s.modules) * module;
        if (sum - expected).abs() > PIECE_SLACK * expected {
            return false;
        }
    }
    let mut n = [0u8; super::ROW_RUNS];
    let Some(q) = quantize(w, true, &SEGMENTS, module, TOLERANCE, &mut n) else {
        return false;
    };
    let left: [u8; 14] = super::array(&n[1..15]);
    let right: [u8; 14] = super::array(&n[29..43]);
    let expected = check_pattern(checksum(&left, &right));
    if n[15..29].iter().zip(expected).any(|(&a, &b)| a != b - b'0') {
        return true;
    }
    let (Some(l), Some(r)) = (half_value(&left), half_value(&right)) else {
        return true;
    };
    let body = u64::from(l) * HALF + u64::from(r);
    if u64::from(l) >= HALF || u64::from(r) >= HALF || body >= BODY_LIMIT {
        return true;
    }
    out.push(LineHit {
        symbology: Symbology::DataBarLimited,
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
        thin: false,
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
            .filter(|h| h.symbology == Symbology::DataBarLimited)
            .map(|h| h.text)
            .collect()
    }

    fn body(rng: &mut Rng) -> String {
        loop {
            let v = rng.next_u64() % BODY_LIMIT;
            if v / HALF < HALF {
                return format!("{v:013}");
            }
        }
    }

    #[test]
    fn group_tables_match_the_combination_counts() {
        let mut next = 0u32;
        for g in 0..7 {
            assert_eq!(BASE[g], next, "group {g}");
            let odd = count(7, i32::from(ODD_SUM[g]), i32::from(ODD_MAX[g]), false);
            let even_sum = 26 - i32::from(ODD_SUM[g]);
            let even = count(7, even_sum, 9 - i32::from(ODD_MAX[g]), true);
            assert_eq!(EVEN_COUNT[g], even, "group {g}");
            next += odd * even;
        }
        assert_eq!(u64::from(next), HALF);
    }

    #[test]
    fn check_patterns_are_distinct_and_have_eighteen_modules() {
        let mut seen: Vec<&[u8]> = Vec::new();
        for p in CHECK_PATTERNS {
            let w = p.as_bytes();
            assert_eq!(w.len(), 14);
            assert_eq!(w.iter().map(|&b| u32::from(b - b'0')).sum::<u32>(), 18);
            assert!(!seen.contains(&w));
            seen.push(w);
        }
        assert_eq!(seen.len(), 89);
    }

    #[test]
    fn reference_vectors_of_the_symbology_are_read() {
        // Module patterns of GTIN 0, as printed by an independent encoder
        // and found in a photographed sample.
        let zero = databar_limited_modules("0000000000000");
        assert_eq!(zero.len(), 79);
        assert_eq!(read(&zero, Render::px(3.0)), vec![element("0000000000000")]);
    }

    #[test]
    fn reads_boundaries_and_random_bodies() {
        let mut bodies: Vec<String> = [
            "0000000000000",
            "0000000000001",
            "0000002013570",
            "0000002013571",
            "1000000000000",
            "1234567890123",
            "1999999999999",
            "0015133531096",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let mut rng = Rng(0x11a17ed);
        for _ in 0..400 {
            bodies.push(body(&mut rng));
        }
        for (i, b) in bodies.iter().enumerate() {
            let px = 1.8 + rng.unit() * 3.0;
            let modules = databar_limited_modules(b);
            let render = Render {
                gain: rng.unit() * 0.25,
                sigma: 0.5 + rng.unit() * 0.3,
                ..Render::px(px)
            };
            let got = read(&modules, render);
            assert!(got.contains(&element(b)), "{i} {b} at {px}: {got:?}");
        }
    }

    #[test]
    fn a_damaged_symbol_is_never_misread() {
        let mut rng = Rng(0x7e57);
        for _ in 0..40 {
            let b = body(&mut rng);
            let good = element(&b);
            let modules = databar_limited_modules(&b);
            for at in 0..74 {
                let mut m = modules.clone();
                m[at] = !m[at];
                for text in read(&m, Render::px(3.0)) {
                    assert_eq!(text, good, "{b} flipped {at}");
                }
            }
        }
    }

    #[test]
    fn needs_its_quiet_zone_and_reads_inverted() {
        let m = databar_limited_modules("1234567890123");
        let good = element("1234567890123");
        // Content right behind the right guard.
        let crowded = with_neighbours(&m[..74], 0, 2);
        assert!(read(&crowded, Render::px(3.0)).is_empty());
        let spaced = with_neighbours(&m[..74], 0, 6);
        assert_eq!(read(&spaced, Render::px(3.0)), vec![good.clone()]);
        let inverted: Vec<bool> = m.iter().map(|b| !b).collect();
        let hits = decode_all(
            &DataBarDecoder,
            &Render::px(3.0).profile(&inverted),
            &opts(),
        );
        assert!(hits.iter().any(|h| h.inverted && h.text == good));
    }

    #[test]
    fn noise_reads_nothing() {
        let mut rng = Rng(0xbad_ca7);
        for _ in 0..4000 {
            let n = 46 + rng.below(30) as usize;
            let widths: Vec<f32> = (0..n)
                .map(|_| (1 + rng.below(4)) as f32 * (0.8 + 0.4 * rng.unit()))
                .collect();
            let runs = crate::runs::Runs::from_widths(true, &widths, 20.0, 20.0);
            let mut out = Vec::new();
            DataBarDecoder.decode(&runs, &opts(), &mut out);
            assert!(out.is_empty(), "{widths:?}");
        }
    }
}
