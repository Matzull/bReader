#![no_main]

use breader_content::gs1;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Raw bytes lossily decoded cover the GS (0x1d) separator and non-ASCII.
    let text = String::from_utf8_lossy(data);
    if let Ok(elements) = gs1::parse(&text) {
        assert!(!elements.is_empty());
        for e in &elements {
            assert!(!e.ai.is_empty() && !e.value.is_empty());
        }
        let _ = gs1::hri(&elements);
    }
});
