#![no_main]

//! A thin Code 39 symbol (module 0.8 to 2.4 samples) drawn from the input and
//! read as a reader line is: width decoders, then the template fallback
//! (D-058). Layout: see `breader_linear::fuzzing::thin_code39`. When the
//! drawing is clean enough, any hit must read the text that was drawn.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (expected, hits) = breader_linear::fuzzing::thin_code39(data);
    for (_, text) in &hits {
        assert!(!text.is_empty());
        if let Some(expected) = &expected {
            assert_eq!(text, expected);
        }
    }
});
