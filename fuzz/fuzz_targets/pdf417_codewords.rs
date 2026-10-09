#![no_main]

// Input: [shape, cols, level, rest...]; `rest` is read as little-endian u16
// codewords.
// - Shape bit 7 clear: a rows x cols matrix (rows = shape % 88 + 3) decoded as
//   given, so most inputs fail Reed-Solomon.
// - Shape bit 7 set: the codewords are the data of a symbol with a valid
//   length descriptor, padding and check codewords, which reaches the
//   compaction layer.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [shape, cols, level, rest @ ..] = data else {
        return;
    };
    let words: Vec<u16> = rest
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    let cols = usize::from(*cols) % 30 + 1;
    let level = *level % 9;
    let payload = if shape & 0x80 != 0 {
        breader_pdf417::fuzzing::decode_stream(cols, level, &words)
    } else {
        let rows = usize::from(*shape) % 88 + 3;
        breader_pdf417::fuzzing::decode_codewords(rows, cols, level, &words)
    };
    if let Some(payload) = payload {
        assert!(!payload.is_empty());
    }
});
