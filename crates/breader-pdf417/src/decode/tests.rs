use super::*;
use crate::testutil::{EncodeOptions, Rng, high_level, render, seal, seal_auto, try_render};
use breader_core::Charset;
use rxing::common::CharacterSet;
use rxing::pdf417::encoder::Compaction;
use rxing::pdf417::encoder::pdf_417_error_correction::generateErrorCorrection;

/// Decodes `data` codewords wrapped in a symbol of level 2 and 5 columns.
fn parse(data: &[u16]) -> Result<DecodedPdf417, Reject> {
    let (layout, cells) = seal(data, 2, 5);
    decode_codewords(&cells, layout)
}

fn bytes(data: &[u16]) -> Vec<u8> {
    parse(data).unwrap().data
}

fn latin1(msg: &str) -> Vec<u8> {
    msg.chars().map(|c| c as u8).collect()
}

/// Packs sub-codewords into text compaction codewords, padding with 29.
fn pack_text(sub: &[u16]) -> Vec<u16> {
    sub.chunks(2)
        .map(|p| p[0] * 30 + p.get(1).copied().unwrap_or(29))
        .collect()
}

fn digits_to_cws(digits: &str) -> Vec<u16> {
    // Numeric compaction: "1" + up to 44 digits, converted to base 900 by long division.
    let mut out = Vec::new();
    for group in digits.as_bytes().chunks(44) {
        let mut n: Vec<u32> = std::iter::once(1)
            .chain(group.iter().map(|d| u32::from(d - b'0')))
            .collect();
        let mut cws = Vec::new();
        while n.iter().any(|&d| d != 0) {
            let mut rem = 0;
            for d in &mut n {
                let cur = rem * 10 + *d;
                *d = cur / 900;
                rem = cur % 900;
            }
            cws.push(rem as u16);
        }
        cws.reverse();
        out.extend(cws);
    }
    out
}

#[test]
fn ecc_matches_rxing_for_all_levels() {
    let mut rng = Rng(11);
    for level in 0..=8u8 {
        let ec = spec::ec_codewords(level);
        let data: Vec<u16> = (0..40 + rng.below(200))
            .map(|_| rng.below(929) as u16)
            .collect();
        let s: String = data
            .iter()
            .map(|&c| char::from_u32(u32::from(c)).unwrap())
            .collect();
        let theirs: Vec<u16> = generateErrorCorrection(&s, u32::from(level))
            .unwrap()
            .chars()
            .map(|c| c as u16)
            .collect();
        let mut ours = vec![0u16; ec];
        PDF417_FIELD.encode(&data, ec, &mut ours).unwrap();
        assert_eq!(theirs, ours, "level {level}");
    }
}

// ---------- text compaction ----------

#[test]
fn text_tables() {
    // Alpha: letters and space, then latches into every submode and back.
    let alpha: Vec<u16> = (0..=26).collect();
    assert_eq!(bytes(&pack_text(&alpha)), b"ABCDEFGHIJKLMNOPQRSTUVWXYZ ");
    // ll, then lowercase.
    let lower: Vec<u16> = [27].into_iter().chain(0..=26).collect();
    assert_eq!(bytes(&pack_text(&lower)), b"abcdefghijklmnopqrstuvwxyz ");
    // ml, then the mixed table 0..=24 and space.
    let mixed: Vec<u16> = [28].into_iter().chain(0..=24).chain([26]).collect();
    assert_eq!(bytes(&pack_text(&mixed)), b"0123456789&\r\t,:#-.$/+%*=^ ");
    // ml, pl, then the punctuation table 0..=28.
    let punct: Vec<u16> = [28, 25].into_iter().chain(0..=28).collect();
    assert_eq!(
        bytes(&pack_text(&punct)),
        b";<>@[\\]_`~!\r\t,:\n-.$/\"|*()?{}'"
    );
}

#[test]
fn text_latches_and_shifts() {
    // Alpha: A, ps ';', B.
    assert_eq!(bytes(&pack_text(&[0, 29, 0, 1])), b"A;B");
    // ll, a, as 'B', c (shift returns to lower), ps '!' (10), d.
    assert_eq!(bytes(&pack_text(&[27, 0, 27, 1, 2, 29, 10, 3])), b"aBc!d");
    // ml '1', ps '&'? mixed 29 shifts to punct: ml, '1', ps ';' (0), '2'.
    assert_eq!(bytes(&pack_text(&[28, 1, 29, 0, 2])), b"1;2");
    // Mixed -> alpha (28) and mixed -> lower (27).
    assert_eq!(bytes(&pack_text(&[28, 1, 28, 0, 28, 1, 27, 0])), b"1A1a");
    // Punct -> alpha (29).
    assert_eq!(bytes(&pack_text(&[28, 25, 0, 29, 0])), b";A");
    // Mixed -> punct (25) latch persists.
    assert_eq!(bytes(&pack_text(&[28, 25, 0, 1, 2])), b";<>");
    // Lower: space is 26, ps and then back to lower.
    assert_eq!(bytes(&pack_text(&[27, 26, 29, 0, 0])), b" ;a");
}

