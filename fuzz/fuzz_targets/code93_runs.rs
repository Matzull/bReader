#![no_main]

// Input: byte 0 selects the mode (bit 0: valid symbol around the data, else
// arbitrary run widths), bytes 1 and 2 are the seed, the rest the data (valid
// symbol: character values; otherwise little-endian run widths in eighths of a
// sample).

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [mode, a, b, rest @ ..] = data else { return };
    let seed = u16::from_le_bytes([*a, *b]);
    if mode & 1 == 1 {
        let (expected, hits) = breader_linear::fuzzing::code93_symbol(rest, seed);
        match expected {
            Some(text) => assert_eq!(hits.iter().map(|h| &h.1).collect::<Vec<_>>(), [&text]),
            None => assert!(hits.is_empty()),
        }
    } else {
        let widths: Vec<u16> = rest
            .chunks(2)
            .map(|c| u16::from(c[0]) | u16::from(c.get(1).copied().unwrap_or(0) & 0x03) << 8)
            .collect();
        for (_, text) in breader_linear::fuzzing::decode_code93_runs(&widths, seed) {
            assert!(!text.is_empty());
        }
    }
});
