use super::encodation::{pad_value, parse};
use super::*;
use breader_core::Charset;
use datamatrix::{DataMatrix, DataMatrixBuilder, EncodationType, SymbolList};
use std::collections::BTreeSet;

// ---- helpers -----------------------------------------------------------

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
    fn byte(&mut self) -> u8 {
        (self.next() >> 24) as u8
    }
}

/// Data codewords followed by the standard pad sequence up to the capacity.
fn pad(size: &SymbolSize, stream: &[u8]) -> Vec<u8> {
    let cap = usize::from(size.data_codewords);
    assert!(
        stream.len() <= cap,
        "stream too long for {}x{}",
        size.rows,
        size.cols
    );
    let mut out = stream.to_vec();
    if out.len() < cap {
        out.push(129);
    }
    while out.len() < cap {
        out.push(pad_value(out.len() + 1));
    }
    out
}

/// All codewords (data, then interleaved error correction).
fn with_ec(size: &SymbolSize, data: &[u8]) -> Vec<u8> {
    assert_eq!(data.len(), usize::from(size.data_codewords));
    let blocks = usize::from(size.blocks);
    let ec = usize::from(size.ec_codewords) / blocks;
    let mut out = data.to_vec();
    out.resize(size.total_codewords(), 0);
    for b in 0..blocks {
        let block: Vec<u8> = data.iter().copied().skip(b).step_by(blocks).collect();
        let mut check = vec![0u8; ec];
        DM_FIELD.encode(&block, ec, &mut check).unwrap();
        for (i, &c) in check.iter().enumerate() {
            out[data.len() + i * blocks + b] = c;
        }
    }
    out
}

/// Draws the symbol: finder, clock, alignment patterns and the codewords.
fn paint(size: &SymbolSize, codewords: &[u8]) -> BitMatrix {
    let (rows, cols) = (usize::from(size.rows), usize::from(size.cols));
    let mut g = BitMatrix::new(cols, rows);
    let (rh, rw) = (
        usize::from(size.region_size.0),
        usize::from(size.region_size.1),
    );
    for vi in 0..usize::from(size.regions.0) {
        for hi in 0..usize::from(size.regions.1) {
            let (y0, x0) = (vi * (rh + 2), hi * (rw + 2));
            for dy in 0..rh + 2 {
                for dx in 0..rw + 2 {
                    let dark = dx == 0
                        || dy == rh + 1
                        || (dy == 0 && dx % 2 == 0)
                        || (dx == rw + 1 && dy % 2 == 1);
                    g.set(x0 + dx, y0 + dy, dark);
                }
            }
        }
    }
    let p = place::placement(size).unwrap();
    for (w, &cw) in p.words.iter().zip(codewords) {
        for (i, &(r, c)) in w.iter().enumerate() {
            let (y, x) = size.symbol_position(usize::from(r), usize::from(c));
            g.set(x, y, cw >> (7 - i) & 1 == 1);
        }
    }
    if let [_, _, _, last] = p.unused[..] {
        let (y, x) = size.symbol_position(usize::from(last.0), usize::from(last.1));
        g.set(x, y, true);
        g.set(x - 1, y - 1, true);
    }
    g
}

fn smallest(stream_len: usize) -> &'static SymbolSize {
    spec::sizes()
        .iter()
        .filter(|s| !s.dmre && usize::from(s.data_codewords) >= stream_len && s.rows == s.cols)
        .min_by_key(|s| s.data_codewords)
        .unwrap()
}

fn grid_for(stream: &[u8]) -> (BitMatrix, &'static SymbolSize) {
    let size = smallest(stream.len());
    (paint(size, &with_ec(size, &pad(size, stream))), size)
}

fn enc_grid(dm: &DataMatrix) -> BitMatrix {
    let bm = dm.bitmap();
    let w = bm.width();
    let mut g = BitMatrix::new(w, bm.height());
    for (i, &b) in bm.bits().iter().enumerate() {
        if b {
            g.set(i % w, i / w, true);
        }
    }
    g
}

fn same(a: &BitMatrix, b: &BitMatrix) -> bool {
    a.width() == b.width()
        && a.height() == b.height()
        && (0..a.height()).all(|y| (0..a.width()).all(|x| a.get(x, y) == b.get(x, y)))
}

/// Reflection across the diagonal through the bottom-left corner.
fn mirror(g: &BitMatrix) -> BitMatrix {
    let (w, h) = (g.width(), g.height());
    let mut m = BitMatrix::new(h, w);
    for r in 0..w {
        for c in 0..h {
            if g.get(w - 1 - r, h - 1 - c) {
                m.set(c, r, true);
            }
        }
    }
    m
}

fn size_of(dm: &DataMatrix) -> &'static SymbolSize {
    let bm = dm.bitmap();
    spec::size(bm.height() as u16, bm.width() as u16).unwrap()
}

fn all_enc_sizes() -> Vec<datamatrix::SymbolSize> {
    SymbolList::all().iter().collect()
}

// ---- encoder cross-checks ------------------------------------------------