#[test]
fn text_padding_shift_is_dropped() {
    // An odd sub-codeword count ends with the pad 29 (a ps).
    assert_eq!(bytes(&pack_text(&[0, 1, 2])), b"ABC");
    // The pad before a byte shift does not affect the next text codeword.
    let mut cws = pack_text(&[0, 1, 2]);
    cws.extend([913, 0xE9]);
    cws.extend(pack_text(&[3, 4]));
    assert_eq!(bytes(&cws), b"ABC\xe9DE");
    // The same pad before a numeric latch.
    let mut cws = pack_text(&[0]);
    cws.push(902);
    cws.extend(digits_to_cws("123"));
    assert_eq!(bytes(&cws), b"A123");
}

#[test]
fn text_invalid_sequences_are_rejected() {
    // ps followed by the al value 29 inside the shift.
    assert!(parse(&pack_text(&[29, 29, 0, 0])).is_err());
    // as followed by a latch value.
    assert!(parse(&pack_text(&[27, 27, 27, 0])).is_err());
    // A text codeword above 899 is a mode codeword, not text: 900 + 29.
    assert!(parse(&[903]).is_err());
}

#[test]
fn latch_to_text_restarts_in_alpha() {
    let mut cws = pack_text(&[27, 0, 1]);
    cws.push(900);
    cws.extend(pack_text(&[0, 1]));
    assert_eq!(bytes(&cws), b"abAB");
}

// ---------- byte compaction ----------

fn byte_codewords(data: &[u8]) -> Vec<u16> {
    let mut out = Vec::new();
    for g in data.chunks(6) {
        if g.len() == 6 {
            let mut v = g.iter().fold(0u64, |a, &b| a << 8 | u64::from(b));
            let mut c = [0u16; 5];
            for x in c.iter_mut().rev() {
                *x = (v % 900) as u16;
                v /= 900;
            }
            out.extend(c);
        } else {
            out.extend(g.iter().map(|&b| u16::from(b)));
        }
    }
    out
}

#[test]
fn byte_compaction_group_boundaries() {
    let mut rng = Rng(5);
    for len in 1..=40 {
        let data: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        // 924 when the count is a multiple of 6, else 901.
        let latch = if len % 6 == 0 { 924 } else { 901 };
        let mut cws = vec![latch];
        cws.extend(byte_codewords(&data));
        assert_eq!(bytes(&cws), data, "len {len} latch {latch}");
    }
}

#[test]
fn byte_latches_and_trailing_groups() {
    // After 901 the last one to five codewords are single bytes: five
    // codewords are five bytes, not a group of six.
    assert_eq!(bytes(&[901, 1, 2, 3, 4, 5]), [1, 2, 3, 4, 5]);
    let mut cws = vec![901];
    cws.extend(byte_codewords(b"abcdef"));
    cws.extend([1, 2, 3, 4, 5]);
    assert_eq!(bytes(&cws), b"abcdef\x01\x02\x03\x04\x05");
    // After 924 the run holds whole groups only.
    let mut cws = vec![924];
    cws.extend(byte_codewords(b"abcdefghijkl"));
    assert_eq!(bytes(&cws), b"abcdefghijkl");
    assert!(parse(&[924, 1, 2, 3]).is_err());
    let mut cws = vec![924];
    cws.extend(byte_codewords(b"abcdef"));
    cws.push(7);
    assert!(parse(&cws).is_err());
}

#[test]
fn byte_compaction_extremes() {
    let all_ff = [0xFFu8; 6];
    let mut cws = vec![924];
    cws.extend(byte_codewords(&all_ff));
    assert_eq!(bytes(&cws), all_ff);
    let zeros = [0u8; 12];
    let mut cws = vec![924];
    cws.extend(byte_codewords(&zeros));
    assert_eq!(bytes(&cws), zeros);
    // A group above 2^48 - 1 cannot be six bytes.
    assert!(parse(&[924, 899, 899, 899, 899, 899]).is_err());
    // 2^48 - 1 is the last valid value.
    let max = (1u64 << 48) - 1;
    let mut c = [0u16; 5];
    let mut v = max;
    for x in c.iter_mut().rev() {
        *x = (v % 900) as u16;
        v /= 900;
    }
    let mut cws = vec![924];
    cws.extend(c);
    assert_eq!(bytes(&cws), &max.to_be_bytes()[2..]);
    // 2^48 is not.
    c[4] += 1;
    let mut cws = vec![924];
    cws.extend(c);
    assert!(parse(&cws).is_err());
    // A single byte above 255 in the remainder.
    assert!(parse(&[901, 256]).is_err());
    assert!(parse(&[901, 1, 2, 3, 4, 300]).is_err());
}

