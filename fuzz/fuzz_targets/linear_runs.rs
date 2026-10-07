#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [a, b, rest @ ..] = data else { return };
    let seed = u16::from_le_bytes([*a, *b]);
    let widths: Vec<u16> = rest
        .chunks(2)
        .map(|c| u16::from(c[0]) | u16::from(c.get(1).copied().unwrap_or(0) & 0x03) << 8)
        .collect();
    for (_, text) in breader_linear::fuzzing::decode_runs(&widths, seed) {
        assert!(!text.is_empty());
    }
});
