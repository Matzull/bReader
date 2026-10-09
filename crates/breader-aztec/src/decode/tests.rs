use super::*;
use crate::build::Bits;
use crate::testutil::{
    self, Rendered, Rng, flip_bit, render, render_rune, render_stream, rxing_encode,
};
use rxing::common::CharacterSet;

fn printable(rng: &mut Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| b' ' + rng.below(95) as u8).collect()
}

/// A payload that fits a symbol with the signed layer count of the encoder
/// (negative: compact) at any error correction level used here.
fn sized(rng: &mut Rng, layers: i32, want: usize) -> Vec<u8> {
    let layout = Layout {
        compact: layers < 0,
        layers: layers.unsigned_abs() as usize,
    };
    printable(rng, want.min(layout.total_bits() / 24).max(1))
}

fn decode(r: &Rendered) -> DecodedAztec {
    decode_grid(&r.grid, false).expect("decodes")
}

#[test]
fn rxing_compact_layers() {
    let mut rng = Rng(1);
    for layers in 1..=4i32 {
        for round in 0..6 {
            let n = 1 + rng.below(8 * layers as usize);
            let data = printable(&mut rng, n);
            let r = rxing_encode(&data, 23, -layers, None).expect("encodes");
            assert_eq!(r.layout.layers, layers as usize);
            let d = decode(&r);
            assert_eq!(d.data, data, "layers {layers} round {round}");
            assert!(d.compact);
            assert_eq!(usize::from(d.layers), r.layout.layers);
            assert_eq!(usize::from(d.data_codewords), r.data_codewords);
            assert_eq!(d.errors_corrected, 0);
            assert_eq!(d.aim_id, *b"]z0");
            assert_eq!(
                d.segments,
                vec![DataSegment {
                    start: 0,
                    end: data.len(),
                    charset: Charset::Latin1
                }]
            );
        }
    }
}

#[test]
fn rxing_full_layers() {
    let mut rng = Rng(2);
    for layers in 1..=32i32 {
        let bits = Layout {
            compact: false,
            layers: layers as usize,
        }
        .total_bits();
        let n = 1 + rng.below(bits / 16);
        let data = printable(&mut rng, n);
        let r = rxing_encode(&data, 23, layers, None).expect("encodes");
        assert_eq!(r.layout.layers, layers as usize);
        assert!(!r.layout.compact);
        let d = decode(&r);
        assert_eq!(d.data, data, "layers {layers}");
        assert!(!d.compact);
        assert_eq!(usize::from(d.layers), layers as usize);
        assert_eq!(usize::from(d.data_codewords), r.data_codewords);
        assert_eq!(
            usize::from(d.data_codewords + d.ec_codewords),
            r.layout.total_codewords()
        );
    }
}

#[test]
fn rxing_automatic_sizes() {
    let mut rng = Rng(3);
    for len in [1usize, 5, 13, 40, 100, 300, 800, 1500] {
        let data = printable(&mut rng, len);
        let r = rxing_encode(&data, 23, 0, None).expect("encodes");
        assert_eq!(decode(&r).data, data, "len {len}");
        let r = rxing_encode(&data, 60, 0, None).expect("encodes");
        assert_eq!(decode(&r).data, data, "len {len} ec 60");
    }
}

#[test]
fn transposed_symbol_is_the_mirror_image() {
    let mut rng = Rng(4);
    for layers in [-1, -3, 1, 5, 20] {
        let data = sized(&mut rng, layers, 12);
        let r = rxing_encode(&data, 23, layers, None).expect("encodes");
        let mirrored = transpose(&r.grid);
        assert!(decode_grid(&mirrored, false).is_err(), "layers {layers}");
        let d = decode_grid(&transpose(&mirrored), true).expect("decodes");
        assert_eq!(d.data, data);
        assert!(d.mirrored);
        assert_eq!(transpose(&transpose(&r.grid)), r.grid);
    }
}

