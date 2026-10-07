#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [a, b, profile @ ..] = data else { return };
    let seed = u16::from_le_bytes([*a, *b]);
    for (_, text) in breader_linear::fuzzing::scan_profile(profile, seed) {
        assert!(!text.is_empty());
    }
});