/// Largest encodable payload of `kind` for the symbol, or `None`.
fn fit(
    s: datamatrix::SymbolSize,
    rng: &mut Rng,
    cap: usize,
    gen_byte: impl Fn(&mut Rng) -> u8,
    per_cw: usize,
) -> Option<(Vec<u8>, DataMatrix)> {
    let mut len = (cap * per_cw).max(1);
    for _ in 0..40 {
        let data: Vec<u8> = (0..len).map(|_| gen_byte(rng)).collect();
        if let Ok(dm) = DataMatrixBuilder::new()
            .with_symbol_list(s)
            .with_macros(false)
            .encode(&data)
        {
            return Some((data, dm));
        }
        len = len * 9 / 10;
        if len == 0 {
            return None;
        }
    }
    None
}

#[test]
fn symbols_match_the_encoder_for_every_size() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let sizes = all_enc_sizes();
    assert_eq!(sizes.len(), spec::SIZE_COUNT);
    let mut seen = BTreeSet::new();
    for s in sizes {
        let tiny = DataMatrix::encode(b"A", s).unwrap();
        let size = size_of(&tiny);
        seen.insert((size.rows, size.cols));
        assert_eq!(
            tiny.data_codewords().len(),
            usize::from(size.data_codewords)
        );
        assert_eq!(tiny.codewords().len(), size.total_codewords());
        let (_, dm) = fit(
            s,
            &mut rng,
            usize::from(size.data_codewords),
            |r| 65 + (r.byte() % 26),
            1,
        )
        .unwrap();
        let ours = with_ec(size, dm.data_codewords());
        assert_eq!(ours, dm.codewords(), "ec {}x{}", size.rows, size.cols);
        assert!(
            same(&paint(size, &ours), &enc_grid(&dm)),
            "grid {}x{}",
            size.rows,
            size.cols
        );
    }
    assert_eq!(seen.len(), spec::SIZE_COUNT);
}

fn digits(r: &mut Rng) -> u8 {
    b'0' + r.byte() % 10
}
fn upper_digits(r: &mut Rng) -> u8 {
    let v = r.byte() % 36;
    if v < 10 { b'0' + v } else { b'A' + v - 10 }
}
fn lower(r: &mut Rng) -> u8 {
    b'a' + r.byte() % 26
}
fn x12(r: &mut Rng) -> u8 {
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789*> \r"[r.below(39)]
}
fn edifact_chars(r: &mut Rng) -> u8 {
    32 + r.byte() % 63
}
fn binary(r: &mut Rng) -> u8 {
    r.byte()
}
fn ascii_mix(r: &mut Rng) -> u8 {
    r.byte() % 127 + 1
}

#[test]
fn round_trip_every_size_and_encodation() {
    type Gen = fn(&mut Rng) -> u8;
    let kinds: [(&str, Gen, usize); 7] = [
        ("digits", digits, 2),
        ("upper", upper_digits, 1),
        ("lower", lower, 1),
        ("x12", x12, 1),
        ("edifact", edifact_chars, 1),
        ("binary", binary, 1),
        ("ascii", ascii_mix, 1),
    ];
    let mut rng = Rng(0xdead_beef_0bad_f00d);
    let mut modes = BTreeSet::new();
    let mut checked = 0;
    for s in all_enc_sizes() {
        let tiny = DataMatrix::encode(b"A", s).unwrap();
        let size = size_of(&tiny);
        let cap = usize::from(size.data_codewords);
        for (name, gen_byte, per_cw) in kinds {
            let Some((data, dm)) = fit(s, &mut rng, cap, gen_byte, per_cw) else {
                continue;
            };
            if let Some(plan) = datamatrix::data::encodation_plan(
                &data,
                &SymbolList::from(s),
                datamatrix::EncodationType::all(),
            ) {
                modes.extend(plan.iter().map(|&(_, m)| format!("{m:?}")));
            }
            let grid = enc_grid(&dm);
            let d = decode_grid(&grid).unwrap_or_else(|e| {
                panic!(
                    "{name} {}x{}: {e:?} {:?}",
                    size.rows,
                    size.cols,
                    dm.data_codewords()
                )
            });
            assert_eq!(d.data, data, "{name} {}x{}", size.rows, size.cols);
            assert_eq!(&d.aim_id, b"]d1");
            assert_eq!((d.rows, d.cols), (size.rows, size.cols));
            assert_eq!(d.errors_corrected, 0);
            assert!(!d.mirrored && d.eci.is_empty() && d.structured_append.is_none());
            assert_eq!(
                d.segments,
                vec![DataSegment {
                    start: 0,
                    end: data.len(),
                    charset: Charset::Latin1
                }]
            );
            checked += 1;
        }
    }
    assert!(checked > 250, "{checked}");
    for m in ["Ascii", "C40", "Text", "X12", "Edifact", "Base256"] {
        assert!(modes.contains(m), "encoder never chose {m}: {modes:?}");
    }
}