#[test]
fn byte_shift_from_text() {
    let mut cws = pack_text(&[0, 1]);
    cws.extend([913, 0x00, 913, 0xFF]);
    cws.extend(pack_text(&[2, 3]));
    assert_eq!(bytes(&cws), b"AB\x00\xffCD");
    // Needs a byte after it, below 256, and a text mode.
    assert!(parse(&[913]).is_err());
    assert!(parse(&[913, 300]).is_err());
    assert!(parse(&[901, 913, 5]).is_err());
    assert!(parse(&[902, 913, 5]).is_err());
}

#[test]
fn byte_run_then_text_and_numeric() {
    let mut cws = vec![901];
    cws.extend(byte_codewords(b"\x01\x02\x03\x04\x05\x06\x07"));
    cws.push(900);
    cws.extend(pack_text(&[0, 1]));
    cws.push(902);
    cws.extend(digits_to_cws("0042"));
    cws.push(901);
    cws.extend([0x41, 0x42]);
    assert_eq!(bytes(&cws), b"\x01\x02\x03\x04\x05\x06\x07AB0042AB");
}

// ---------- numeric compaction ----------

#[test]
fn numeric_compaction_lengths() {
    let mut rng = Rng(77);
    for len in 1..=120 {
        let digits: String = (0..len)
            .map(|_| char::from(b'0' + rng.below(10) as u8))
            .collect();
        let mut cws = vec![902];
        cws.extend(digits_to_cws(&digits));
        assert_eq!(bytes(&cws), digits.as_bytes(), "len {len}");
    }
}

#[test]
fn numeric_group_limits() {
    // 44 digits fill a group of 15 codewords; 45 digits make a second group.
    let nines = "9".repeat(44);
    let cws = digits_to_cws(&nines);
    assert_eq!(cws.len(), 15);
    let mut full = vec![902];
    full.extend(&cws);
    assert_eq!(bytes(&full), nines.as_bytes());
    let long = "9".repeat(45);
    let mut c = vec![902];
    c.extend(digits_to_cws(&long));
    assert_eq!(bytes(&c), long.as_bytes());
    // The value must start with the digit 1 (a group of 1 codeword: 10..=19, 100..=199).
    assert!(parse(&[902, 20]).is_err());
    assert!(parse(&[902, 1]).is_err());
    assert!(parse(&[902, 0]).is_err());
    assert_eq!(bytes(&[902, 10]), b"0");
    assert_eq!(bytes(&[902, 199]), b"99");
    assert!(parse(&[902, 200]).is_err());
    // 900^15 - 1 starts with 2: no group of 15 may hold it.
    let mut c = vec![902];
    c.extend([899u16; 15]);
    assert!(parse(&c).is_err());
}

// ---------- ECI ----------

#[test]
fn eci_escapes_and_segments() {
    // 927 26, then bytes.
    let mut cws = pack_text(&[0, 1]);
    cws.extend([927, 26, 901, 0xC3, 0xA9]);
    let d = parse(&cws).unwrap();
    assert_eq!(d.data, b"AB\xc3\xa9");
    assert_eq!(
        d.segments,
        [
            DataSegment {
                start: 0,
                end: 2,
                charset: Charset::Latin1
            },
            DataSegment {
                start: 2,
                end: 4,
                charset: Charset::Eci(26)
            },
        ]
    );
    assert_eq!(&d.aim_id, b"]L1");
    // Two-codeword escape: 900 + 5 * 900 + 7 = 5407.
    let d = parse(&[926, 5, 7, 901, 0x41]).unwrap();
    assert_eq!(d.segments[0].charset, Charset::Eci(5407));
    // Highest two-codeword value, and the one-codeword high range.
    let d = parse(&[926, 899, 899, 901, 0x41]).unwrap();
    assert_eq!(d.segments[0].charset, Charset::Eci(810_899));
    let d = parse(&[925, 0, 901, 0x41]).unwrap();
    assert_eq!(d.segments[0].charset, Charset::Eci(810_900));
    let d = parse(&[925, 899, 901, 0x41]).unwrap();
    assert_eq!(d.segments[0].charset, Charset::Eci(811_799));
    // ECI 3 is the default but is still reported as declared.
    let d = parse(&[927, 3, 901, 0x41]).unwrap();
    assert_eq!(d.segments[0].charset, Charset::Eci(3));
    // Changing ECI mid-stream; an empty region between two escapes vanishes.
    let d = parse(&[927, 20, 927, 26, 901, 0x41, 927, 3, 927, 7, 0x42]).unwrap();
    assert_eq!(d.data, b"AB");
    assert_eq!(d.segments.len(), 2);
    assert_eq!(d.segments[0].charset, Charset::Eci(26));
    assert_eq!(d.segments[1].charset, Charset::Eci(7));
    // No ECI: the plain data is one Latin-1 segment and the modifier is 2.
    let d = parse(&pack_text(&[0, 1])).unwrap();
    assert_eq!(
        d.segments,
        [DataSegment {
            start: 0,
            end: 2,
            charset: Charset::Latin1
        }]
    );
    assert_eq!(&d.aim_id, b"]L2");
}

