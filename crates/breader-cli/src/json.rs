//! Minimal JSON writing: strings with correct escaping and finite numbers.

use std::fmt::Write;

/// Appends `s` as a JSON string literal (quotes included).
pub fn push_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Appends a float; JSON has no NaN or infinity, so those become `null`.
pub fn push_f32(out: &mut String, v: f32) {
    if v.is_finite() {
        let _ = write!(out, "{v}");
    } else {
        out.push_str("null");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(s: &str) -> String {
        let mut out = String::new();
        push_string(&mut out, s);
        out
    }

    #[test]
    fn plain_text_is_only_quoted() {
        assert_eq!(string("hello"), "\"hello\"");
        assert_eq!(string(""), "\"\"");
    }

    #[test]
    fn quotes_backslashes_and_named_controls_are_escaped() {
        assert_eq!(string("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(string("\n\r\t\u{8}\u{c}"), "\"\\n\\r\\t\\b\\f\"");
    }

    #[test]
    fn other_controls_use_four_digit_escapes() {
        assert_eq!(string("\u{0}\u{1f}\u{7f}"), "\"\\u0000\\u001f\u{7f}\"");
        // GS1 group separator, common in Code 128 payloads.
        assert_eq!(string("01\u{1d}10"), "\"01\\u001d10\"");
    }

    #[test]
    fn non_ascii_is_kept_as_utf8() {
        assert_eq!(string("é€😀"), "\"é€😀\"");
    }

    #[test]
    fn floats_are_finite_numbers_or_null() {
        let mut out = String::new();
        push_f32(&mut out, 12.5);
        out.push(' ');
        push_f32(&mut out, f32::NAN);
        out.push(' ');
        push_f32(&mut out, f32::INFINITY);
        out.push(' ');
        push_f32(&mut out, -0.25);
        assert_eq!(out, "12.5 null null -0.25");
    }
}