#[test]
fn encoder_gs1_eci_and_macros() {
    let list = SymbolList::default();
    let gs1 = b"01034531200000111719112510ABCD1234\x1d2110";
    let dm = DataMatrix::encode_gs1(gs1, list.clone()).unwrap();
    let d = decode_grid(&enc_grid(&dm)).unwrap();
    assert_eq!(d.data, gs1);
    assert_eq!(&d.aim_id, b"]d2");

    let dm = DataMatrixBuilder::new()
        .with_fnc1_start(true)
        .with_encodation_types(EncodationType::C40 | EncodationType::Ascii)
        .encode(b"01034531200000111719112510ABCD1234")
        .unwrap();
    let d = decode_grid(&enc_grid(&dm)).unwrap();
    assert_eq!(d.data, b"01034531200000111719112510ABCD1234");
    assert_eq!(&d.aim_id, b"]d2");

    let text = "h\u{e9}llo \u{20ac} \u{4e16}\u{754c}";
    let dm = DataMatrixBuilder::new().encode_str(text).unwrap();
    let d = decode_grid(&enc_grid(&dm)).unwrap();
    assert_eq!(d.data, text.as_bytes());
    assert_eq!(d.eci, vec![26]);
    assert_eq!(&d.aim_id, b"]d4");
    assert_eq!(
        d.segments,
        vec![DataSegment {
            start: 0,
            end: text.len(),
            charset: Charset::Eci(26)
        }]
    );

    let dm = DataMatrixBuilder::new().encode_str("h\u{e9}llo").unwrap();
    let d = decode_grid(&enc_grid(&dm)).unwrap();
    assert_eq!(d.data, b"h\xe9llo");
    assert!(d.eci.is_empty());

    for (head, name) in [("05", "05"), ("06", "06")] {
        let payload = format!("[)>\x1e{head}\x1dABC12345\x1e\x04");
        let dm = DataMatrixBuilder::new()
            .with_macros(true)
            .encode(payload.as_bytes())
            .unwrap();
        let cw = dm.data_codewords();
        assert!(cw[0] == 236 || cw[0] == 237, "encoder skipped macro {name}");
        let d = decode_grid(&enc_grid(&dm)).unwrap();
        assert_eq!(d.data, payload.as_bytes());
        assert_eq!(
            d.segments.first().map(|s| (s.start, s.end, s.charset)),
            Some((0, 7, Charset::Ascii))
        );
    }
}

// ---- hand-built codeword streams -------------------------------------------

fn parsed(stream: &[u8]) -> Result<super::encodation::Parsed, Reject> {
    parse(stream, true)
}

fn ascii(s: &str) -> Vec<u8> {
    s.bytes().map(|b| b + 1).collect()
}

/// Triples of C40/Text/X12 values to codewords.
fn triples(values: &[u8]) -> Vec<u8> {
    assert_eq!(values.len() % 3, 0);
    values
        .chunks(3)
        .flat_map(|t| {
            let v = 1600 * u32::from(t[0]) + 40 * u32::from(t[1]) + u32::from(t[2]) + 1;
            [(v / 256) as u8, (v % 256) as u8]
        })
        .collect()
}

fn base256(bytes: &[u8], start: usize, zero_form: bool) -> Vec<u8> {
    let mask = |pos: usize| ((149 * pos) % 255 + 1) as u8;
    let mut field = Vec::new();
    if zero_form {
        field.push(0);
    } else if bytes.len() <= 249 {
        field.push(bytes.len() as u8);
    } else {
        field.push((bytes.len() / 250 + 249) as u8);
        field.push((bytes.len() % 250) as u8);
    }
    field.extend_from_slice(bytes);
    let mut out = vec![231u8];
    for (i, &b) in field.iter().enumerate() {
        out.push(b.wrapping_add(mask(start + 2 + i)));
    }
    out
}

fn eci_cws(n: u32) -> Vec<u8> {
    match n {
        0..=126 => vec![241, n as u8 + 1],
        127..=16382 => vec![
            241,
            ((n - 127) / 254 + 128) as u8,
            ((n - 127) % 254 + 1) as u8,
        ],
        _ => {
            let m = n - 16383;
            vec![
                241,
                (m / 64516 + 192) as u8,
                (m / 254 % 254 + 1) as u8,
                (m % 254 + 1) as u8,
            ]
        }
    }
}

fn edifact_cws(chars: &[u8], unlatch: bool) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::new();
    let mut push = |v: u8| (0..6).rev().for_each(|k| bits.push(v >> k & 1 == 1));
    for &c in chars {
        push(c & 0x3f);
    }
    if unlatch {
        push(31);
    }
    let mut out = vec![240u8];
    out.extend(bits.chunks(8).map(|c| {
        c.iter()
            .enumerate()
            .fold(0u8, |a, (i, &b)| a | u8::from(b) << (7 - i))
    }));
    out
}

#[test]
fn ascii_basics() {
    let mut s = ascii("Hi");
    s.extend([130 + 42, 130, 229, 235, 98, 235, 1]);
    let p = parsed(&s).unwrap();
    assert_eq!(p.data, b"Hi420099\xe1\x80");
    assert_eq!(&p.aim_id, b"]d1");
    // Upper shift needs an ASCII character after it.
    assert!(parsed(&[235]).is_err());
    assert!(parsed(&[235, 129]).is_err());
    assert!(parsed(&[235, 130]).is_err());
}

