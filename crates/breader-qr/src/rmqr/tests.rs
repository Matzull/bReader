//! Decoder tests: real symbols of the zxing-cpp samples, and round trips
//! against the `qrcode-generator` encoder (a black box with its own tables).

use super::decode::{decode_grid, read_format};
use super::samples::{R7X43_H, R17X139_M};
use super::spec::*;
use breader_core::{BitMatrix, EcLevel, Reject};
use qrcode_generator::rmqr::{Encoder, ErrorCorrection, Fnc1, Version};
use qrcode_generator::{Segment, Symbol};

const PI: &str = "3.1415926535897932384626433832795028841971693993751058209749445923078164062862089986280348253421170679821480865132823066470938446095505822317253594081";

pub(crate) fn grid_from(rows: &[&str]) -> BitMatrix {
    let mut g = BitMatrix::new(rows[0].len(), rows.len());
    for (y, r) in rows.iter().enumerate() {
        for (x, c) in r.bytes().enumerate() {
            g.set(x, y, c == b'#');
        }
    }
    g
}

pub(crate) fn version(i: usize) -> Version {
    use Version::*;
    [
        R7x43, R7x59, R7x77, R7x99, R7x139, R9x43, R9x59, R9x77, R9x99, R9x139, R11x27, R11x43,
        R11x59, R11x77, R11x99, R11x139, R13x27, R13x43, R13x59, R13x77, R13x99, R13x139, R15x43,
        R15x59, R15x77, R15x99, R15x139, R17x43, R17x59, R17x77, R17x99, R17x139,
    ][i]
}

pub(crate) fn matrix(sym: &Symbol) -> BitMatrix {
    let mut g = BitMatrix::new(sym.width(), sym.height());
    for y in 0..sym.height() {
        for x in 0..sym.width() {
            if sym.module(x, y).unwrap() {
                g.set(x, y, true);
            }
        }
    }
    g
}

pub(crate) fn encode(i: usize, high: bool, segments: &[Segment]) -> Option<BitMatrix> {
    let ec = if high {
        ErrorCorrection::High
    } else {
        ErrorCorrection::Medium
    };
    let sym = Encoder::new(ec)
        .boost_error_correction(false)
        .version(version(i))
        .encode_segments(segments)
        .ok()?;
    Some(matrix(&sym))
}

pub(crate) fn encode_fnc1(i: usize, fnc1: Fnc1, segments: &[Segment]) -> Option<BitMatrix> {
    let sym = Encoder::new(ErrorCorrection::Medium)
        .boost_error_correction(false)
        .version(version(i))
        .fnc1(Some(fnc1))
        .encode_segments(segments)
        .ok()?;
    Some(matrix(&sym))
}

fn digits(n: usize, salt: usize) -> String {
    (0..n)
        .map(|k| char::from(b'0' + ((k * 7 + salt * 3 + k / 3) % 10) as u8))
        .collect()
}

#[test]
fn real_symbols_of_the_zxing_samples() {
    let g = grid_from(R7X43_H);
    let d = decode_grid(&g).unwrap();
    assert_eq!(d.data, b",,");
    assert_eq!((d.version, d.width, d.height), (1, 43, 7));
    assert_eq!(d.ec_level, EcLevel::H);
    assert_eq!(&d.aim_id, b"]Q1");
    let g = grid_from(R17X139_M);
    let d = decode_grid(&g).unwrap();
    assert_eq!(d.data, PI.as_bytes());
    assert_eq!((d.version, d.width, d.height), (32, 139, 17));
    assert_eq!(d.ec_level, EcLevel::M);
}

