#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [shape, rest @ ..] = data else { return };
    // A quarter of the inputs go through a valid symbol built around the bytes.
    if shape & 0x40 != 0 {
        let [size, stream @ ..] = rest else { return };
        let size = usize::from(*size);
        if let Some(payload) = breader_datamatrix::fuzzing::decode_stream(size, stream) {
            assert!(!payload.is_empty());
        }
        return;
    }
    // Valid shapes come from the size table (square 10..=144 step 2/4 and
    // rectangles); the high bit selects arbitrary dimensions instead.
    let (rows, cols) = if shape & 0x80 == 0 {
        const DIMS: [(usize, usize); 20] = [
            (10, 10),
            (12, 12),
            (14, 14),
            (16, 16),
            (18, 18),
            (22, 22),
            (26, 26),
            (32, 32),
            (52, 52),
            (64, 64),
            (104, 104),
            (144, 144),
            (8, 18),
            (8, 32),
            (16, 48),
            (8, 144),
            (26, 40),
            (26, 64),
            (20, 36),
            (12, 88),
        ];
        DIMS[usize::from(*shape) % DIMS.len()]
    } else {
        let [r, c, rest @ ..] = rest else { return };
        let _ = rest;
        (usize::from(*r) % 150 + 1, usize::from(*c) % 150 + 1)
    };
    if let Some(payload) = breader_datamatrix::fuzzing::decode_grid(rows, cols, rest) {
        // A decoded symbol always carries at least one data character.
        assert!(!payload.is_empty());
    }
});