#[test]
fn wrong_rotation_is_rejected() {
    let r = rxing_encode(b"ROTATE ME", 23, 0, None).unwrap();
    let n = r.grid.width();
    let mut rot = BitMatrix::new(n, n);
    for y in 0..n {
        for x in 0..n {
            if r.grid.get(x, y) {
                rot.set(n - 1 - y, x, true);
            }
        }
    }
    assert!(decode_grid(&rot, false).is_err());
    assert!(decode_grid(&r.grid, false).is_ok());
}

#[test]
fn builder_matches_rxing() {
    // The independent writer must produce the same modules as the encoder
    // for the same codewords: compare after decoding both.
    let mut rng = Rng(5);
    for layers in [-2, -4, 3, 9, 24] {
        let data = sized(&mut rng, layers, 20);
        let r = rxing_encode(&data, 40, layers, None).unwrap();
        // Reuse the encoder's own data codewords through our decoder.
        let words: Vec<u16> = {
            let w = r.layout.word_size();
            let bits: Vec<bool> = spec::data_positions(r.layout)
                .iter()
                .map(|&(x, y)| r.grid.get(x, y))
                .collect();
            bits[r.layout.start_pad()..]
                .chunks_exact(w)
                .take(r.data_codewords)
                .map(|c| c.iter().fold(0u16, |a, &b| a << 1 | u16::from(b)))
                .collect()
        };
        let mine = render(r.layout, &words);
        assert_eq!(mine, r.grid, "layers {layers}");
    }
}

#[test]
fn text_modes_through_the_encoder() {
    let samples: Vec<&[u8]> = vec![
        b"HELLO WORLD",
        b"hello world",
        b"Hello, World! 0123456789",
        b"a1b2c3 D4E5F6",
        b"1234567890,.1234567890",
        b"line1\r\nline2\rline3\n",
        b". , : ; ! \" # $ % & ' ( ) * + - / < = > ? [ ] { }",
        b"@\\^_`|~\x7f\x1b\x1c\x1d\x1e\x1f\x01\x02\x03\x04\x05\x06\x07\x08\t\x0b\x0c",
        b"ABC abc 123 !@# \x00\x01 \xe9\xff",
        b"12345678901234567890 ABCDEFGHIJ klmnop. A, b: C",
    ];
    for s in samples {
        let r = rxing_encode(s, 23, 0, None).unwrap();
        assert_eq!(decode(&r).data, s, "{}", String::from_utf8_lossy(s));
    }
}

#[test]
fn every_byte_and_byte_pair() {
    let all: Vec<u8> = (0..=255).collect();
    let r = rxing_encode(&all, 23, 0, None).unwrap();
    assert_eq!(decode(&r).data, all);
    let mut rng = Rng(6);
    for _ in 0..300 {
        let n = 2 + rng.below(6);
        let data: Vec<u8> = (0..n).map(|_| rng.next() as u8).collect();
        let r = rxing_encode(&data, 23, 0, None).unwrap();
        assert_eq!(decode(&r).data, data);
    }
    // Text biased toward mode changes.
    let alphabet = b"AZaz09 ,.:;!?\r\n[]{}@^|\x01\x1b\xa0\xff";
    for _ in 0..300 {
        let n = 1 + rng.below(60);
        let data: Vec<u8> = (0..n)
            .map(|_| alphabet[rng.below(alphabet.len())])
            .collect();
        let r = rxing_encode(&data, 23, 0, None).unwrap();
        assert_eq!(decode(&r).data, data, "{data:?}");
    }
}

#[test]
fn binary_shift_lengths() {
    let mut rng = Rng(7);
    for len in [
        1usize, 2, 30, 31, 32, 33, 62, 63, 64, 500, 2047, 2078, 2079, 2200,
    ] {
        let mut data = vec![0xE9u8];
        data.extend((1..len).map(|_| 0x80 | rng.next() as u8));
        let r = rxing_encode(&data, 23, 0, None);
        let Some(r) = r else { continue };
        assert_eq!(decode(&r).data, data, "len {len}");
    }
    let mut mixed = b"TEXT before ".to_vec();
    mixed.extend((0..70).map(|i| 0x80 + i as u8));
    mixed.extend_from_slice(b" and after 123");
    let r = rxing_encode(&mixed, 23, 0, None).unwrap();
    assert_eq!(decode(&r).data, mixed);
}

