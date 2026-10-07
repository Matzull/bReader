#![no_main]

use breader_content::charset;
use breader_core::Charset;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [kind, e0, e1, e2, e3, bytes @ ..] = data else {
        return;
    };
    let eci = u32::from_le_bytes([*e0, *e1, *e2, *e3]);
    let eci = if kind & 0x80 != 0 { eci % 1000 } else { eci };
    let cs = match kind % 5 {
        0 => Charset::Ascii,
        1 => Charset::Latin1,
        2 => Charset::ShiftJis,
        3 => Charset::Eci(eci),
        _ => Charset::Undeclared,
    };
    if let Some(s) = charset::decode(cs, bytes) {
        if cs == Charset::Ascii {
            assert!(s.is_ascii());
        }
        if cs == Charset::Latin1 {
            assert_eq!(s.chars().count(), bytes.len());
        }
    }
});