#[test]
fn padding_is_validated() {
    let size = smallest(10);
    let stream = pad(size, &ascii("ABC"));
    assert_eq!(stream[3], 129);
    assert_eq!(parsed(&stream).unwrap().data, b"ABC");
    let mut bad = stream.clone();
    *bad.last_mut().unwrap() ^= 1;
    assert!(parsed(&bad).is_err());
    let mut bad = stream.clone();
    bad[4] = 129;
    assert!(parsed(&bad).is_err());
    // Data after the pad.
    let mut bad = stream.clone();
    bad[5] = 66;
    assert!(parsed(&bad).is_err());
    // Only padding is not a message.
    assert!(parsed(&pad(size, &[])).is_err());
    assert!(parsed(&[]).is_err());
}

/// Data codewords with a first pad and then `fill` instead of the standard sequence.
fn pad_with(size: &SymbolSize, stream: &[u8], fill: u8) -> Vec<u8> {
    let mut out = stream.to_vec();
    out.push(129);
    out.resize(usize::from(size.data_codewords), fill);
    out
}

#[test]
fn nonstandard_pads_pass_only_without_corrections_on_large_blocks() {
    let large = spec::sizes()
        .iter()
        .find(|s| !s.dmre && usize::from(s.ec_codewords) / usize::from(s.blocks) > SMALL_BLOCK_EC)
        .unwrap();
    let stream = ascii("ABC");
    let grid = paint(large, &with_ec(large, &pad_with(large, &stream, 129)));
    let d = decode_grid(&grid).unwrap();
    assert_eq!((d.data.as_slice(), d.errors_corrected), (&b"ABC"[..], 0));
    // One corrected module keeps the pad check on.
    let mut bad = grid.clone();
    let (y, x) = large.symbol_position(1, 1);
    bad.set(x, y, !bad.get(x, y));
    assert!(decode_grid(&bad).is_err());
    // The standard pads still decode after a correction.
    let std_grid = paint(large, &with_ec(large, &pad(large, &stream)));
    let mut hit = std_grid.clone();
    hit.set(x, y, !hit.get(x, y));
    assert_eq!(decode_grid(&hit).unwrap().errors_corrected, 1);
    // Small blocks keep the check even when clean.
    let small = smallest(6);
    assert!(usize::from(small.ec_codewords) / usize::from(small.blocks) <= SMALL_BLOCK_EC);
    let grid = paint(small, &with_ec(small, &pad_with(small, &stream, 129)));
    assert!(decode_grid(&grid).is_err());
}

#[test]
fn text_mode_shift_sets_and_upper_shift() {
    let mut s = vec![239];
    s.extend(triples(&[
        14, 2, 2, 5, 2, 27, 2, 30, 2, 0, 1, 30, 14, 3, 3, 0, 9, 1, 0, 3, 3, 3, 3, 3,
    ]));
    s.push(254);
    s.extend(ascii("Z"));
    let p = parsed(&s).unwrap();
    assert_eq!(p.data, b"aB1{~`\xe1  \t!     Z");
}

#[test]
fn c40_mode_sets() {
    let mut s = vec![230];
    s.extend(triples(&[14, 1, 13, 1, 15, 2, 2, 0, 10, 3, 4, 5]));
    s.push(254);
    s.extend(ascii("z"));
    assert_eq!(parsed(&s).unwrap().data, b"A.:b\n 01z");

    // FNC1 inside C40 is a GS; shift 2 value 27.
    let mut s = vec![230];
    s.extend(triples(&[14, 1, 27]));
    assert_eq!(parsed(&s).unwrap().data, b"A\x1d");

    // Upper shift followed by a shift 3 character.
    let mut s = vec![230];
    s.extend(triples(&[1, 30, 2, 2, 14, 15]));
    assert_eq!(parsed(&s).unwrap().data, b"\xe2AB");

    // All printable ASCII round trips through the three sets.
    let mut vals = Vec::new();
    for c in 0u8..128 {
        match c {
            b' ' => vals.push(3),
            b'0'..=b'9' => vals.push(c - b'0' + 4),
            b'A'..=b'Z' => vals.push(c - b'A' + 14),
            0..=31 => vals.extend([0, c]),
            33..=47 => vals.extend([1, c - 33]),
            58..=64 => vals.extend([1, c - 58 + 15]),
            91..=95 => vals.extend([1, c - 91 + 22]),
            _ => vals.extend([
                2,
                if c == 96 {
                    0
                } else if c >= 123 {
                    c - 123 + 27
                } else {
                    c - 96
                },
            ]),
        }
    }
    while vals.len() % 3 != 0 {
        vals.push(0);
    }
    let mut s = vec![230];
    s.extend(triples(&vals));
    let p = parsed(&s).unwrap();
    assert_eq!(p.data, (0u8..128).collect::<Vec<_>>());
}

#[test]
fn text_mode_covers_ascii() {
    let mut vals = Vec::new();
    for c in 0u8..128 {
        match c {
            b' ' => vals.push(3),
            b'0'..=b'9' => vals.push(c - b'0' + 4),
            b'a'..=b'z' => vals.push(c - b'a' + 14),
            0..=31 => vals.extend([0, c]),
            33..=47 => vals.extend([1, c - 33]),
            58..=64 => vals.extend([1, c - 58 + 15]),
            91..=95 => vals.extend([1, c - 91 + 22]),
            _ => vals.extend([
                2,
                if c == 96 {
                    0
                } else if c >= 123 {
                    c - 123 + 27
                } else {
                    c - 64
                },
            ]),
        }
    }
    while vals.len() % 3 != 0 {
        vals.push(0);
    }
    let mut s = vec![239];
    s.extend(triples(&vals));
    assert_eq!(parsed(&s).unwrap().data, (0u8..128).collect::<Vec<_>>());
}