#[test]
fn eci_through_the_encoder() {
    for (cs, eci, text) in [
        (CharacterSet::UTF8, 26, "caf\u{e9} \u{20ac}"),
        (CharacterSet::ISO8859_2, 4, "\u{141}\u{f3}d\u{17a}"),
        (CharacterSet::Shift_JIS, 20, "\u{30c6}\u{30b9}\u{30c8}"),
    ] {
        let bytes = cs.encode(text).unwrap();
        let r = rxing_encode(&bytes, 23, 0, Some(cs)).unwrap();
        let d = decode(&r);
        assert_eq!(d.data, bytes);
        assert_eq!(d.eci, vec![eci]);
        assert_eq!(
            d.segments,
            vec![DataSegment {
                start: 0,
                end: bytes.len(),
                charset: Charset::Eci(eci)
            }]
        );
        assert_eq!(d.aim_id, *b"]z3");
    }
}

/// Flips `count` distinct codewords' worth of damage: one module in each of
/// `count` distinct codewords of the data region (and in check words when
/// `any` is set).
fn damage_codewords(
    grid: &mut BitMatrix,
    layout: Layout,
    count: usize,
    rng: &mut Rng,
    data_words: usize,
    any: bool,
) {
    let w = layout.word_size();
    let total = if any {
        layout.total_codewords()
    } else {
        data_words
    };
    let mut hit = Vec::new();
    while hit.len() < count {
        let k = rng.below(total);
        if !hit.contains(&k) {
            hit.push(k);
            flip_bit(grid, layout, layout.start_pad() + k * w + rng.below(w));
        }
    }
}

#[test]
fn corrects_within_the_guard() {
    let mut rng = Rng(8);
    for (layers, len) in [
        (-1, 6),
        (-2, 14),
        (-3, 30),
        (-4, 60),
        (1, 4),
        (3, 40),
        (6, 300),
        (10, 400),
        (25, 800),
    ] {
        let data = sized(&mut rng, layers, len);
        let r = rxing_encode(&data, 40, layers, None).unwrap();
        let ec = r.layout.total_codewords() - r.data_codewords;
        let max_errors = budget(ec) / 2;
        assert!(max_errors >= 1, "layers {layers}");
        for errors in [1, max_errors / 2 + 1, max_errors] {
            let mut g = r.grid.clone();
            damage_codewords(&mut g, r.layout, errors, &mut rng, r.data_codewords, true);
            let d = decode_grid(&g, false).expect("corrected");
            assert_eq!(d.data, data, "layers {layers} errors {errors}");
            assert!(d.errors_corrected as usize <= errors);
        }
    }
}

#[test]
fn never_wrong_beyond_the_guard() {
    let mut rng = Rng(9);
    let mut rejected = 0;
    let mut total = 0;
    for _ in 0..400 {
        let layers = [-1, -2, -3, -4, 1, 2, 4, 8, 12][rng.below(9)];
        let len = 4 + rng.below(30);
        let data = printable(&mut rng, len);
        let Some(r) = rxing_encode(&data, 30, layers, None) else {
            continue;
        };
        let n = r.layout.total_codewords();
        let ec = n - r.data_codewords;
        let errors = (budget(ec) / 2 + 1 + rng.below(ec + 4)).min(n);
        let mut g = r.grid.clone();
        damage_codewords(&mut g, r.layout, errors, &mut rng, r.data_codewords, true);
        total += 1;
        match decode_grid(&g, false) {
            Ok(d) => assert_eq!(d.data, data, "wrong data accepted"),
            Err(_) => rejected += 1,
        }
    }
    assert!(rejected * 10 > total * 9, "{rejected}/{total}");
}

