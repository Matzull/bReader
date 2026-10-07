#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [version, rest @ ..] = data else { return };
    // Valid dimensions are 21 + 4 * (version - 1); also try off-size grids.
    let dim = if version & 0x80 == 0 {
        21 + 4 * (usize::from(*version) % 40)
    } else {
        usize::from(*version & 0x7f) + 1
    };
    if let Some(payload) = breader_qr::fuzzing::decode_grid(dim, rest) {
        // A decoded QR may legitimately have an empty payload only if it
        // carries no segments; the reader rejects those, so just touch it.
        let _ = payload.len();
    }
});