#[test]
fn triple_mode_endings() {
    // One codeword left after a triple is plain ASCII.
    let mut s = vec![230];
    s.extend(triples(&[14, 15, 16]));
    s.extend(ascii("x"));
    assert_eq!(parsed(&s).unwrap().data, b"ABCx");
    // Trailing Shift 1 acts as padding.
    let mut s = vec![230];
    s.extend(triples(&[14, 15, 0]));
    assert_eq!(parsed(&s).unwrap().data, b"AB");
    // So does any other shift value some encoders use, but not a dangling Upper Shift.
    for last in [1u8, 2] {
        let mut s = vec![230];
        s.extend(triples(&[14, 15, last]));
        assert_eq!(parsed(&s).unwrap().data, b"AB", "shift {last}");
    }
    let mut s = vec![230];
    s.extend(triples(&[14, 1, 30]));
    assert!(parsed(&s).is_err());
    // Unreachable triple values.
    assert!(parsed(&[230, 255, 255]).is_err());
    assert!(parsed(&[230, 0, 0]).is_err());
    assert!(parsed(&[230, 250, 1]).is_err());
    // C40 shift 2 reserved values.
    for v in [28u8, 29, 31] {
        let mut s = vec![230];
        s.extend(triples(&[1, v, 3]));
        assert!(parsed(&s).is_err());
    }
}

#[test]
fn x12_mode() {
    let mut s = vec![238];
    s.extend(triples(&[14, 15, 1, 2, 0, 3, 13, 4, 5]));
    s.push(254);
    s.extend(ascii("x"));
    assert_eq!(parsed(&s).unwrap().data, b"AB*>\r 901x");
    let mut s = vec![238];
    s.extend(triples(&[40, 0, 0]));
    assert!(parsed(&s).is_err());
}

#[test]
fn edifact_mode() {
    let chars = b"ABC+1 UNH:";
    let mut s = edifact_cws(chars, true);
    s.extend(ascii("x"));
    assert_eq!(parsed(&s).unwrap().data, b"ABC+1 UNH:x");
    // Without unlatch when the symbol ends: 4 chars = 3 codewords.
    assert_eq!(parsed(&edifact_cws(b"ABCD", false)).unwrap().data, b"ABCD");
    // Unlatch only, every length mod 4.
    for n in 1..=9 {
        let c: Vec<u8> = (0..n).map(|i| 64 + i as u8).collect();
        let mut s = edifact_cws(&c, true);
        s.extend(ascii("."));
        let mut want = c.clone();
        want.push(b'.');
        assert_eq!(parsed(&s).unwrap().data, want, "n={n}");
    }
    // Nonzero padding bits after the unlatch.
    let mut s = edifact_cws(b"AB", true);
    *s.last_mut().unwrap() |= 1;
    assert!(parsed(&s).is_err());
    // All 63 characters.
    let all: Vec<u8> = (32u8..95).collect();
    let mut s = edifact_cws(&all, true);
    s.extend(ascii("."));
    let mut want = all.clone();
    want.push(b'.');
    assert_eq!(parsed(&s).unwrap().data, want);
}

#[test]
fn base256_mode() {
    let mut rng = Rng(77);
    for (len, zero) in [
        (1, false),
        (5, false),
        (249, false),
        (250, false),
        (251, false),
        (1500, false),
        (7, true),
    ] {
        let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        for start in [0usize, 3] {
            let mut s = ascii(&"ab"[..start.min(2)]);
            s.extend((s.len()..start).map(|_| 66));
            let at = s.len();
            s.extend(base256(&bytes, at, zero));
            let p = parsed(&s).unwrap();
            assert_eq!(&p.data[start..], &bytes[..], "len {len} start {start}");
        }
    }
    // After the field the mode is ASCII again.
    let mut s = base256(&[1, 2, 3], 0, false);
    s.extend(ascii("k"));
    assert_eq!(parsed(&s).unwrap().data, b"\x01\x02\x03k");
    // Length beyond the stream, truncated headers, empty field.
    let mut s = base256(&[1, 2, 3], 0, false);
    s.pop();
    assert!(parsed(&s).is_err());
    assert!(parsed(&[231]).is_err());
    assert!(parsed(&base256(&[], 0, true)).is_err());
}

