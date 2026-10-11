use super::decode::{correctable, decode_grid, read_format};
use super::spec::*;
use breader_core::{BitMatrix, EcLevel, Reject};
use qrcode::{Color, EcLevel as QEc, QrCode, Version};

fn qec(l: EcLevel) -> QEc {
    match l {
        EcLevel::L => QEc::L,
        EcLevel::M => QEc::M,
        EcLevel::Q => QEc::Q,
        _ => QEc::L,
    }
}

pub(crate) fn encode(data: &[u8], version: u8, level: EcLevel) -> Option<BitMatrix> {
    let code = QrCode::with_version(data, Version::Micro(i16::from(version)), qec(level)).ok()?;
    let w = code.width();
    let mut g = BitMatrix::new(w, w);
    for (i, c) in code.to_colors().iter().enumerate() {
        if *c == Color::Dark {
            g.set(i % w, i / w, true);
        }
    }
    Some(g)
}

fn transpose(g: &BitMatrix) -> BitMatrix {
    let d = g.width();
    let mut t = BitMatrix::new(d, d);
    for y in 0..d {
        for x in 0..d {
            t.set(y, x, g.get(x, y));
        }
    }
    t
}

/// Appends the `n` low bits of `v`, most significant first.
fn push(bits: &mut Vec<bool>, v: u32, n: usize) {
    bits.extend((0..n).rev().map(|i| v >> i & 1 == 1));
}

/// Bitstream of one numeric segment in a symbol of `version`.
pub(crate) fn numeric_stream(version: u8, digits: &str) -> Vec<bool> {
    let mut b = Vec::new();
    push(&mut b, 0, mode_bits(version));
    push(&mut b, digits.len() as u32, count_bits(version, 0));
    for chunk in digits.as_bytes().chunks(3) {
        let v: u32 = std::str::from_utf8(chunk).unwrap().parse().unwrap();
        push(&mut b, v, [0, 4, 7, 10][chunk.len()]);
    }
    b
}

/// Writer of symbols from a data bitstream: terminator, padding, check
/// codewords and placement follow the standard as read from the tables in
/// `spec`. `corrupt` lets a test damage the codewords before masking.
pub(crate) fn build(
    version: u8,
    level: EcLevel,
    mut stream: Vec<bool>,
    mask: u8,
    pad: impl Fn(&mut Vec<bool>, usize),
) -> BitMatrix {
    let number = VARIANTS
        .iter()
        .position(|v| v.version == version && v.level == level)
        .unwrap();
    let var = VARIANTS[number];
    let t = terminator_bits(version).min(var.data_bits - stream.len());
    stream.extend(std::iter::repeat_n(false, t));
    pad(&mut stream, var.data_bits);
    assert_eq!(stream.len(), var.data_bits);
    let dcw = var.data_codewords();
    let mut data = vec![0u8; dcw];
    for (i, &b) in stream.iter().enumerate() {
        data[i / 8] |= u8::from(b) << (7 - i % 8);
    }
    let mut ec = vec![0u8; var.ec];
    breader_ecc::QR_FIELD
        .encode(&data, var.ec, &mut ec)
        .unwrap();
    let mut bits = stream;
    for b in &ec {
        push(&mut bits, u32::from(*b), 8);
    }
    let d = dimension(version);
    let mut g = BitMatrix::new(d, d);
    for (&(x, y), &b) in data_positions(d).iter().zip(&bits) {
        g.set(x, y, b ^ mask_bit(mask, y, x));
    }
    for y in 0..7usize {
        for x in 0..7usize {
            let r = (x as i32 - 3).abs().max((y as i32 - 3).abs());
            g.set(x, y, r != 2);
        }
    }
    for i in 8..d {
        g.set(i, 0, i % 2 == 0);
        g.set(0, i, i % 2 == 0);
    }
    let word = crate::spec::FORMAT_WORDS[number << 2 | usize::from(mask)] ^ FORMAT_MASK;
    for (i, &(x, y)) in FORMAT_POSITIONS.iter().enumerate() {
        g.set(x, y, word >> i & 1 == 1);
    }
    g
}