#[test]
fn eci_truncated_or_out_of_range() {
    assert!(parse(&[927]).is_err());
    assert!(parse(&[926, 1]).is_err());
    assert!(parse(&[925]).is_err());
    assert!(parse(&[927, 900]).is_err());
    assert!(parse(&[926, 1, 900, 901, 1]).is_err());
    // ECI without any data after it is an empty payload.
    assert!(parse(&[927, 26]).is_err());
}

#[test]
fn eci_inside_numeric_and_byte_runs() {
    // The escape ends the byte run: a group and a single byte, ECI, then two single bytes.
    let mut cws = vec![901];
    cws.extend(byte_codewords(b"abcdefg"));
    cws.extend([927, 26, 0x41, 0x42]);
    let d = parse(&cws).unwrap();
    assert_eq!(d.data, b"abcdefgAB");
    assert_eq!(d.segments[1].charset, Charset::Eci(26));
    let mut cws = vec![902];
    cws.extend(digits_to_cws("12"));
    cws.extend([927, 9]);
    cws.extend(digits_to_cws("34"));
    let d = parse(&cws).unwrap();
    assert_eq!(d.data, b"1234");
}

// ---------- reader initialisation, reserved codewords, padding ----------

#[test]
fn reader_initialisation_only_first() {
    let mut cws = vec![921];
    cws.extend(pack_text(&[0, 1]));
    let d = parse(&cws).unwrap();
    assert!(d.reader_init);
    assert_eq!(d.data, b"AB");
    let mut cws = pack_text(&[0, 1]);
    cws.push(921);
    assert!(parse(&cws).is_err());
    assert!(!parse(&pack_text(&[0, 1])).unwrap().reader_init);
}

#[test]
fn reserved_and_stray_codewords_are_rejected() {
    for cw in (903..=912).chain(914..=920).chain([922, 923]) {
        assert!(parse(&[0, cw, 0]).is_err(), "{cw}");
        assert!(parse(&[0, 1, 2, cw]).is_err(), "{cw}");
    }
}

#[test]
fn padding_and_empty_payloads() {
    // seal pads with 900; the payload is just the data.
    assert_eq!(bytes(&pack_text(&[0, 1])), b"AB");
    assert!(parse(&[]).is_err());
    assert!(parse(&[900, 900]).is_err());
    assert!(parse(&[902]).is_err());
}

// ---------- Macro PDF417 ----------

fn segment_index_cws(index: u32) -> [u16; 2] {
    let v = 100_000 + index;
    [(v / 900) as u16, (v % 900) as u16]
}

#[test]
fn macro_minimal_and_terminator() {
    let mut cws = pack_text(&[0, 1]);
    cws.push(928);
    cws.extend(segment_index_cws(3));
    cws.extend([12, 345]);
    let d = parse(&cws).unwrap();
    let m = d.macro_pdf417.unwrap();
    assert_eq!(d.data, b"AB");
    assert_eq!(m.segment_index, 3);
    assert_eq!(m.file_id, "012345");
    assert!(!m.last_segment);
    assert_eq!(m.file_name, None);
    cws.push(922);
    let m = parse(&cws).unwrap().macro_pdf417.unwrap();
    assert!(m.last_segment);
}

#[test]
fn macro_all_fields() {
    let mut cws = pack_text(&[0, 1, 2]);
    cws.push(928);
    cws.extend(segment_index_cws(99_998));
    cws.extend([1, 2, 3]);
    // 0: file name "AB.TXT": A B, ml '.', al, T X T.
    cws.extend([923, 0]);
    cws.extend(pack_text(&[0, 1, 28, 17, 28, 19, 23, 19]));
    // 1: segment count 5.
    cws.extend([923, 1]);
    cws.extend(digits_to_cws("5"));
    // 2: time stamp.
    cws.extend([923, 2]);
    cws.extend(digits_to_cws("1700000000"));
    // 3: sender "A", 4: addressee "B" (lowercase via ll).
    cws.extend([923, 3]);
    cws.extend(pack_text(&[0, 1]));
    cws.extend([923, 4]);
    cws.extend(pack_text(&[27, 1, 2]));
    // 5: file size, 6: checksum.
    cws.extend([923, 5]);
    cws.extend(digits_to_cws("123456789012345678"));
    cws.extend([923, 6]);
    cws.extend(digits_to_cws("65535"));
    cws.push(922);
    let d = parse(&cws).unwrap();
    assert_eq!(d.data, b"ABC");
    let m = d.macro_pdf417.unwrap();
    assert_eq!(m.segment_index, 99_998);
    assert_eq!(m.file_id, "001002003");
    assert_eq!(m.file_name.as_deref(), Some("AB.TXT"));
    assert_eq!(m.segment_count, Some(5));
    assert_eq!(m.timestamp, Some(1_700_000_000));
    assert_eq!(m.sender.as_deref(), Some("AB"));
    assert_eq!(m.addressee.as_deref(), Some("bc"));
    assert_eq!(m.file_size, Some(123_456_789_012_345_678));
    assert_eq!(m.checksum, Some(65_535));
    assert!(m.last_segment);
}

