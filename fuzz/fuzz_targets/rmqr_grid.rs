#![no_main]

use libfuzzer_sys::fuzz_target;

const SIZES: [(usize, usize); 32] = [
    (7, 43),
    (7, 59),
    (7, 77),
    (7, 99),
    (7, 139),
    (9, 43),
    (9, 59),
    (9, 77),
    (9, 99),
    (9, 139),
    (11, 27),
    (11, 43),
    (11, 59),
    (11, 77),
    (11, 99),
    (11, 139),
    (13, 27),
    (13, 43),
    (13, 59),
    (13, 77),
    (13, 99),
    (13, 139),
    (15, 43),
    (15, 59),
    (15, 77),
    (15, 99),
    (15, 139),
    (17, 43),
    (17, 59),
    (17, 77),
    (17, 99),
    (17, 139),
];

fuzz_target!(|data: &[u8]| {
    let [size, rest @ ..] = data else { return };
    // Valid sizes are the 32 of the standard; also try off-size grids.
    let (h, w) = if size & 0x80 == 0 {
        SIZES[usize::from(*size) % SIZES.len()]
    } else {
        (
            usize::from(*size & 0x1f) + 1,
            usize::from(rest.first().copied().unwrap_or(0)) % 150 + 1,
        )
    };
    if let Some(payload) = breader_qr::fuzzing::decode_rmqr_grid(w, h, rest) {
        let _ = payload.len();
    }
});