#[test]
fn invalid_codewords_are_erasures() {
    let mut rng = Rng(10);
    for (layers, len) in [(-3, 30), (-4, 40), (4, 30), (9, 100)] {
        let data = sized(&mut rng, layers, len);
        let r = rxing_encode(&data, 60, layers, None).unwrap();
        let ec = r.layout.total_codewords() - r.data_codewords;
        let w = r.layout.word_size();
        let erasable = budget(ec).min(r.data_codewords - 1);
        assert!(erasable >= 4);
        for count in [1, erasable / 2, erasable] {
            let mut g = r.grid.clone();
            let mut hit = Vec::new();
            while hit.len() < count {
                let k = rng.below(r.data_codewords);
                if !hit.contains(&k) {
                    hit.push(k);
                    let fill = rng.below(2) == 1;
                    for j in 0..w {
                        let (x, y) =
                            spec::data_positions(r.layout)[r.layout.start_pad() + k * w + j];
                        g.set(x, y, fill);
                    }
                }
            }
            let d = decode_grid(&g, false).expect("erasures recovered");
            assert_eq!(d.data, data, "layers {layers} count {count}");
            assert!(d.erasures as usize <= count);
        }
        // One more than the guard allows is refused.
        let over = budget(ec) + 1;
        if over <= r.data_codewords {
            let mut g = r.grid.clone();
            for k in 0..over {
                for j in 0..w {
                    let (x, y) = spec::data_positions(r.layout)[r.layout.start_pad() + k * w + j];
                    g.set(x, y, false);
                }
            }
            assert!(decode_grid(&g, false).is_err());
        }
    }
}

#[test]
fn mode_message_guard() {
    let r = rxing_encode(b"MODE MESSAGE", 23, -2, None).unwrap();
    let c = (r.grid.width() / 2) as i32;
    let pos = spec::mode_positions(true);
    // One flipped bit is corrected, two are not (compact: at most 1).
    let mut g = r.grid.clone();
    let (dx, dy) = pos[5];
    let v = g.get((c + dx) as usize, (c + dy) as usize);
    g.set((c + dx) as usize, (c + dy) as usize, !v);
    let d = decode_grid(&g, false).expect("one error");
    assert_eq!(d.errors_corrected, 1);
    let (dx, dy) = pos[20];
    let v = g.get((c + dx) as usize, (c + dy) as usize);
    g.set((c + dx) as usize, (c + dy) as usize, !v);
    assert!(decode_grid(&g, false).is_err());

    // Full-range: two corrected words, not three.
    let r = rxing_encode(b"MODE MESSAGE", 23, 2, None).unwrap();
    let c = (r.grid.width() / 2) as i32;
    let pos = spec::mode_positions(false);
    let mut g = r.grid.clone();
    for (n, i) in [3usize, 17, 31].into_iter().enumerate() {
        let (dx, dy) = pos[i];
        let v = g.get((c + dx) as usize, (c + dy) as usize);
        g.set((c + dx) as usize, (c + dy) as usize, !v);
        let res = decode_grid(&g, false);
        if n < 2 {
            assert_eq!(res.expect("corrected").errors_corrected, n as u32 + 1);
        } else {
            assert!(res.is_err());
        }
    }
}

#[test]
fn mode_message_values_match_the_encoder() {
    let mut rng = Rng(11);
    for layers in [-1, -4, 1, 7, 32] {
        let r = rxing_encode(&sized(&mut rng, layers, 10), 23, layers, None).unwrap();
        let bits = read_mode_bits(&r.grid, r.layout.compact);
        assert_eq!(
            bits,
            testutil::mode_message(r.layout, r.data_codewords, None)
        );
    }
}

#[test]
fn runes() {
    for value in 0..=255u8 {
        let g = render_rune(value);
        let d = decode_grid(&g, false).expect("rune");
        assert_eq!(d.rune, Some(value));
        assert_eq!(d.data, value.to_string().into_bytes());
        assert_eq!(d.aim_id, *b"]zC");
        assert_eq!(d.layers, 0);
        assert!(d.compact);
    }
    // One corrupted mode message module is corrected, two are not.
    let mut g = render_rune(77);
    let c = 5i32;
    let pos = spec::mode_positions(true);
    let flip = |g: &mut BitMatrix, i: usize| {
        let (x, y) = ((c + pos[i].0) as usize, (c + pos[i].1) as usize);
        let v = g.get(x, y);
        g.set(x, y, !v);
    };
    flip(&mut g, 9);
    assert_eq!(decode_grid(&g, false).unwrap().rune, Some(77));
    flip(&mut g, 22);
    assert!(decode_grid(&g, false).is_err());
}