#[test]
fn macro_block_before_data() {
    // Control block first, then data in the default text mode.
    // The file id runs up to the next mode codeword: here the text latch.
    let mut cws = vec![928];
    cws.extend(segment_index_cws(0));
    cws.extend([7, 900]);
    cws.extend(pack_text(&[0, 1]));
    let d = parse(&cws).unwrap();
    assert_eq!(d.data, b"AB");
    assert_eq!(d.macro_pdf417.unwrap().file_id, "007");
}

#[test]
fn macro_malformed_blocks_are_rejected() {
    let idx = segment_index_cws(1);
    let head = |extra: &[u16]| {
        let mut v = pack_text(&[0, 1]);
        v.push(928);
        v.extend(idx);
        v.extend(extra);
        v
    };
    // Missing file id.
    assert!(parse(&head(&[])).is_err());
    assert!(parse(&head(&[922])).is_err());
    // Truncated index.
    assert!(parse(&[0, 928, 5]).is_err());
    // Index outside 0..=99998, or not five digits.
    for bad in [[111u16, 0], [100, 0], [0, 5]] {
        let mut v = pack_text(&[0, 1]);
        v.extend([928, bad[0], bad[1], 1]);
        assert!(parse(&v).is_err(), "{bad:?}");
    }
    let mut v = pack_text(&[0, 1]);
    v.push(928);
    v.extend(segment_index_cws(99_999));
    v.push(1);
    assert!(parse(&v).is_err());
    // Unknown field, duplicate field, empty field, truncated field marker.
    assert!(parse(&head(&[1, 923, 7, 1])).is_err());
    assert!(parse(&head(&[1, 923, 1, 10, 923, 1, 10])).is_err());
    assert!(parse(&head(&[1, 923, 1])).is_err());
    assert!(parse(&head(&[1, 923, 1, 922])).is_err());
    assert!(parse(&head(&[1, 923])).is_err());
    // Numeric field that is not a valid group, or too large for the type.
    assert!(parse(&head(&[1, 923, 5, 20])).is_err());
    let mut big = head(&[1, 923, 1]);
    big.extend(digits_to_cws("99999999999"));
    assert!(parse(&big).is_err());
    // Text field with an invalid shift sequence.
    let mut t = head(&[1, 923, 0]);
    t.extend(pack_text(&[29, 29]));
    assert!(parse(&t).is_err());
    // Two control blocks.
    let mut two = head(&[1]);
    two.push(928);
    two.extend(idx);
    two.push(2);
    assert!(parse(&two).is_err());
    // A terminator without a block.
    assert!(parse(&[0, 922]).is_err());
}

// ---------- symbol-level checks ----------

fn corrupt(rng: &mut Rng, cells: &mut [u16], p: usize) {
    cells[p] = (cells[p] + 1 + rng.below(928) as u16) % 929;
}

fn pick(rng: &mut Rng, n: usize, k: usize) -> Vec<usize> {
    let mut v = Vec::new();
    while v.len() < k {
        let p = rng.below(n);
        if !v.contains(&p) {
            v.push(p);
        }
    }
    v
}

#[test]
fn symbol_length_descriptor_is_checked() {
    let (layout, mut cells) = seal(&pack_text(&[0, 1, 2, 3]), 2, 5);
    let ec = spec::ec_codewords(2);
    let n = cells.len();
    assert!(decode_codewords(&cells, layout).is_ok());
    // Re-seal with a different descriptor: valid check codewords, wrong length.
    for bad in [0u16, 1, (n - ec - 1) as u16, (n - ec + 1) as u16, 928] {
        let mut c = cells.clone();
        c[0] = bad;
        let mut check = vec![0u16; ec];
        PDF417_FIELD.encode(&c[..n - ec], ec, &mut check).unwrap();
        c[n - ec..].copy_from_slice(&check);
        assert_eq!(
            decode_codewords(&c, layout),
            Err(Reject::Format),
            "sld {bad}"
        );
    }
    // Wrong matrix size for the layout.
    cells.pop();
    assert_eq!(decode_codewords(&cells, layout), Err(Reject::Format));
}

#[test]
fn layout_limits() {
    // A symbol with no room for data after the check codewords.
    let layout = Layout::new(3, 1, 2).unwrap();
    assert_eq!(decode_codewords(&[0; 3], layout), Err(Reject::Format));
    let layout = Layout::new(90, 30, 0).unwrap();
    assert_eq!(
        decode_codewords(&vec![0; 2700], layout),
        Err(Reject::Format)
    );
}