#[test]
fn fnc1_positions() {
    let mut s = vec![232];
    s.extend(ascii("01"));
    s.push(232);
    s.extend(ascii("2"));
    let p = parsed(&s).unwrap();
    assert_eq!(p.data, b"01\x1d2");
    assert_eq!(&p.aim_id, b"]d2");

    // Second position after a letter and after a digit pair.
    let mut s = ascii("A");
    s.push(232);
    s.extend(ascii("bc"));
    let p = parsed(&s).unwrap();
    assert_eq!((&p.data[..], &p.aim_id), (&b"Abc"[..], b"]d3"));
    let s = [130 + 12, 232, 100, 232, 100];
    let p = parsed(&s).unwrap();
    assert_eq!((&p.data[..], &p.aim_id), (&b"12c\x1dc"[..], b"]d3"));

    // After a single digit or a longer prefix FNC1 is a GS.
    for prefix in ["1", "AB"] {
        let mut s = ascii(prefix);
        s.push(232);
        s.extend(ascii("c"));
        let p = parsed(&s).unwrap();
        assert_eq!(p.data, [prefix.as_bytes(), b"\x1dc"].concat());
        assert_eq!(&p.aim_id, b"]d1");
    }

    // First position with ECI before it and with GS1 in C40 afterwards.
    let mut s = eci_cws(26);
    s.push(232);
    s.push(230);
    s.extend(triples(&[14, 1, 27]));
    let p = parsed(&s).unwrap();
    assert_eq!(
        (&p.data[..], &p.aim_id, &p.eci[..]),
        (&b"A\x1d"[..], b"]d5", &[26][..])
    );

    // ECI, then second position FNC1.
    let mut s = eci_cws(3);
    s.extend(ascii("A"));
    s.push(232);
    s.extend(ascii("b"));
    assert_eq!(&parsed(&s).unwrap().aim_id, b"]d6");
}

#[test]
fn eci_forms_and_segments() {
    for n in [
        0u32, 1, 3, 26, 126, 127, 128, 254, 16382, 16383, 16384, 50000, 899, 999_999,
    ] {
        let mut s = ascii("a");
        s.extend(eci_cws(n));
        s.extend(ascii("b"));
        let p = parsed(&s).unwrap();
        assert_eq!(p.eci, vec![n], "eci {n}");
        assert_eq!(
            p.segments,
            vec![
                DataSegment {
                    start: 0,
                    end: 1,
                    charset: Charset::Latin1
                },
                DataSegment {
                    start: 1,
                    end: 2,
                    charset: Charset::Eci(n)
                },
            ]
        );
        assert_eq!(&p.aim_id, b"]d4");
    }
    // Two ECIs, the first one empty.
    let mut s = eci_cws(3);
    s.extend(eci_cws(26));
    s.extend(ascii("a"));
    s.extend(eci_cws(4));
    s.extend(ascii("b"));
    let p = parsed(&s).unwrap();
    assert_eq!(p.eci, vec![3, 26, 4]);
    assert_eq!(p.segments.len(), 2);
    assert_eq!(p.segments[0].charset, Charset::Eci(26));
    assert_eq!(p.segments[1].charset, Charset::Eci(4));
    // Out of range and truncated.
    assert!(parsed(&[241, 207, 254, 254, 66]).is_err());
    assert!(parsed(&[241, 208, 1, 1, 66]).is_err());
    assert!(parsed(&[241, 129, 255, 66]).is_err());
    assert!(parsed(&[241, 129, 0, 66]).is_err());
    assert!(parsed(&[241, 0, 66]).is_err());
    assert!(parsed(&[66, 241]).is_err());
    assert!(parsed(&[66, 241, 192, 1]).is_err());
}

#[test]
fn structured_append() {
    let ind = |pos: u8, total: u8| ((pos - 1) << 4) | (17 - total);
    let mut s = vec![233, ind(2, 5), 7, 9];
    s.extend(ascii("hi"));
    let p = parsed(&s).unwrap();
    assert_eq!(p.structured_append, Some((2, 5, 0x0709)));
    assert_eq!(p.data, b"hi");
    assert_eq!(&p.aim_id, b"]d1");
    let mut s = vec![233, ind(16, 16), 254, 254];
    s.extend(ascii("x"));
    assert_eq!(
        parsed(&s).unwrap().structured_append,
        Some((16, 16, 0xfefe))
    );
    // SA followed by FNC1 in first position and ECI.
    let mut s = vec![233, ind(1, 2), 1, 1, 232];
    s.extend(ascii("x"));
    let p = parsed(&s).unwrap();
    assert_eq!(&p.aim_id, b"]d2");
    // Invalid headers.
    for bad in [
        vec![233, ind(5, 5) | 0x10, 7, 9, 66],
        vec![233, 0x10, 7, 9, 66],
        vec![233, 0x1c, 0, 9, 66],
        vec![233, 0x1c, 7, 255, 66],
        vec![233, 0x1c, 7],
        vec![66, 233, 0x1c, 7, 9, 66],
    ] {
        assert!(parsed(&bad).is_err(), "{bad:?}");
    }
    // Position 6 of 5 is invalid.
    assert!(parsed(&[233, (5 << 4) | 12, 7, 9, 66]).is_err());
}

