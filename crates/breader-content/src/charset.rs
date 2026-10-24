//! Charset decoding for symbol payloads: ECI designators, declared segment
//! charsets and the heuristic for undeclared byte-mode data.

use breader_core::Charset;
use encoding_rs::Encoding;

const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', 'É', 'æ', 'Æ',
    'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', 'á', 'í', 'ó', 'ú', 'ñ', 'Ñ',
    'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕',
    '╣', '║', '╗', '╝', '╜', '╛', '┐', '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦',
    '╠', '═', '╬', '╧', '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐',
    '▀', 'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', '≡', '±',
    '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{a0}',
];

/// Decodes `bytes` under `charset`. Returns `None` when the charset is
/// unknown or binary, or when the bytes are not valid in it.
pub fn decode(charset: Charset, bytes: &[u8]) -> Option<String> {
    match charset {
        Charset::Ascii => ascii(bytes),
        Charset::Latin1 => Some(latin1(bytes)),
        Charset::ShiftJis => strict(encoding_rs::SHIFT_JIS, bytes),
        Charset::Eci(n) => decode_eci(n, bytes),
        Charset::Undeclared => Some(undeclared(bytes)),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

fn ascii(bytes: &[u8]) -> Option<String> {
    bytes
        .is_ascii()
        .then(|| bytes.iter().map(|&b| char::from(b)).collect())
}

fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| char::from(b)).collect()
}

fn cp437(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            0..=0x7f => char::from(b),
            _ => CP437_HIGH[usize::from(b - 0x80)],
        })
        .collect()
}

fn utf32(bytes: &[u8], big_endian: bool) -> Option<String> {
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&c| {
            char::from_u32(if big_endian {
                u32::from_be_bytes(c)
            } else {
                u32::from_le_bytes(c)
            })
        })
        .collect()
}

fn strict(encoding: &'static Encoding, bytes: &[u8]) -> Option<String> {
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|s| s.into_owned())
}

fn decode_eci(eci: u32, bytes: &[u8]) -> Option<String> {
    use encoding_rs as e;
    let encoding = match eci {
        0 | 2 => return Some(cp437(bytes)),
        1 | 3 => return Some(latin1(bytes)),
        4 => e::ISO_8859_2,
        5 => e::ISO_8859_3,
        6 => e::ISO_8859_4,
        7 => e::ISO_8859_5,
        8 => e::ISO_8859_6,
        9 => e::ISO_8859_7,
        10 => e::ISO_8859_8,
        // WHATWG maps ISO-8859-9 and -11 to their Windows supersets, which
        // differ only in the C1 range.
        11 => e::WINDOWS_1254,
        12 => e::ISO_8859_10,
        13 => e::WINDOWS_874,
        15 => e::ISO_8859_13,
        16 => e::ISO_8859_14,
        17 => e::ISO_8859_15,
        18 => e::ISO_8859_16,
        20 => e::SHIFT_JIS,
        21 => e::WINDOWS_1250,
        22 => e::WINDOWS_1251,
        23 => e::WINDOWS_1252,
        24 => e::WINDOWS_1256,
        25 => e::UTF_16BE,
        26 => e::UTF_8,
        27 | 170 => return ascii(bytes),
        28 => e::BIG5,
        29 | 31 => e::GBK,
        30 => e::EUC_KR,
        32 => e::GB18030,
        33 => e::UTF_16LE,
        34 => return utf32(bytes, true),
        35 => return utf32(bytes, false),
        _ => return None,
    };
    strict(encoding, bytes)
}

/// True when every byte is valid Shift JIS and at least one double-byte
/// character occurs.
fn is_shift_jis_text(bytes: &[u8]) -> bool {
    let mut double = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            0x00..=0x7f | 0xa1..=0xdf => i += 1,
            0x81..=0x9f | 0xe0..=0xfc => match bytes.get(i + 1) {
                Some(0x40..=0x7e | 0x80..=0xfc) => {
                    double = true;
                    i += 2;
                }
                _ => return false,
            },
            _ => return false,
        }
    }
    double
}