#[test]
fn guard_budget_per_level() {
    // ec, errors accepted, erasures accepted.
    let table = [
        (0usize, 0usize, 0usize),
        (1, 1, 2),
        (2, 3, 6),
        (3, 7, 14),
        (4, 15, 31),
        (5, 31, 63),
    ];
    let mut rng = Rng(1234);
    for (level, max_err, max_era) in table {
        let ec = spec::ec_codewords(level as u8);
        let data = pack_text(&(0..60).map(|i| (i * 7 % 26) as u16).collect::<Vec<_>>());
        let (layout, clean) = seal(&data, level as u8, 8);
        let n = clean.len();
        let expect = decode_codewords(&clean, layout).unwrap().data;
        assert_eq!(expect.len(), 60);
        // Errors.
        for e in 0..=(ec / 2 + 1).min(n) {
            let mut cells = clean.clone();
            for p in pick(&mut rng, n, e) {
                corrupt(&mut rng, &mut cells, p);
            }
            let r = decode_codewords(&cells, layout);
            if e <= max_err {
                let d = r.unwrap_or_else(|x| panic!("level {level} e={e}: {x:?}"));
                assert_eq!((d.errors_corrected as usize, d.erasures), (e, 0));
                assert_eq!(d.data, expect);
            } else {
                assert_eq!(r, Err(Reject::ErrorCorrection), "level {level} e={e}");
            }
        }
        // Erasures: every flagged cell is damaged.
        for f in 0..=(ec + 1).min(n) {
            let mut cells = clean.clone();
            for p in pick(&mut rng, n, f) {
                cells[p] = ERASED;
            }
            let r = decode_codewords(&cells, layout);
            if f <= max_era {
                let d = r.unwrap_or_else(|x| panic!("level {level} f={f}: {x:?}"));
                assert_eq!((d.errors_corrected as usize, d.erasures as usize), (0, f));
                assert_eq!(d.data, expect);
            } else {
                assert_eq!(r, Err(Reject::ErrorCorrection), "level {level} f={f}");
            }
        }
    }
}

#[test]
fn guard_mixed_errors_and_erasures() {
    let mut rng = Rng(99);
    let data = pack_text(&(0..80).map(|i| (i * 5 % 26) as u16).collect::<Vec<_>>());
    let (layout, clean) = seal(&data, 3, 10);
    let n = clean.len();
    let budget = 14;
    for e in 0..=8usize {
        for f in 0..=16usize {
            let mut cells = clean.clone();
            let pos = pick(&mut rng, n, e + f);
            for &p in &pos[..e] {
                corrupt(&mut rng, &mut cells, p);
            }
            for &p in &pos[e..] {
                cells[p] = ERASED;
            }
            let r = decode_codewords(&cells, layout);
            if 2 * e + f <= budget {
                let d = r.unwrap_or_else(|x| panic!("e={e} f={f}: {x:?}"));
                assert_eq!((d.errors_corrected as usize, d.erasures as usize), (e, f));
            } else {
                assert_eq!(r, Err(Reject::ErrorCorrection), "e={e} f={f}");
            }
        }
    }
}

#[test]
fn beyond_capacity_never_decodes_wrong_data() {
    let mut rng = Rng(0xabcdef);
    let mut wrong = 0;
    for case in 0..3000 {
        let level = (case % 4) as u8;
        let data = pack_text(&(0..30).map(|_| rng.below(26) as u16).collect::<Vec<_>>());
        let (layout, clean) = seal(&data, level, 6);
        let expect = decode_codewords(&clean, layout).unwrap().data;
        let ec = spec::ec_codewords(level);
        let e = ec / 2 + 1 + rng.below(ec);
        let mut cells = clean.clone();
        for p in pick(&mut rng, clean.len(), e) {
            corrupt(&mut rng, &mut cells, p);
        }
        if let Ok(d) = decode_codewords(&cells, layout) {
            wrong += usize::from(d.data != expect);
        }
    }
    assert_eq!(wrong, 0);
}

#[test]
fn erased_values_above_928_are_erasures() {
    let (layout, mut cells) = seal(&pack_text(&[0, 1, 2, 3]), 2, 5);
    cells[3] = 929;
    cells[4] = 5000;
    let d = decode_codewords(&cells, layout).unwrap();
    assert_eq!(d.erasures, 2);
    assert_eq!(d.data, b"ABCD");
}

// ---------- round trips with an independent encoder ----------

fn roundtrip(msg: &str, opts: EncodeOptions) {
    let Some(r) = try_render(msg, opts) else {
        return;
    };
    let d = decode_codewords(&r.cells, r.layout).unwrap_or_else(|e| panic!("{e:?} for {msg:?}"));
    assert_eq!(d.data, latin1(msg), "{msg:?} {:?}", r.layout);
    assert_eq!(d.layout, r.layout);
}

