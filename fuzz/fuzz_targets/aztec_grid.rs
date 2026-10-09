#![no_main]

// Input: [shape, rest...].
// - Shape bit 7 set: a valid symbol built around `rest` as the high-level
//   bit stream; bit 6 set makes it an Aztec Rune of value rest[0] instead;
//   bits 0-2 add check words.
// - Shape bit 6 set (bit 7 clear): `rest` as a bare high-level bit stream.
// - Otherwise: an arbitrary square module grid; the side comes from the
//   table of valid symbol sizes, or from rest[0] when bit 5 is set; bit 4
//   decodes the transposed grid.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [shape, rest @ ..] = data else { return };
    if shape & 0x80 != 0 {
        let rune = (shape & 0x40 != 0).then(|| rest.first().copied().unwrap_or(0));
        if let Some(payload) = breader_aztec::fuzzing::decode_stream(rune, shape & 7, rest) {
            assert!(!payload.is_empty());
        }
        return;
    }
    if shape & 0x40 != 0 {
        if let Some(payload) = breader_aztec::fuzzing::decode_bits(rest) {
            assert!(!payload.is_empty());
        }
        return;
    }
    const SIZES: [usize; 16] = [
        11, 15, 19, 23, 27, 31, 35, 51, 75, 99, 101, 123, 139, 147, 151, 19,
    ];
    let (size, rest) = if shape & 0x20 != 0 {
        let [s, rest @ ..] = rest else { return };
        (usize::from(*s) % 160 + 1, rest)
    } else {
        (SIZES[usize::from(*shape) % SIZES.len()], rest)
    };
    if let Some(payload) = breader_aztec::fuzzing::decode_grid(size, rest, shape & 0x10 != 0) {
        assert!(!payload.is_empty());
    }
});