/// Undeclared byte data: strict UTF-8, then Shift JIS (only when it has a
/// double-byte character and is fully valid), otherwise ISO-8859-1.
fn undeclared(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    if is_shift_jis_text(bytes)
        && let Some(s) = strict(encoding_rs::SHIFT_JIS, bytes)
    {
        return s;
    }
    latin1(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eci(n: u32, bytes: &[u8]) -> Option<String> {
        decode(Charset::Eci(n), bytes)
    }

    #[test]
    fn eci_latin1_family() {
        assert_eq!(eci(3, b"caf\xe9").as_deref(), Some("café"));
        assert_eq!(eci(1, b"caf\xe9").as_deref(), Some("café"));
        assert_eq!(eci(4, b"\xb1").as_deref(), Some("ą"));
        assert_eq!(eci(7, b"\xc0\xe0").as_deref(), Some("Рр"));
        assert_eq!(eci(17, b"\xa4").as_deref(), Some("€"));
        assert_eq!(eci(9, b"\xe1").as_deref(), Some("α"));
        assert_eq!(eci(22, b"\xcf\xf0").as_deref(), Some("Пр"));
        assert_eq!(eci(23, b"\x80").as_deref(), Some("€"));
        assert_eq!(eci(21, b"\xa3").as_deref(), Some("Ł"));
    }

    #[test]
    fn eci_cp437() {
        assert_eq!(
            eci(2, b"A\x80\x9b\xb0\xe1\xff").as_deref(),
            Some("AÇ¢░ß\u{a0}")
        );
        assert_eq!(eci(0, &[0xfb]).as_deref(), Some("√"));
        assert_eq!(CP437_HIGH.len(), 128);
    }

    #[test]
    fn eci_shift_jis_and_cjk() {
        assert_eq!(eci(20, &[0x93, 0xfa, 0x96, 0x7b]).as_deref(), Some("日本"));
        assert_eq!(eci(20, &[0x93]), None);
        assert_eq!(eci(28, &[0xa4, 0x40]).as_deref(), Some("一"));
        assert_eq!(eci(29, &[0xd6, 0xd0]).as_deref(), Some("中"));
        assert_eq!(eci(31, &[0xd6, 0xd0]).as_deref(), Some("中"));
        assert_eq!(eci(30, &[0xb0, 0xa1]).as_deref(), Some("가"));
        assert_eq!(eci(32, &[0xd6, 0xd0]).as_deref(), Some("中"));
    }

    #[test]
    fn eci_unicode() {
        assert_eq!(eci(26, "héllo €".as_bytes()).as_deref(), Some("héllo €"));
        assert_eq!(eci(26, &[0xc3]), None);
        assert_eq!(eci(25, &[0x00, 0x41, 0x20, 0xac]).as_deref(), Some("A€"));
        assert_eq!(eci(33, &[0x41, 0x00, 0xac, 0x20]).as_deref(), Some("A€"));
        assert_eq!(eci(25, &[0x00]), None);
        assert_eq!(eci(25, &[0xd8, 0x00]), None);
        assert_eq!(
            eci(34, &[0, 0, 0, 0x41, 0, 1, 0xf6, 0x00]).as_deref(),
            Some("A😀")
        );
        assert_eq!(eci(35, &[0x41, 0, 0, 0]).as_deref(), Some("A"));
        assert_eq!(eci(34, &[0, 0, 0xd8, 0]), None);
        assert_eq!(eci(34, &[0, 0, 0]), None);
    }

    #[test]
    fn eci_ascii_binary_unknown() {
        assert_eq!(eci(27, b"abc").as_deref(), Some("abc"));
        assert_eq!(eci(170, b"abc").as_deref(), Some("abc"));
        assert_eq!(eci(27, b"\xe9"), None);
        assert_eq!(eci(899, b"abc"), None);
        assert_eq!(eci(14, b"abc"), None);
        assert_eq!(eci(12345, b"abc"), None);
        assert_eq!(eci(u32::MAX, b""), None);
    }

    #[test]
    fn undeclared_heuristic() {
        let d = |b: &[u8]| decode(Charset::Undeclared, b).unwrap();
        assert_eq!(d("héllo".as_bytes()), "héllo");
        assert_eq!(d(b"plain"), "plain");
        assert_eq!(d(&[0x93, 0xfa, 0x96, 0x7b]), "日本");
        assert_eq!(d(b"caf\xe9"), "café");
        // Valid Shift JIS bytes without a double-byte character stay Latin-1.
        assert_eq!(d(&[0xb1, 0xb2]), "±²");
        // Truncated double-byte sequence.
        assert_eq!(d(&[0x93]), "\u{93}");
        assert_eq!(d(b""), "");
    }

    #[test]
    fn declared_simple_charsets() {
        assert_eq!(decode(Charset::Ascii, b"ab").as_deref(), Some("ab"));
        assert_eq!(decode(Charset::Ascii, b"\xe9"), None);
        assert_eq!(decode(Charset::Latin1, b"\xe9").as_deref(), Some("é"));
        assert_eq!(
            decode(Charset::ShiftJis, &[0x93, 0xfa]).as_deref(),
            Some("日")
        );
        assert_eq!(decode(Charset::ShiftJis, &[0x81]), None);
    }

    #[test]
    fn random_bytes_never_panic() {
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        for _ in 0..3000 {
            let len = (next() % 40) as usize;
            let bytes: Vec<u8> = (0..len).map(|_| next() as u8).collect();
            for n in (0..40).chain([170, 899, 1_000_000]) {
                let _ = eci(n, &bytes);
            }
            for cs in [
                Charset::Ascii,
                Charset::Latin1,
                Charset::ShiftJis,
                Charset::Undeclared,
            ] {
                let _ = decode(cs, &bytes);
            }
        }
    }
}