#[test]
fn rxing_text_roundtrip_all_ascii() {
    // Every printable and the characters of the four submodes, shuffled.
    let all: String = (0x20u8..0x7F)
        .map(char::from)
        .chain(['\t', '\r', '\n'])
        .collect();
    for compaction in [Compaction::TEXT, Compaction::AUTO] {
        roundtrip(
            &all,
            EncodeOptions {
                compaction,
                ..EncodeOptions::default()
            },
        );
    }
    let mut rng = Rng(31);
    for _ in 0..200 {
        let len = 1 + rng.below(200);
        let msg: String = (0..len)
            .map(|_| {
                let pool = all.as_bytes();
                char::from(pool[rng.below(pool.len())])
            })
            .collect();
        for compaction in [Compaction::TEXT, Compaction::AUTO] {
            let opts = EncodeOptions {
                compaction,
                level: rng.below(9) as u32,
                compact: rng.below(2) == 0,
                ..EncodeOptions::default()
            };
            roundtrip(&msg, opts);
        }
    }
}

#[test]
fn rxing_text_modes_with_runs() {
    // Long runs of one submode, then switches, to exercise latches over shifts.
    let msg =
        "UPPER lower 0123456789 &,:#-.$/+%*=^ ;<>@[\\]_`~!|\"(){}?' mixed Case, 12345 end.\r\n";
    for level in 0..=8 {
        roundtrip(
            msg,
            EncodeOptions {
                compaction: Compaction::TEXT,
                level,
                ..EncodeOptions::default()
            },
        );
        roundtrip(
            msg,
            EncodeOptions {
                compaction: Compaction::AUTO,
                level,
                ..EncodeOptions::default()
            },
        );
    }
}

#[test]
fn rxing_binary_roundtrip() {
    let mut rng = Rng(8);
    for len in (1..=60).chain([100, 255, 400]) {
        let msg: String = (0..len).map(|_| char::from(rng.next() as u8)).collect();
        for compaction in [Compaction::BYTE, Compaction::AUTO] {
            let opts = EncodeOptions {
                compaction,
                level: 1 + rng.below(5) as u32,
                ..EncodeOptions::default()
            };
            let Some(r) = try_render(&msg, opts) else {
                continue;
            };
            let d = decode_codewords(&r.cells, r.layout).unwrap();
            assert_eq!(
                d.data,
                latin1(&msg),
                "len {len} compaction {}",
                compaction as u8
            );
        }
    }
}

#[test]
fn rxing_numeric_roundtrip() {
    let mut rng = Rng(21);
    for len in (1..=60).chain([100, 200, 300]) {
        let msg: String = (0..len)
            .map(|_| char::from(b'0' + rng.below(10) as u8))
            .collect();
        for compaction in [Compaction::NUMERIC, Compaction::AUTO] {
            roundtrip(
                &msg,
                EncodeOptions {
                    compaction,
                    level: 2,
                    ..EncodeOptions::default()
                },
            );
        }
    }
    // Leading zeros and a run of 44 and 45 digits.
    for digits in [
        "0",
        "00",
        "000000000000000000",
        "0".repeat(44).as_str(),
        "7".repeat(45).as_str(),
        "1".repeat(88).as_str(),
    ] {
        roundtrip(
            digits,
            EncodeOptions {
                compaction: Compaction::NUMERIC,
                ..EncodeOptions::default()
            },
        );
    }
}

#[test]
fn rxing_mixed_compaction_roundtrip() {
    let mut rng = Rng(4);
    for _ in 0..300 {
        let mut msg = String::new();
        for _ in 0..1 + rng.below(6) {
            let len = 1 + rng.below(30);
            match rng.below(3) {
                0 => msg.extend((0..len).map(|_| char::from(b'0' + rng.below(10) as u8))),
                1 => msg.extend((0..len).map(|_| char::from(0x20 + rng.below(95) as u8))),
                _ => msg.extend((0..len).map(|_| char::from(rng.next() as u8))),
            }
        }
        let opts = EncodeOptions {
            level: rng.below(9) as u32,
            compact: rng.below(2) == 0,
            ..EncodeOptions::default()
        };
        roundtrip(&msg, opts);
    }
}

#[test]
fn rxing_eci() {
    let opts = EncodeOptions {
        encoding: Some(CharacterSet::UTF8),
        ..EncodeOptions::default()
    };
    let msg = "h\u{e9}llo \u{20ac} w\u{f6}rld \u{4e2d}\u{6587}";
    let r = render(msg, opts);
    let d = decode_codewords(&r.cells, r.layout).unwrap();
    assert_eq!(d.data, msg.as_bytes());
    assert_eq!(d.segments.len(), 1);
    assert_eq!(d.segments[0].charset, Charset::Eci(26));
    assert_eq!(&d.aim_id, b"]L1");

    let opts = EncodeOptions {
        auto_eci: true,
        ..EncodeOptions::default()
    };
    let msg = "abc \u{20ac} \u{410}\u{411}\u{412} xyz";
    let r = render(msg, opts);
    let d = decode_codewords(&r.cells, r.layout).unwrap();
    // Decode each segment with the charset the symbol declares: ECI 4..=13 are
    // ISO 8859 parts, 26 is UTF-8; here the check is that the escapes parse
    // and every byte is covered by a declared segment.
    assert!(!d.data.is_empty());
    assert_eq!(d.segments.first().unwrap().start, 0);
    assert_eq!(d.segments.last().unwrap().end, d.data.len());
    for w in d.segments.windows(2) {
        assert_eq!(w[0].end, w[1].start);
        assert_ne!(w[0].charset, w[1].charset);
    }
}

