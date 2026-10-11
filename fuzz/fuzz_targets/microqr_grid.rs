#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [size, rest @ ..] = data else { return };
    // Valid dimensions are 11, 13, 15 and 17; also try off-size grids.
    let dim = if size & 0x80 == 0 {
        11 + 2 * (usize::from(*size) % 4)
    } else {
        usize::from(*size & 0x3f) + 1
    };
    if let Some(payload) = breader_qr::fuzzing::decode_micro_grid(dim, rest) {
        let _ = payload.len();
    }
});