/// Standard padding: zero bits to a codeword boundary, then alternating
/// pad codewords, a final half codeword being zeros.
pub(crate) fn standard_pad(s: &mut Vec<bool>, total: usize) {
    while !s.len().is_multiple_of(8) && s.len() < total {
        s.push(false);
    }
    let mut k = 0;
    while s.len() + 8 <= total {
        push(s, [0xEC, 0x11][k % 2], 8);
        k += 1;
    }
    s.resize(total, false);
}

#[test]
fn built_symbols_decode_at_every_variant() {
    for (n, var) in VARIANTS.iter().enumerate() {
        for mask in 0..4 {
            let s = numeric_stream(var.version, "1");
            let g = build(var.version, var.level, s, mask, standard_pad);
            let d = decode_grid(&g).unwrap_or_else(|e| panic!("{var:?} mask {mask}: {e:?}"));
            assert_eq!(d.data, b"1");
            assert_eq!(
                (d.version, d.ec_level, d.mask),
                (var.version, var.level, mask)
            );
            assert_eq!(read_format(&g).map(|f| f.0), Some(n));
        }
    }
}

#[test]
fn bad_padding_is_rejected() {
    let s = numeric_stream(2, "12");
    let ok = build(2, EcLevel::L, s.clone(), 1, standard_pad);
    assert!(decode_grid(&ok).is_ok());
    // Wrong pad codeword.
    let g = build(2, EcLevel::L, s.clone(), 1, |s, total| {
        standard_pad(s, total);
        let n = s.len();
        s[n - 3] = !s[n - 3];
    });
    assert!(decode_grid(&g).is_err());
    // Data after the terminator.
    let g = build(2, EcLevel::L, s, 1, |s, total| {
        standard_pad(s, total);
        s[total - 20] = true;
    });
    assert!(decode_grid(&g).is_err());
}

#[test]
fn format_information_matches_the_encoder() {
    for (n, var) in VARIANTS.iter().enumerate() {
        let g = encode(b"1", var.version, var.level).unwrap();
        let (number, mask, dist) = read_format(&g).expect("format");
        assert_eq!((number, dist), (n, 0), "{var:?} mask {mask}");
    }
}

#[test]
fn encoder_symbols_decode() {
    let payloads: [&[u8]; 4] = [b"1", b"12345", b"HELLO", b"ab"];
    let mut count = 0;
    for var in VARIANTS {
        for p in payloads {
            // The encoder's M3-L symbols do not decode (and a real M3-L
            // sample from the zxing-cpp set does with this decoder), so
            // that variant is covered by `built_symbols_decode_at_every_variant`.
            if (var.version, var.level) == (3, EcLevel::L) {
                continue;
            }
            let Some(g) = encode(p, var.version, var.level) else {
                continue;
            };
            let d = decode_grid(&g).unwrap_or_else(|e| panic!("{var:?} {p:?}: {e:?}"));
            assert_eq!(d.data, p, "{var:?}");
            assert_eq!((d.version, d.ec_level), (var.version, var.level));
            assert!(!d.mirrored);
            let m = decode_grid(&transpose(&g)).unwrap();
            assert_eq!(m.data, p);
            assert!(m.mirrored);
            count += 1;
        }
    }
    assert!(count >= 20, "{count}");
}

#[test]
fn correction_limits() {
    assert_eq!(
        VARIANTS.map(|v| correctable(v.ec)),
        [0, 1, 2, 2, 3, 3, 4, 6]
    );
    let g = encode(b"12345", 1, EcLevel::DetectionOnly).unwrap();
    let mut bad = g.clone();
    let (x, y) = data_positions(11)[3];
    bad.set(x, y, !g.get(x, y));
    assert!(decode_grid(&bad).is_err());
    // M4-L corrects up to 3 codewords: flip modules in 3 distinct codewords.
    let g = encode(b"HELLO WORLD 0123", 4, EcLevel::L).unwrap();
    let pos = data_positions(17);
    let mut bad = g.clone();
    for cw in [0usize, 5, 9] {
        let (x, y) = pos[cw * 8 + 2];
        bad.set(x, y, !g.get(x, y));
    }
    assert_eq!(decode_grid(&bad).unwrap().errors_corrected, 3);
    let (x, y) = pos[12 * 8 + 2];
    bad.set(x, y, !g.get(x, y));
    assert_eq!(decode_grid(&bad), Err(Reject::ErrorCorrection));
}
