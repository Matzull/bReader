use super::*;
use crate::spec;
use qrcode::bits::Bits;
use qrcode::types::Version;
use qrcode::{Color, EcLevel as QEc, QrCode};

const LEVELS: [EcLevel; 4] = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H];

fn qec(l: EcLevel) -> QEc {
    match l {
        EcLevel::L => QEc::L,
        EcLevel::M => QEc::M,
        EcLevel::Q => QEc::Q,
        EcLevel::H => QEc::H,
        _ => unreachable!(),
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn to_grid(code: &QrCode) -> BitMatrix {
    let w = code.width();
    let mut g = BitMatrix::new(w, w);
    for (i, c) in code.to_colors().iter().enumerate() {
        if *c == Color::Dark {
            g.set(i % w, i / w, true);
        }
    }
    g
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

// ---- test-only encoder -------------------------------------------------

fn gf_tables() -> ([u8; 512], [u8; 256]) {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x = 1u16;
    for (i, e) in exp.iter_mut().enumerate().take(255) {
        *e = x as u8;
        log[x as usize] = i as u8;
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11d;
        }
    }
    for i in 255..512 {
        exp[i] = exp[i - 255];
    }
    (exp, log)
}

fn rs_encode(data: &[u8], ec: usize) -> Vec<u8> {
    let (exp, log) = gf_tables();
    let mul = |a: u8, b: u8| {
        if a == 0 || b == 0 {
            0
        } else {
            exp[usize::from(log[usize::from(a)]) + usize::from(log[usize::from(b)])]
        }
    };
    let mut g = vec![1u8];
    for &root in exp.iter().take(ec) {
        let mut n = vec![0u8; g.len() + 1];
        for (j, &c) in g.iter().enumerate() {
            n[j] ^= c;
            n[j + 1] ^= mul(c, root);
        }
        g = n;
    }
    let mut rem = vec![0u8; ec];
    for &d in data {
        let f = d ^ rem[0];
        rem.remove(0);
        rem.push(0);
        for (r, &gc) in rem.iter_mut().zip(&g[1..]) {
            *r ^= mul(gc, f);
        }
    }
    rem
}

/// Interleaves data blocks and EC, returns the final codeword sequence.
fn make_codewords(version: u8, level: EcLevel, data: &[u8]) -> Vec<u8> {
    let layout = spec::block_layout(version, level).unwrap();
    assert_eq!(data.len(), layout.data_codewords());
    let ec = usize::from(layout.ec_per_block);
    let mut blocks: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
    let mut off = 0;
    for &(n, k) in &layout.groups {
        for _ in 0..n {
            let d = data[off..off + usize::from(k)].to_vec();
            off += usize::from(k);
            let e = rs_encode(&d, ec);
            blocks.push((d, e));
        }
    }
    let mut out = Vec::new();
    let maxk = blocks.iter().map(|b| b.0.len()).max().unwrap();
    for i in 0..maxk {
        for b in &blocks {
            if let Some(&c) = b.0.get(i) {
                out.push(c);
            }
        }
    }
    for i in 0..ec {
        for b in &blocks {
            out.push(b.1[i]);
        }
    }
    out
}

fn bch_word(value: u32, poly: u32, poly_bits: u32, data_bits: u32) -> u32 {
    let mut rem = value << (poly_bits - 1);
    for bit in (poly_bits - 1..data_bits + poly_bits - 1).rev() {
        if rem & (1 << bit) != 0 {
            rem ^= poly << (bit - (poly_bits - 1));
        }
    }
    (value << (poly_bits - 1)) | rem
}

/// Places codewords, format and version information. Finder/timing patterns
/// are not drawn (the decoder never reads them), except where the format
/// areas need no help.
fn place(version: u8, level: EcLevel, mask: u8, codewords: &[u8]) -> BitMatrix {
    let d = spec::dimension(version);
    let mut g = BitMatrix::new(d, d);
    for (i, &(x, y)) in spec::data_positions(version).iter().enumerate() {
        let bit = codewords
            .get(i / 8)
            .is_some_and(|c| c & (0x80 >> (i % 8)) != 0);
        g.set(x, y, bit ^ mask_bit(mask, x, y));
    }
    let lv = match level {
        EcLevel::L => 1,
        EcLevel::M => 0,
        EcLevel::Q => 3,
        EcLevel::H => 2,
        _ => unreachable!(),
    };
    let fmt = bch_word((lv << 3) | u32::from(mask), 0x537, 11, 5) ^ 0x5412;
    for p in spec::format_positions(d) {
        for (i, &(x, y)) in p.iter().enumerate() {
            g.set(x, y, fmt & (1 << i) != 0);
        }
    }
    if version >= 7 {
        let w = bch_word(u32::from(version), 0x1F25, 13, 6);
        for p in spec::version_positions(d) {
            for (i, &(x, y)) in p.iter().enumerate() {
                g.set(x, y, w & (1 << i) != 0);
            }
        }
    }
    g
}

/// Bit writer for hand-built streams.
#[derive(Default)]
struct Bw(Vec<bool>);
impl Bw {
    fn put(&mut self, v: u32, n: usize) {
        for i in (0..n).rev() {
            self.0.push(v >> i & 1 != 0);
        }
    }
    fn finish(mut self, version: u8, level: EcLevel) -> Vec<u8> {
        let cap = spec::block_layout(version, level).unwrap().data_codewords() * 8;
        assert!(self.0.len() <= cap);
        let t = 4.min(cap - self.0.len());
        self.put(0, t);
        while !self.0.len().is_multiple_of(8) {
            self.0.push(false);
        }
        let mut bytes: Vec<u8> = self
            .0
            .chunks(8)
            .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
            .collect();
        let mut pad = [0xEC, 0x11].into_iter().cycle();
        while bytes.len() * 8 < cap {
            bytes.push(pad.next().unwrap());
        }
        bytes
    }
}

fn hand(version: u8, level: EcLevel, mask: u8, bw: Bw) -> BitMatrix {
    let data = bw.finish(version, level);
    place(version, level, mask, &make_codewords(version, level, &data))
}

// ---- tests ---------------------------------------------------------------

#[test]
fn annex_i_golden() {
    let data = [
        0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC,
        0x11,
    ];
    let ec = rs_encode(&data, 10);
    assert_eq!(
        ec,
        [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55]
    );
    let g = place(1, EcLevel::M, 2, &make_codewords(1, EcLevel::M, &data));
    let r = decode_grid(&g).unwrap();
    assert_eq!(r.data, b"01234567");
    assert_eq!((r.version, r.ec_level, r.mask), (1, EcLevel::M, 2));
    assert_eq!(r.errors_corrected, 0);
    assert!(!r.mirrored);
    assert_eq!(&r.aim_id, b"]Q1");
}

#[test]
fn encoder_helper_agrees_with_qrcode_crate() {
    // Version 1 needs no alignment patterns, so this runs before alignment lands.
    for l in LEVELS {
        let code = QrCode::with_version(b"HELLO", Version::Normal(1), qec(l)).unwrap();
        let r = decode_grid(&to_grid(&code)).unwrap();
        assert_eq!(r.data, b"HELLO");
        assert_eq!(r.ec_level, l);
    }
}

fn sample_text(rng: &mut Rng, kind: usize, len: usize) -> Vec<u8> {
    (0..len)
        .map(|_| match kind {
            0 => b'0' + rng.below(10) as u8,
            1 => spec::ALNUM[rng.below(45)],
            2 => rng.below(256) as u8,
            _ => {
                if rng.below(2) == 0 {
                    b'0' + rng.below(10) as u8
                } else {
                    b'a' + rng.below(26) as u8
                }
            }
        })
        .collect()
}

fn encode_fit(rng: &mut Rng, v: u8, l: EcLevel, kind: usize) -> (QrCode, Vec<u8>) {
    let layout = spec::block_layout(v, l).unwrap();
    let mut len = layout.data_codewords() * 3 / 2;
    loop {
        let text = sample_text(rng, kind, len);
        if let Ok(code) = QrCode::with_version(&text, Version::Normal(i16::from(v)), qec(l)) {
            return (code, text);
        }
        len = len * 9 / 10;
        assert!(len > 0);
    }
}

#[test]
fn roundtrip_all_versions_levels_modes() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for v in 1..=40u8 {
        for l in LEVELS {
            for kind in 0..4 {
                let (code, text) = encode_fit(&mut rng, v, l, kind);
                let r = decode_grid(&to_grid(&code))
                    .unwrap_or_else(|e| panic!("v{v} {l:?} k{kind}: {e:?}"));
                assert_eq!(r.data, text, "v{v} {l:?} k{kind}");
                assert_eq!((r.version, r.ec_level), (v, l));
                assert_eq!(r.errors_corrected, 0);
                assert!(!r.mirrored && r.eci.is_empty());
                assert_eq!(&r.aim_id, b"]Q1");
            }
        }
    }
}

#[test]
fn all_masks_hand_encoded() {
    let mut rng = Rng(77);
    for v in [1u8, 5, 9, 10, 27, 40] {
        for l in LEVELS {
            for mask in 0..8 {
                let mut bw = Bw::default();
                let text = sample_text(&mut rng, 2, 5);
                bw.put(4, 4);
                bw.put(5, count_bits_for(v, 2));
                for &b in &text {
                    bw.put(u32::from(b), 8);
                }
                let r = decode_grid(&hand(v, l, mask, bw)).unwrap();
                assert_eq!((r.mask, r.data.as_slice()), (mask, text.as_slice()));
            }
        }
    }
}

fn count_bits_for(v: u8, mode: usize) -> usize {
    spec::count_bits(v, mode)
}

fn bits_decode(version: i16, level: EcLevel, f: impl FnOnce(&mut Bits)) -> DecodedQr {
    let mut bits = Bits::new(Version::Normal(version));
    f(&mut bits);
    bits.push_terminator(qec(level)).unwrap();
    let code = QrCode::with_bits(bits, qec(level)).unwrap();
    decode_grid(&to_grid(&code)).unwrap()
}

#[test]
fn explicit_modes_via_qrcode_bits() {
    let r = bits_decode(1, EcLevel::L, |b| {
        b.push_numeric_data(b"0123456789").unwrap()
    });
    assert_eq!(r.data, b"0123456789");
    let r = bits_decode(1, EcLevel::L, |b| {
        b.push_alphanumeric_data(b"AC-42 $%*+./:").unwrap()
    });
    assert_eq!(r.data, b"AC-42 $%*+./:");
    let r = bits_decode(1, EcLevel::M, |b| {
        b.push_byte_data(&[0, 255, 0x1D, 65]).unwrap()
    });
    assert_eq!(r.data, [0, 255, 0x1D, 65]);
    // Mixed segments, then odd lengths of numeric
    let r = bits_decode(2, EcLevel::L, |b| {
        b.push_numeric_data(b"12").unwrap();
        b.push_alphanumeric_data(b"X").unwrap();
        b.push_numeric_data(b"1").unwrap();
        b.push_byte_data(b"z").unwrap();
    });
    assert_eq!(r.data, b"12X1z");
}

#[test]
fn eci_designators() {
    for (eci, aim) in [
        (26u32, b"]Q2"),
        (127, b"]Q2"),
        (128, b"]Q2"),
        (16383, b"]Q2"),
        (16384, b"]Q2"),
        (999_999, b"]Q2"),
    ] {
        let r = bits_decode(3, EcLevel::L, |b| {
            b.push_eci_designator(eci).unwrap();
            b.push_byte_data(b"hi").unwrap();
        });
        assert_eq!(r.eci, [eci]);
        assert_eq!(r.data, b"hi");
        assert_eq!(&r.aim_id, aim);
    }
    let r = bits_decode(3, EcLevel::L, |b| {
        b.push_eci_designator(3).unwrap();
        b.push_byte_data(b"a").unwrap();
        b.push_eci_designator(20).unwrap();
        b.push_byte_data(b"b").unwrap();
    });
    assert_eq!(r.eci, [3, 20]);
}

#[test]
fn fnc1_modes() {
    let r = bits_decode(2, EcLevel::L, |b| {
        b.push_fnc1_first_position().unwrap();
        b.push_alphanumeric_data(b"01%10ABC%%X").unwrap();
    });
    assert_eq!(r.data, b"01\x1D10ABC%X");
    assert_eq!(&r.aim_id, b"]Q3");
    let r = bits_decode(2, EcLevel::L, |b| {
        b.push_fnc1_second_position(37).unwrap();
        b.push_alphanumeric_data(b"AB").unwrap();
    });
    assert_eq!(r.data, b"37AB");
    assert_eq!(&r.aim_id, b"]Q5");
    let r = bits_decode(2, EcLevel::L, |b| {
        b.push_fnc1_second_position(b'a' + 100).unwrap();
        b.push_numeric_data(b"1").unwrap();
    });
    assert_eq!(r.data, b"a1");
}

#[test]
fn fnc1_with_eci_ids() {
    let mut bw = Bw::default();
    bw.put(5, 4);
    bw.put(7, 4);
    bw.put(9, 8);
    bw.put(4, 4);
    bw.put(1, 8);
    bw.put(u32::from(b'x'), 8);
    let r = decode_grid(&hand(1, EcLevel::L, 0, bw)).unwrap();
    assert_eq!(
        (r.data.as_slice(), &r.aim_id, r.eci.as_slice()),
        (&b"x"[..], b"]Q4", &[9][..])
    );
    let mut bw = Bw::default();
    bw.put(7, 4);
    bw.put(9, 8);
    bw.put(9, 4);
    bw.put(5, 8);
    bw.put(4, 4);
    bw.put(1, 8);
    bw.put(u32::from(b'x'), 8);
    let r = decode_grid(&hand(1, EcLevel::L, 0, bw)).unwrap();
    assert_eq!(&r.aim_id, b"]Q6");
    assert_eq!(r.data, b"05x");
}

#[test]
fn kanji_mode() {
    // Shift-JIS 0x935F (U+4E00 area) -> 0x935F - 0x8140 = 0x121F -> 0x12*0xC0+0x1F = 0xD9F
    // and 0xE040 -> 0xE040 - 0xC140 = 0x1F00 -> 0x1F*0xC0+0 = 0x1740.
    let mut bw = Bw::default();
    bw.put(8, 4);
    bw.put(2, 8);
    bw.put(0x0D9F, 13);
    bw.put(0x1740, 13);
    let r = decode_grid(&hand(1, EcLevel::M, 3, bw)).unwrap();
    assert_eq!(r.data, [0x93, 0x5F, 0xE0, 0x40]);
    let r = bits_decode(1, EcLevel::L, |b| {
        b.push_kanji_data(&[0x93, 0x5F, 0xE0, 0x40]).unwrap()
    });
    assert_eq!(r.data, [0x93, 0x5F, 0xE0, 0x40]);
}

#[test]
fn structured_append_is_skipped() {
    let mut bw = Bw::default();
    bw.put(3, 4);
    bw.put(1, 4);
    bw.put(3, 4);
    bw.put(0xA7, 8);
    bw.put(1, 4);
    bw.put(3, 10);
    bw.put(123, 10);
    let r = decode_grid(&hand(1, EcLevel::L, 1, bw)).unwrap();
    assert_eq!(r.data, b"123");
}

type Case = Box<dyn Fn(&mut Bw)>;

#[test]
fn malformed_bitstreams_rejected() {
    let cases: Vec<Case> = vec![
        // numeric group 1000
        Box::new(|b| {
            b.put(1, 4);
            b.put(3, 10);
            b.put(1000, 10);
        }),
        // numeric pair 100
        Box::new(|b| {
            b.put(1, 4);
            b.put(2, 10);
            b.put(100, 7);
        }),
        // numeric single 10
        Box::new(|b| {
            b.put(1, 4);
            b.put(1, 10);
            b.put(10, 4);
        }),
        // alnum pair >= 2025
        Box::new(|b| {
            b.put(2, 4);
            b.put(2, 9);
            b.put(2025, 11);
        }),
        // alnum single >= 45
        Box::new(|b| {
            b.put(2, 4);
            b.put(1, 9);
            b.put(45, 6);
        }),
        // byte count beyond the data
        Box::new(|b| {
            b.put(4, 4);
            b.put(200, 8);
            b.put(0, 8);
        }),
        // unknown mode 0b0110
        Box::new(|b| {
            b.put(6, 4);
            b.put(0, 8);
        }),
        // invalid ECI header 111xxxxx
        Box::new(|b| {
            b.put(7, 4);
            b.put(0xE0, 8);
        }),
        // FNC1 after data
        Box::new(|b| {
            b.put(1, 4);
            b.put(1, 10);
            b.put(1, 4);
            b.put(5, 4);
        }),
        // bad application indicator
        Box::new(|b| {
            b.put(9, 4);
            b.put(150, 8);
        }),
        // numeric segment cut by the end of data
        Box::new(|b| {
            b.put(1, 4);
            b.put(500, 10);
        }),
    ];
    for (i, c) in cases.iter().enumerate() {
        let mut bw = Bw::default();
        c(&mut bw);
        let cap = spec::block_layout(1, EcLevel::L).unwrap().data_codewords() * 8;
        // do not pad with a terminator inside the bad segment: pad with ones
        while bw.0.len() < cap && i != 10 {
            bw.put(0, 1);
        }
        let data: Vec<u8> =
            bw.0.chunks(8)
                .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
                .collect();
        let mut data = data;
        data.resize(19, if i == 10 { 0xFF } else { 0 });
        let g = place(1, EcLevel::L, 4, &make_codewords(1, EcLevel::L, &data));
        let e = decode_grid(&g);
        assert!(e.is_err(), "case {i}: {e:?}");
    }
}

#[test]
fn nothing_read_after_terminator() {
    let mut bw = Bw::default();
    bw.put(4, 4);
    bw.put(1, 8);
    bw.put(65, 8);
    bw.put(0, 4);
    // garbage that looks like a valid byte segment after the terminator
    bw.put(4, 4);
    bw.put(1, 8);
    bw.put(66, 8);
    let cap = 19 * 8;
    while bw.0.len() < cap {
        bw.put(1, 1);
    }
    let data: Vec<u8> =
        bw.0.chunks(8)
            .map(|c| c.iter().fold(0u8, |a, &b| (a << 1) | u8::from(b)))
            .collect();
    let r = decode_grid(&place(
        1,
        EcLevel::L,
        0,
        &make_codewords(1, EcLevel::L, &data),
    ))
    .unwrap();
    assert_eq!(r.data, b"A");
}

fn flip_codeword_modules(
    g: &mut BitMatrix,
    version: u8,
    level: EcLevel,
    rng: &mut Rng,
    count: usize,
) {
    let total = spec::block_layout(version, level)
        .unwrap()
        .total_codewords()
        * 8;
    let pos = spec::data_positions(version);
    let mut chosen: Vec<usize> = Vec::new();
    while chosen.len() < count {
        let i = rng.below(total);
        if !chosen.contains(&i) {
            chosen.push(i);
            let (x, y) = pos[i];
            g.set(x, y, !g.get(x, y));
        }
    }
}

#[test]
fn errors_within_capacity_are_corrected() {
    let mut rng = Rng(0xdead_beef_cafe);
    for v in [1u8, 2, 3, 6, 7, 10, 15, 21, 27, 33, 40] {
        for l in LEVELS {
            let (code, text) = encode_fit(&mut rng, v, l, 2);
            let ec = usize::from(spec::block_layout(v, l).unwrap().ec_per_block);
            for _ in 0..4 {
                let mut g = to_grid(&code);
                let n = 1 + rng.below((ec - usize::from(spec::misdecode_protection(v, l))) / 2);
                flip_codeword_modules(&mut g, v, l, &mut rng, n);
                let r = decode_grid(&g).unwrap_or_else(|e| panic!("v{v} {l:?} n{n}: {e:?}"));
                assert_eq!(r.data, text);
                assert!(
                    r.errors_corrected >= 1 && r.errors_corrected as usize <= n,
                    "{}",
                    r.errors_corrected
                );
            }
        }
    }
}

#[test]
fn errors_beyond_capacity_never_yield_wrong_data() {
    let mut rng = Rng(0x5eed);
    for v in [5u8, 8, 12, 20, 30] {
        for l in [EcLevel::Q, EcLevel::H] {
            let (code, text) = encode_fit(&mut rng, v, l, 2);
            let total = spec::block_layout(v, l).unwrap().total_codewords() * 8;
            for _ in 0..10 {
                let mut g = to_grid(&code);
                flip_codeword_modules(&mut g, v, l, &mut rng, total / 4);
                if let Ok(r) = decode_grid(&g) {
                    assert_eq!(r.data, text, "wrong data accepted v{v} {l:?}");
                }
            }
        }
    }
    // Small codes (misdecode protection): more errors than allowed must never
    // produce wrong data.
    for v in 1..=3u8 {
        for l in LEVELS {
            let (code, text) = encode_fit(&mut rng, v, l, 2);
            let layout = spec::block_layout(v, l).unwrap();
            let allowed = (usize::from(layout.ec_per_block)
                - usize::from(spec::misdecode_protection(v, l)))
                / 2;
            for _ in 0..1500 {
                let mut g = to_grid(&code);
                let n = allowed + 1 + rng.below(40);
                flip_codeword_modules(&mut g, v, l, &mut rng, n);
                if let Ok(r) = decode_grid(&g) {
                    assert_eq!(r.data, text, "wrong data accepted v{v} {l:?} n{n}");
                }
            }
        }
    }
}

#[test]
fn transposed_grid_is_mirrored() {
    let mut rng = Rng(9);
    for v in [1u8, 4, 7, 14, 40] {
        for l in LEVELS {
            let (code, text) = encode_fit(&mut rng, v, l, 3);
            let r = decode_grid(&transpose(&to_grid(&code))).unwrap();
            assert!(r.mirrored);
            assert_eq!(r.data, text);
            assert_eq!((r.version, r.ec_level), (v, l));
        }
    }
}

#[test]
fn format_info_error_tolerance() {
    let mut rng = Rng(31);
    let (code, text) = encode_fit(&mut rng, 1, EcLevel::Q, 1);
    let base = to_grid(&code);
    let d = base.width();
    let pos = spec::format_positions(d);
    for copy in 0..2 {
        // up to 3 flipped bits in one copy, plus the other copy destroyed
        for nflip in 0..=3 {
            let mut g = base.clone();
            for &(x, y) in &pos[1 - copy] {
                g.set(x, y, rng.below(2) == 0);
            }
            let mut idx: Vec<usize> = Vec::new();
            while idx.len() < nflip {
                let i = rng.below(15);
                if !idx.contains(&i) {
                    idx.push(i);
                    let (x, y) = pos[copy][i];
                    g.set(x, y, !g.get(x, y));
                }
            }
            let r = decode_grid(&g);
            // the destroyed copy may happen to be nearest-valid too; the result must still be right or rejected
            if let Ok(r) = r {
                assert_eq!(r.data, text);
            }
            if nflip <= 3 {
                // retry with the damaged copy as the only candidate: always decodes
                let mut g2 = base.clone();
                for &(x, y) in &pos[1 - copy] {
                    g2.set(x, y, false);
                }
                for &i in &idx {
                    let (x, y) = pos[copy][i];
                    g2.set(x, y, !g2.get(x, y));
                }
                assert_eq!(decode_grid(&g2).unwrap().data, text);
            }
        }
    }
}

#[test]
fn version_info_read_and_correct() {
    let mut rng = Rng(5);
    for v in 7..=40u8 {
        let (code, _) = encode_fit(&mut rng, v, EcLevel::L, 0);
        let base = to_grid(&code);
        let d = base.width();
        assert_eq!(read_version(&base), Some(v));
        let pos = spec::version_positions(d);
        for copy in 0..2 {
            let mut g = base.clone();
            for _ in 0..3 {
                let (x, y) = pos[copy][rng.below(18)];
                g.set(x, y, !g.get(x, y));
            }
            // flipping the same bit twice cancels, still within distance 3
            assert_eq!(read_version(&g), Some(v));
            let mut g = base.clone();
            for &(x, y) in &pos[1 - copy] {
                g.set(x, y, !g.get(x, y));
            }
            assert_eq!(read_version(&g), Some(v));
        }
    }
    assert_eq!(read_version(&BitMatrix::new(21, 21)), None);
    assert_eq!(read_version(&BitMatrix::new(10, 12)), None);
}

#[test]
fn random_grids_never_panic_or_succeed_spuriously() {
    let mut rng = Rng(0x0bad_f00d);
    let mut oks = 0;
    for i in 0..3000 {
        let v = 1 + rng.below(40) as u8;
        let d = spec::dimension(v);
        let mut g = BitMatrix::new(d, d);
        let density = [2, 5, 10, 1][i % 4];
        for y in 0..d {
            for x in 0..d {
                if rng.below(10) < density {
                    g.set(x, y, true);
                }
            }
        }
        if decode_grid(&g).is_ok() {
            oks += 1;
        }
    }
    assert_eq!(oks, 0);
    // invalid shapes
    for (w, h) in [
        (0, 0),
        (1, 1),
        (20, 20),
        (22, 22),
        (21, 22),
        (178, 178),
        (181, 181),
    ] {
        assert!(decode_grid(&BitMatrix::new(w, h)).is_err());
    }
}

#[test]
fn random_valid_codewords_with_random_streams_never_panic() {
    // Valid RS blocks around random payloads exercise the bitstream parser.
    let mut rng = Rng(0x7777);
    for _ in 0..1500 {
        let v = 1 + rng.below(40) as u8;
        let l = LEVELS[rng.below(4)];
        let k = spec::block_layout(v, l).unwrap().data_codewords();
        let data: Vec<u8> = (0..k)
            .map(|i| {
                if i < 6 || rng.below(4) == 0 {
                    rng.below(256) as u8
                } else {
                    0
                }
            })
            .collect();
        let g = place(v, l, rng.below(8) as u8, &make_codewords(v, l, &data));
        let _ = decode_grid(&g);
    }
}

#[test]
fn segments_record_charsets_and_merge() {
    use breader_core::{Charset, DataSegment};
    let seg = |start, end, charset| DataSegment {
        start,
        end,
        charset,
    };
    let r = bits_decode(5, EcLevel::L, |b| {
        b.push_numeric_data(b"12").unwrap();
        b.push_alphanumeric_data(b"AB").unwrap();
        b.push_kanji_data(&[0x93, 0x5F]).unwrap();
        b.push_byte_data(b"x").unwrap();
        b.push_eci_designator(26).unwrap();
        b.push_byte_data(b"yz").unwrap();
        b.push_eci_designator(3).unwrap();
        b.push_byte_data(b"w").unwrap();
        b.push_alphanumeric_data(b"Q").unwrap();
    });
    assert_eq!(
        r.segments,
        [
            seg(0, 4, Charset::Ascii),
            seg(4, 6, Charset::ShiftJis),
            seg(6, 7, Charset::Undeclared),
            seg(7, 9, Charset::Eci(26)),
            seg(9, 10, Charset::Eci(3)),
            seg(10, 11, Charset::Ascii),
        ]
    );
    let r = bits_decode(2, EcLevel::L, |b| {
        b.push_fnc1_first_position().unwrap();
        b.push_alphanumeric_data(b"01%10ABC").unwrap();
    });
    assert_eq!(r.segments.len(), 1);
    assert_eq!(r.segments[0].end, r.data.len());
}