#[test]
fn rxing_errors_and_erasures_on_encoded_symbols() {
    let mut rng = Rng(2024);
    for case in 0..120 {
        let len = 20 + rng.below(250);
        let msg: String = (0..len)
            .map(|_| char::from(0x20 + rng.below(95) as u8))
            .collect();
        let level = 2 + rng.below(5) as u32;
        let opts = EncodeOptions {
            level,
            cols: Some(4 + rng.below(8) as u32),
            ..EncodeOptions::default()
        };
        let Some(r) = try_render(&msg, opts) else {
            continue;
        };
        let ec = spec::ec_codewords(level as u8);
        let budget = ec - reserve(ec);
        let n = r.cells.len();
        let e = rng.below(budget / 2 + 1);
        let f = rng.below(budget - 2 * e + 1);
        let pos = pick(&mut rng, n, e + f);
        let mut cells = r.cells.clone();
        for &p in &pos[..e] {
            corrupt(&mut rng, &mut cells, p);
        }
        for &p in &pos[e..] {
            cells[p] = if case % 2 == 0 { ERASED } else { 5000 };
        }
        let d = decode_codewords(&cells, r.layout).unwrap_or_else(|x| panic!("{x:?} e={e} f={f}"));
        assert_eq!(d.data, msg.as_bytes());
        assert_eq!((d.errors_corrected as usize, d.erasures as usize), (e, f));
    }
}

#[test]
fn rxing_largest_symbols() {
    // Level 8: 512 check codewords.
    let msg: String = (0..380)
        .map(|i| char::from(b'A' + (i % 26) as u8))
        .collect();
    let r = render(
        &msg,
        EncodeOptions {
            level: 8,
            compaction: Compaction::TEXT,
            ..EncodeOptions::default()
        },
    );
    let d = decode_codewords(&r.cells, r.layout).unwrap();
    assert_eq!(d.data, msg.as_bytes());
    let mut cells = r.cells.clone();
    let mut rng = Rng(1);
    for p in pick(&mut rng, cells.len(), 255) {
        corrupt(&mut rng, &mut cells, p);
    }
    let d = decode_codewords(&cells, r.layout).unwrap();
    assert_eq!(d.data, msg.as_bytes());
    assert_eq!(d.errors_corrected, 255);
}

#[test]
fn hand_built_symbol_from_high_level_stream() {
    // The encoder's high-level codewords wrapped by our sealer decode the same.
    let msg = "Hello, PDF417 \u{e9} 0123456789012345 \u{1}\u{2}";
    let hl = high_level(msg, Compaction::AUTO);
    for level in [0u8, 3, 6] {
        let (layout, cells) = seal_auto(&hl, level);
        let d = decode_codewords(&cells, layout).unwrap();
        assert_eq!(d.data, latin1(msg));
    }
}

#[test]
fn eci_inside_a_byte_run_keeps_the_groups_aligned() {
    // A symbol with designators between the groups of one byte run (the
    // run is 12 codewords: two groups and two single bytes).
    let data = [
        1u16, 927, 10, 901, 927, 20, 246, 877, 166, 106, 797, 927, 30, 927, 29, 415, 537, 357, 329,
        194, 927, 8, 927, 9, 225, 926, 899, 898, 927, 22, 144, 927, 4, 900, 913, 172, 925, 1, 900,
    ];
    let d = parse(&data).unwrap();
    let charsets: Vec<_> = d
        .segments
        .iter()
        .map(|s| (s.start, s.end, s.charset))
        .collect();
    assert_eq!(
        charsets[..4],
        [
            (0, 2, Charset::Latin1),
            (2, 8, Charset::Eci(20)),
            (8, 14, Charset::Eci(29)),
            (14, 15, Charset::Eci(9)),
        ]
    );
}

#[test]
fn a_designator_may_not_split_a_group_of_five() {
    assert_eq!(
        parse(&[901, 246, 877, 927, 4, 166, 106, 797, 5, 5, 5]).err(),
        Some(Reject::Format)
    );
}

#[test]
fn text_decoder_view_of_segments() {
    // Without ECI everything is one ISO-8859-1 segment, as for Data Matrix.
    let d = parse(&[901, 0xE9]).unwrap();
    assert_eq!(d.data, [0xE9]);
    assert_eq!(
        d.segments,
        [DataSegment {
            start: 0,
            end: 1,
            charset: Charset::Latin1
        }]
    );
}