#[test]
fn macros_and_reader_programming() {
    let mut s = vec![236];
    s.extend(ascii("AB"));
    let p = parsed(&s).unwrap();
    assert_eq!(p.data, b"[)>\x1e05\x1dAB\x1e\x04");
    assert_eq!(
        p.segments,
        vec![
            DataSegment {
                start: 0,
                end: 7,
                charset: Charset::Ascii
            },
            DataSegment {
                start: 7,
                end: 9,
                charset: Charset::Latin1
            },
            DataSegment {
                start: 9,
                end: 11,
                charset: Charset::Ascii
            },
        ]
    );
    let mut s = vec![237];
    s.extend(ascii("AB"));
    assert_eq!(parsed(&s).unwrap().data, b"[)>\x1e06\x1dAB\x1e\x04");
    // A macro must come first; macro with nothing else is still a message.
    assert!(parsed(&[66, 236]).is_err());
    assert_eq!(parsed(&[236]).unwrap().data, b"[)>\x1e05\x1d\x1e\x04");
    assert!(parsed(&[236, 237, 66]).is_err());
    assert!(parsed(&[232, 236, 66]).is_err());
    // Reader programming is refused.
    assert!(parsed(&[234, 66]).is_err());
    assert!(parsed(&[66, 234, 66]).is_err());
    // Reserved codewords.
    for cw in [242u8, 250, 254, 255, 0] {
        assert!(parsed(&[66, cw, 66]).is_err(), "{cw}");
    }
}

#[test]
fn hand_built_streams_through_symbols() {
    let mut text = vec![239];
    text.extend(triples(&[14, 2, 2, 5, 2, 27, 2, 30, 2, 0, 1, 30, 14, 3, 3]));
    text.push(254);
    let mut eci = eci_cws(26);
    eci.extend(ascii("x"));
    eci.push(235);
    eci.push(2);
    let streams: Vec<(Vec<u8>, &[u8])> = vec![
        (text, b"aB1{~`\xe1  "),
        (eci, b"x\x81"),
        (vec![236, 66, 67], b"[)>\x1e05\x1dAB\x1e\x04"),
    ];
    for (stream, want) in streams {
        let (grid, _) = grid_for(&stream);
        let d = decode_grid(&grid).unwrap();
        assert_eq!(d.data, want);
    }
}

// ---- error correction ----------------------------------------------------

/// Flips `count` distinct codewords of block `b` (all bits of at least one
/// module each); returns the number flipped.
fn corrupt_block(
    grid: &mut BitMatrix,
    size: &SymbolSize,
    b: usize,
    count: usize,
    rng: &mut Rng,
) -> usize {
    let blocks = usize::from(size.blocks);
    let p = place::placement(size).unwrap();
    let mut idx: Vec<usize> = (0..size.total_codewords())
        .filter(|&k| {
            let data = usize::from(size.data_codewords);
            if k < data {
                k % blocks == b
            } else {
                (k - data) % blocks == b
            }
        })
        .collect();
    let mut done = 0;
    while done < count && !idx.is_empty() {
        let k = idx.swap_remove(rng.below(idx.len()));
        let mut mask = rng.byte();
        if mask == 0 {
            mask = 0x81;
        }
        for (i, &(r, c)) in p.words[k].iter().enumerate() {
            if mask >> (7 - i) & 1 == 1 {
                let (y, x) = size.symbol_position(usize::from(r), usize::from(c));
                grid.set(x, y, !grid.get(x, y));
            }
        }
        done += 1;
    }
    done
}

fn sample_payload(size: &SymbolSize, rng: &mut Rng) -> (Vec<u8>, BitMatrix) {
    let sizes = all_enc_sizes();
    let s = sizes
        .into_iter()
        .find(|&s| {
            let t = DataMatrix::encode(b"A", s).unwrap();
            let bm = t.bitmap();
            bm.width() == usize::from(size.cols) && bm.height() == usize::from(size.rows)
        })
        .unwrap();
    let (data, dm) = fit(s, rng, usize::from(size.data_codewords), upper_digits, 1).unwrap();
    (data, enc_grid(&dm))
}

#[test]
fn corrects_up_to_capacity_in_every_block() {
    let mut rng = Rng(0x5eed_5eed_1234_4321);
    for size in spec::sizes() {
        let (data, clean) = sample_payload(size, &mut rng);
        let ec = usize::from(size.ec_codewords) / usize::from(size.blocks);
        let t = max_corrections(ec);
        assert!(t >= 1);
        for round in 0..3 {
            let mut g = clean.clone();
            let mut total = 0;
            for b in 0..usize::from(size.blocks) {
                total += corrupt_block(
                    &mut g,
                    size,
                    b,
                    if round == 0 { t } else { rng.below(t + 1) },
                    &mut rng,
                );
            }
            let d = decode_grid(&g)
                .unwrap_or_else(|e| panic!("{}x{} round {round}: {e:?}", size.rows, size.cols));
            assert_eq!(d.data, data);
            assert_eq!(d.errors_corrected as usize, total);
        }
    }
}

#[test]
fn beyond_capacity_never_yields_wrong_data() {
    let mut rng = Rng(0xabad_cafe_f00d_0001);
    let mut failures = 0;
    for size in spec::sizes()
        .iter()
        .filter(|s| s.ec_codewords / u16::from(s.blocks) >= 20)
    {
        let (data, clean) = sample_payload(size, &mut rng);
        let ec = usize::from(size.ec_codewords) / usize::from(size.blocks);
        for extra in 1..=4 {
            let mut g = clean.clone();
            corrupt_block(&mut g, size, 0, ec / 2 + extra, &mut rng);
            match decode_grid(&g) {
                Ok(d) => assert_eq!(d.data, data, "wrong data {}x{}", size.rows, size.cols),
                Err(_) => failures += 1,
            }
        }
    }
    assert!(failures > 40, "{failures}");
}