#[test]
fn every_version_and_level_numeric_at_capacity() {
    for i in 0..VERSIONS {
        for high in [false, true] {
            let mut best = None;
            for n in (1..400).step_by(1) {
                let s = digits(n, i);
                match encode(i, high, &[Segment::numeric(&s).unwrap()]) {
                    Some(g) => best = Some((s, g)),
                    None => break,
                }
            }
            let (s, g) = best.unwrap();
            let d = decode_grid(&g).unwrap_or_else(|e| panic!("v{i} high={high}: {e:?}"));
            assert_eq!(d.data, s.as_bytes(), "v{i} high={high}");
            assert_eq!(usize::from(d.version), i + 1);
            assert_eq!(d.ec_level, if high { EcLevel::H } else { EcLevel::M });
            assert_eq!(d.errors_corrected, 0);
        }
    }
}

#[test]
fn every_mode_in_every_version() {
    let mut failures = Vec::new();
    for i in 0..VERSIONS {
        for high in [false, true] {
            let cases: [Vec<Segment>; 5] = [
                vec![Segment::alphanumeric("AB-12 $%*+./:Z").unwrap()],
                vec![Segment::bytes([0x00u8, 0xFF, 0x80, 0x41])],
                vec![Segment::kanji("日本語").unwrap()],
                vec![
                    Segment::numeric("12").unwrap(),
                    Segment::alphanumeric("A").unwrap(),
                    Segment::bytes(b"b"),
                ],
                vec![
                    Segment::eci(qrcode_generator::rmqr::EciAssignment::UTF_8),
                    Segment::bytes("é".as_bytes()),
                ],
            ];
            for segs in cases {
                let Some(g) = encode(i, high, &segs) else {
                    continue;
                };
                match decode_grid(&g) {
                    Ok(d) => {
                        assert!(!d.data.is_empty());
                        assert_eq!(d.eci.is_empty(), !format!("{segs:?}").contains("Eci"));
                    }
                    Err(e) => failures.push(format!("v{i} {:?} {e:?}", &format!("{segs:?}")[..20])),
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn fnc1_modes_and_identifiers() {
    let a = encode_fnc1(
        11,
        Fnc1::Gs1,
        &[Segment::numeric("0112345678901231").unwrap()],
    )
    .unwrap();
    let d = decode_grid(&a).unwrap();
    assert_eq!(d.data, b"0112345678901231");
    assert_eq!(&d.aim_id, b"]Q3");
    let ind = qrcode_generator::rmqr::ApplicationIndicator::numeric(37).unwrap();
    let b = encode_fnc1(
        11,
        Fnc1::Industry(ind),
        &[Segment::numeric("123456").unwrap()],
    )
    .unwrap();
    let d = decode_grid(&b).unwrap();
    assert_eq!(d.data, b"37123456");
    assert_eq!(&d.aim_id, b"]Q5");
}

#[test]
fn corrects_errors_up_to_the_guard() {
    let s = digits(40, 3);
    let g = encode(12, false, &[Segment::numeric(&s).unwrap()]).unwrap();
    let t = template(12);
    let ec = layout(12, false).ec;
    // One codeword per damaged module position, spread across the symbol.
    let max = crate::micro::decode::correctable(ec);
    assert!(max >= 2);
    let mut h = g.clone();
    for k in 0..max {
        let (x, y) = t.data[k * 40 + 3];
        h.set(
            usize::from(x),
            usize::from(y),
            !h.get(usize::from(x), usize::from(y)),
        );
    }
    let d = decode_grid(&h).unwrap();
    assert_eq!(d.data, s.as_bytes());
    assert!(d.errors_corrected > 0);
    // One more damaged codeword than the guard allows must not decode to
    // anything but the right payload.
    let mut h = g.clone();
    for k in 0..=max + 1 {
        let (x, y) = t.data[k * 40 + 3];
        h.set(
            usize::from(x),
            usize::from(y),
            !h.get(usize::from(x), usize::from(y)),
        );
    }
    if let Ok(d) = decode_grid(&h) {
        assert_eq!(d.data, s.as_bytes());
    }
}

#[test]
fn format_survives_a_few_flips_in_one_copy() {
    let g = encode(20, true, &[Segment::numeric("123456").unwrap()]).unwrap();
    for flips in 0..=3 {
        let mut h = g.clone();
        for &(x, y) in format_left().iter().take(flips) {
            h.set(x, y, !h.get(x, y));
        }
        assert_eq!(read_format(&h), Some((true, 20)), "{flips}");
    }
    // Four flips in the left copy leave the right one to carry the format.
    let mut h = g.clone();
    for &(x, y) in format_left().iter().take(4) {
        h.set(x, y, !h.get(x, y));
    }
    assert_eq!(read_format(&h), Some((true, 20)));
    // Seven flips in the left copy may land near another word; the clean
    // right copy still wins.
    let mut h = g.clone();
    for &(x, y) in format_left().iter().take(7) {
        h.set(x, y, !h.get(x, y));
    }
    assert_eq!(read_format(&h), Some((true, 20)));
    // Both copies damaged beyond distance two each: refused.
    let mut h = g.clone();
    for &(x, y) in format_left().iter().take(4) {
        h.set(x, y, !h.get(x, y));
    }
    let w = h.width();
    let ht = h.height();
    for &(x, y) in format_right(w, ht).iter().take(4) {
        h.set(x, y, !h.get(x, y));
    }
    assert_eq!(read_format(&h), None);
}

#[test]
fn wrong_size_and_wrong_format_are_refused() {
    let g = encode(5, false, &[Segment::numeric("1234567").unwrap()]).unwrap();
    // A grid of another size with the same bits.
    let mut other = BitMatrix::new(g.width() - 1, g.height());
    for y in 0..g.height() {
        for x in 0..g.width() - 1 {
            other.set(x, y, g.get(x, y));
        }
    }
    assert_eq!(decode_grid(&other).unwrap_err(), Reject::Sampling);
    // The format of R9x43 in a symbol of R9x59.
    let g2 = encode(6, false, &[Segment::numeric("1234567").unwrap()]).unwrap();
    let mut mixed = g2.clone();
    for &(x, y) in format_left().iter() {
        mixed.set(x, y, g.get(x, y));
    }
    for &(x, y) in format_right(43, 9).iter() {
        let (dx, dy) = (x + 16, y);
        mixed.set(dx, dy, g.get(x, y));
    }
    assert!(decode_grid(&mixed).is_err());
}

#[test]
fn modified_padding_is_refused() {
    use super::bits::parse;
    let mut stream = vec![0u8; 6];
    // Mode 001, count 0001, digit 0001, then the terminator and zeros.
    stream[0] = 0b0010_0010;
    stream[1] = 0b0010_0000;
    stream[2] = 0xEC;
    stream[3] = 0x11;
    stream[4] = 0xEC;
    stream[5] = 0x11;
    let p = parse(&stream, 0).unwrap();
    assert_eq!(p.data, b"1");
    let mut bad = stream.clone();
    bad[3] = 0x10;
    assert!(parse(&bad, 0).is_err());
    let mut bad = stream.clone();
    bad[1] |= 0x01;
    assert!(parse(&bad, 0).is_err());
    let mut bad = stream.clone();
    bad[4] = 0x11;
    assert!(parse(&bad, 0).is_err());
}

#[test]
fn random_grids_never_decode() {
    let mut s = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    for i in 0..VERSIONS {
        let t = template(i);
        for _ in 0..200 {
            let mut g = BitMatrix::new(t.w, t.h);
            for y in 0..t.h {
                for x in 0..t.w {
                    let want = match t.cell(x, y) {
                        DARK => true,
                        LIGHT => false,
                        _ => next() & 1 == 1,
                    };
                    g.set(x, y, want);
                }
            }
            // Function patterns perfect, format and data random.
            if let Ok(d) = decode_grid(&g) {
                panic!("v{i} decoded random content {:?}", d.data);
            }
        }
    }
}