#[test]
fn rune_mask_is_not_a_codeword() {
    // A Rune must not read as a regular compact mode message, nor the other
    // way round, within the correction cap.
    for value in 0..=255u8 {
        let g = render_rune(value);
        assert!(read_mode(&g, true).is_err(), "rune {value}");
    }
}

#[test]
fn fnc1_positions_and_aim() {
    let build = |f: &dyn Fn(&mut Bits)| {
        let mut b = Bits::default();
        f(&mut b);
        let (_, grid) = render_stream(&b, 3).unwrap();
        decode_grid(&grid, false).unwrap()
    };
    // FNC1 first: P/S, FLG(0), then "01" in digit mode, GS between fields.
    let d = build(&|b| {
        b.push(0, 5).push(0, 5).push(0, 3); // P/S FLG(0)
        b.push(30, 5); // D/L
        b.push(2, 4).push(3, 4); // 0 1
        b.push(14, 4); // U/L
        b.push(0, 5).push(0, 5).push(0, 3); // GS
        b.push(2, 5); // A
    });
    assert_eq!(d.data, b"01\x1dA");
    assert_eq!(d.aim_id, *b"]z1");

    // FNC1 second position after one letter.
    let d = build(&|b| {
        b.push(2, 5); // A
        b.push(0, 5).push(0, 5).push(0, 3); // FNC1
        b.push(3, 5).push(4, 5); // B C
    });
    assert_eq!(d.data, b"ABC");
    assert_eq!(d.aim_id, *b"]z2");

    // Second position after two digits.
    let d = build(&|b| {
        b.push(30, 5); // D/L
        b.push(2, 4).push(3, 4); // 0 1
        b.push(0, 4).push(0, 5).push(0, 3); // P/S FLG(0) from digit mode
        b.push(2, 4); // 0
    });
    assert_eq!(d.data, b"010");
    assert_eq!(d.aim_id, *b"]z2");

    // FNC1 after three characters is a group separator and does not change the identifier.
    let d = build(&|b| {
        b.push(2, 5).push(3, 5).push(4, 5);
        b.push(0, 5).push(0, 5).push(0, 3);
        b.push(5, 5);
    });
    assert_eq!(d.data, b"ABC\x1dD");
    assert_eq!(d.aim_id, *b"]z0");

    // A second FNC1 after a first-position one is a group separator.
    let d = build(&|b| {
        b.push(0, 5).push(0, 5).push(0, 3);
        b.push(2, 5);
        b.push(0, 5).push(0, 5).push(0, 3);
        b.push(3, 5);
    });
    assert_eq!(d.data, b"A\x1dB");
    assert_eq!(d.aim_id, *b"]z1");
}

#[test]
fn eci_forms() {
    let build = |f: &dyn Fn(&mut Bits)| {
        let mut b = Bits::default();
        f(&mut b);
        let (_, grid) = render_stream(&b, 3).unwrap();
        decode_grid(&grid, false).unwrap()
    };
    // ECI 3 (one digit) before the text, ECI 26 (two digits) in the middle,
    // ECI 123456 (six digits) at the end, with FNC1 first for the identifier.
    let d = build(&|b| {
        b.push(0, 5).push(0, 5).push(0, 3); // FNC1 first
        b.push(0, 5).push(0, 5).push(1, 3).push(5, 4); // ECI 3
        b.push(2, 5).push(3, 5); // A B
        b.push(0, 5).push(0, 5).push(2, 3).push(4, 4).push(8, 4); // ECI 26
        b.push(4, 5); // C
        b.push(0, 5).push(0, 5).push(6, 3);
        for d in [1, 2, 3, 4, 5, 6] {
            b.push(d + 2, 4);
        }
        b.push(5, 5); // D
    });
    assert_eq!(d.data, b"ABCD");
    assert_eq!(d.eci, vec![3, 26, 123456]);
    assert_eq!(d.aim_id, *b"]z4");
    assert_eq!(
        d.segments,
        vec![
            DataSegment {
                start: 0,
                end: 2,
                charset: Charset::Eci(3)
            },
            DataSegment {
                start: 2,
                end: 3,
                charset: Charset::Eci(26)
            },
            DataSegment {
                start: 3,
                end: 4,
                charset: Charset::Eci(123456)
            },
        ]
    );
}