#[test]
fn small_blocks_refuse_the_last_correction() {
    // 10x10 has five EC codewords: two errors are within the code's reach
    // but beyond what we trust.
    let size = spec::size(10, 10).unwrap();
    let clean = paint(size, &with_ec(size, &pad(size, &ascii("AB"))));
    let mut rng = Rng(5);
    for _ in 0..20 {
        let mut g = clean.clone();
        corrupt_block(&mut g, size, 0, 1, &mut rng);
        let d = decode_grid(&g).unwrap();
        assert_eq!((d.data, d.errors_corrected), (b"AB".to_vec(), 1));
        let mut g = clean.clone();
        corrupt_block(&mut g, size, 0, 2, &mut rng);
        assert_eq!(decode_grid(&g), Err(Reject::ErrorCorrection));
    }
}

// ---- mirroring, shape and robustness -----------------------------------------

#[test]
fn mirrored_symbols_decode_for_every_shape() {
    let mut rng = Rng(99);
    for size in spec::sizes() {
        let (data, clean) = sample_payload(size, &mut rng);
        let m = mirror(&clean);
        assert_eq!((m.width(), m.height()), (clean.height(), clean.width()));
        let d = decode_grid(&m).unwrap_or_else(|e| panic!("{}x{}: {e:?}", size.rows, size.cols));
        assert_eq!(d.data, data);
        assert!(d.mirrored);
        assert_eq!((d.rows, d.cols), (size.rows, size.cols));
        assert!(!decode_grid(&clean).unwrap().mirrored);
    }
}

#[test]
fn wrong_dimensions_are_rejected() {
    for (w, h) in [
        (0, 0),
        (9, 9),
        (11, 10),
        (10, 12),
        (145, 145),
        (300, 300),
        (1, 1),
    ] {
        assert_eq!(decode_grid(&BitMatrix::new(w, h)), Err(Reject::Sampling));
    }
}

#[test]
fn random_input_never_panics_or_decodes_to_nothing() {
    let mut rng = Rng(0x0123_4567_89ab_cdef);
    for size in spec::sizes() {
        for density in [1usize, 2, 4, 8] {
            let mut g = BitMatrix::new(usize::from(size.cols), usize::from(size.rows));
            for y in 0..usize::from(size.rows) {
                for x in 0..usize::from(size.cols) {
                    if rng.below(density) == 0 {
                        g.set(x, y, true);
                    }
                }
            }
            if let Ok(d) = decode_grid(&g) {
                assert!(!d.data.is_empty());
            }
        }
    }
    // Random codeword streams through the whole encodation layer.
    let mut accepted = 0;
    for _ in 0..20000 {
        let n = 1 + rng.below(40);
        let mut s: Vec<u8> = (0..n).map(|_| rng.byte()).collect();
        // Bias towards the interesting codewords.
        for c in s.iter_mut() {
            if rng.below(3) == 0 {
                *c = [
                    229u8, 230, 231, 232, 233, 234, 235, 236, 237, 238, 239, 240, 241, 254, 129,
                ][rng.below(15)];
            }
        }
        if let Ok(p) = parse(&s, true) {
            assert!(!p.data.is_empty());
            let mut at = 0;
            for seg in &p.segments {
                assert_eq!(seg.start, at);
                assert!(seg.end > seg.start);
                at = seg.end;
            }
            accepted += 1;
        }
    }
    assert!(accepted > 100, "{accepted}");
}

#[test]
fn fuzzing_entry_matches_decoder() {
    let size = spec::size(16, 16).unwrap();
    let g = paint(size, &with_ec(size, &pad(size, &ascii("fuzz"))));
    let (rows, cols) = (16usize, 16usize);
    let mut bits = vec![0u8; (rows * cols).div_ceil(8)];
    for y in 0..rows {
        for x in 0..cols {
            if g.get(x, y) {
                bits[(y * cols + x) / 8] |= 1 << ((y * cols + x) % 8);
            }
        }
    }
    assert_eq!(
        crate::fuzzing::decode_grid(rows, cols, &bits),
        Some(b"fuzz".to_vec())
    );
    assert_eq!(crate::fuzzing::decode_grid(0, 5, &bits), None);
    assert_eq!(crate::fuzzing::decode_grid(500, 5, &bits), None);
}

#[test]
fn fuzzing_stream_entry_decodes_valid_streams() {
    let mut s = vec![230];
    s.extend(triples(&[14, 15, 16]));
    s.push(254);
    for index in [0usize, 5, 23, 24, 30, 47, 48] {
        assert_eq!(
            crate::fuzzing::decode_stream(index, &s),
            Some(b"ABC".to_vec())
        );
    }
    assert_eq!(crate::fuzzing::decode_stream(3, &[]), None);
    assert_eq!(crate::fuzzing::decode_stream(3, &[66]), Some(b"A".to_vec()));
}