#[test]
fn stuffing_runs() {
    for byte in [0x00u8, 0xFF, 0x80, 0x7F] {
        for len in [1usize, 7, 40, 100] {
            let data = vec![byte; len];
            let r = rxing_encode(&data, 23, 0, None).unwrap();
            assert_eq!(decode(&r).data, data, "byte {byte:#x} len {len}");
        }
    }
    let data: Vec<u8> = (0..300)
        .map(|i| if i % 37 < 20 { 0 } else { 0xFF })
        .collect();
    let r = rxing_encode(&data, 23, 0, None).unwrap();
    assert_eq!(decode(&r).data, data);
}

#[test]
fn large_symbol_with_many_errors() {
    let mut rng = Rng(12);
    let data = printable(&mut rng, 1200);
    let r = rxing_encode(&data, 25, 32, None).unwrap();
    assert_eq!(r.layout.layers, 32);
    let ec = r.layout.total_codewords() - r.data_codewords;
    let errors = budget(ec) / 2;
    assert!(errors > 100);
    let mut g = r.grid.clone();
    damage_codewords(&mut g, r.layout, errors, &mut rng, r.data_codewords, true);
    let d = decode_grid(&g, false).expect("corrected");
    assert_eq!(d.data, data);
    assert_eq!(d.errors_corrected as usize, errors);
}

#[test]
fn random_data_behind_a_valid_finder_is_rejected() {
    let mut rng = Rng(13);
    let mut accepted = 0;
    for round in 0..3000 {
        let layers: i32 = [-1, -2, -3, -4, 1, 2, 3][round % 7];
        let layout = Layout {
            compact: layers < 0,
            layers: layers.unsigned_abs() as usize,
        };
        // Valid mode message and finder, random everything else.
        let words = vec![0x2au16 & ((1 << layout.word_size()) - 1); 3 + rng.below(8)];
        let mut g = render(layout, &words);
        let w = layout.word_size();
        for (i, &(x, y)) in spec::data_positions(layout).iter().enumerate() {
            if i >= layout.start_pad() + words.len() * w || rng.below(2) == 0 {
                g.set(x, y, rng.below(2) == 1);
            }
        }
        if decode_grid(&g, false).is_ok() {
            accepted += 1;
        }
    }
    assert_eq!(accepted, 0);
}

#[test]
fn blank_and_full_data_areas_are_rejected() {
    for fill in [false, true] {
        let layout = Layout {
            compact: false,
            layers: 3,
        };
        let mut g = render(layout, &[5, 6, 7, 8]);
        for &(x, y) in &spec::data_positions(layout) {
            g.set(x, y, fill);
        }
        assert!(decode_grid(&g, false).is_err());
    }
}

#[test]
fn non_square_and_oversized_grids() {
    assert_eq!(
        decode_grid(&BitMatrix::new(15, 19), false),
        Err(Reject::Sampling)
    );
    assert_eq!(
        decode_grid(&BitMatrix::new(155, 155), false),
        Err(Reject::Sampling)
    );
    assert_eq!(
        decode_grid(&BitMatrix::new(21, 21), false),
        Err(Reject::NoPattern)
    );
    assert_eq!(
        decode_grid(&BitMatrix::new(0, 0), false),
        Err(Reject::NoPattern)
    );
}
