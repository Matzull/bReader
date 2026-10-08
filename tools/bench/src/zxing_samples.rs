//! Conversion of the zxing-cpp `test/samples` layout (one directory per symbology, an expected
//! `<stem>.txt` or `<stem>.bin` next to each image) into the bench annotation format.

use std::path::Path;

use serde_json::{Value, json};

use crate::groundtruth::encode_hex;

pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "webp", "gif"];

/// Annotation format name for a sample directory such as `qrcode-2`.
pub fn dir_format(dir: &str) -> String {
    let prefix = dir.trim_end_matches(|c: char| c.is_ascii_digit());
    let prefix = prefix.strip_suffix('-').unwrap_or(prefix);
    match prefix {
        "qrcode" => "QR_CODE",
        "microqrcode" => "MICRO_QR_CODE",
        "rmqrcode" => "RMQR_CODE",
        "datamatrix" => "DATAMATRIX",
        "code39" => "CODE_39",
        "code93" => "CODE_93",
        "code128" => "CODE_128",
        "ean8" | "ean8-extension" => "EAN_8",
        "ean13" | "ean13-extension" => "EAN_13",
        "upca" | "upca-extension" => "UPC_A",
        "upce" | "upce-extension" => "UPC_E",
        "itf" => "ITF",
        "aztec" => "AZTEC",
        "pdf417" => "PDF417",
        "micropdf417" => "MICRO_PDF417",
        "codabar" => "CODABAR",
        "rss14" => "DATABAR",
        "rssexpanded" => "DATABAR_EXPANDED",
        "rssexpandedstacked" => "DATABAR_EXPANDED_STACKED",
        "databarltd" => "DATABAR_LIMITED",
        "maxicode" => "MAXICODE",
        "dxfilmedge" => "DX_FILM_EDGE",
        "telepen" => "TELEPEN",
        other => return other.to_ascii_uppercase().replace('-', "_"),
    }
    .to_string()
}

/// Directories whose images must not decode to anything.
pub fn is_negative_dir(dir: &str) -> bool {
    dir.starts_with("falsepositives")
}

/// Format named by a `<stem>.result.txt` `format=` line (e.g. "Code 32"), when it differs from
/// the directory's symbology.
pub fn result_format(result_txt: &str) -> Option<String> {
    let value = result_txt
        .lines()
        .find_map(|l| l.strip_prefix("format="))?
        .trim();
    let name: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    (!name.is_empty()).then_some(name)
}

/// `01-02` -> `01`: the parts of a structured-append symbol share the merged expected file.
pub fn merged_stem(stem: &str) -> Option<&str> {
    let (head, tail) = stem.rsplit_once('-')?;
    (!head.is_empty() && !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()))
        .then_some(head)
}

/// Drops the EAN-2 / EAN-5 add-on digits zxing-cpp appends to the text of EAN/UPC symbols in the
/// `*-extension-*` directories; the default reader ignores add-ons, so the labeled symbol is the
/// main code.
pub fn strip_addon(format: &str, data: &[u8]) -> Vec<u8> {
    let main: &[usize] = match format {
        "EAN_13" => &[13],
        "UPC_A" => &[12, 13],
        "EAN_8" | "UPC_E" => &[8],
        _ => return data.to_vec(),
    };
    if data.iter().all(u8::is_ascii_digit)
        && let Some(&n) = main
            .iter()
            .find(|&&n| data.len() == n + 2 || data.len() == n + 5)
    {
        return data[..n].to_vec();
    }
    data.to_vec()
}

/// Annotation barcode for the expected bytes of one image.
pub fn barcode(format: &str, data: &[u8]) -> Value {
    match std::str::from_utf8(data) {
        Ok(text) => json!({ "text": text, "format": format, "points": [] }),
        Err(_) => json!({
            "text": String::from_utf8_lossy(data),
            "format": format,
            "bytes_hex": encode_hex(data),
            "points": [],
        }),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Label {
    Expected(Vec<u8>),
    /// Image is a part of a structured-append symbol; only the merged text is known.
    StructuredAppendPart,
    Missing,
}

pub fn read_label(dir: &Path, stem: &str) -> Label {
    for ext in ["txt", "bin"] {
        if let Ok(data) = std::fs::read(dir.join(format!("{stem}.{ext}"))) {
            return Label::Expected(data);
        }
    }
    if let Some(head) = merged_stem(stem)
        && ["txt", "bin"]
            .iter()
            .any(|ext| dir.join(format!("{head}.{ext}")).is_file())
    {
        return Label::StructuredAppendPart;
    }
    Label::Missing
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_follow_the_directory_prefix() {
        assert_eq!(dir_format("qrcode-2"), "QR_CODE");
        assert_eq!(dir_format("microqrcode-1"), "MICRO_QR_CODE");
        assert_eq!(dir_format("ean13-extension-1"), "EAN_13");
        assert_eq!(
            dir_format("rssexpandedstacked-2"),
            "DATABAR_EXPANDED_STACKED"
        );
        assert_eq!(dir_format("newcode-1"), "NEWCODE");
    }

    #[test]
    fn result_format_names() {
        assert_eq!(
            result_format("symbologyIdentifier=]A0\nformat=Code 32"),
            Some("CODE_32".into())
        );
        assert_eq!(result_format("symbologyIdentifier=]A0\n"), None);
    }

    #[test]
    fn structured_append_stems() {
        assert_eq!(merged_stem("01-02"), Some("01"));
        assert_eq!(merged_stem("telepen-alpha-1"), Some("telepen-alpha"));
        assert_eq!(merged_stem("abc"), None);
        assert_eq!(merged_stem("a-b"), None);
    }

    #[test]
    fn addons_are_stripped_from_ean_upc_labels() {
        assert_eq!(
            strip_addon("EAN_13", b"978073520044951299"),
            b"9780735200449"
        );
        assert_eq!(strip_addon("EAN_13", b"978073520044912"), b"9780735200449");
        assert_eq!(strip_addon("UPC_A", b"0027011006951"), b"0027011006951");
        assert_eq!(
            strip_addon("UPC_A", b"002701100695102601"),
            b"0027011006951"
        );
        assert_eq!(strip_addon("UPC_A", b"00270110069512"), b"002701100695");
        assert_eq!(strip_addon("EAN_8", b"1234567012"), b"12345670");
        assert_eq!(strip_addon("CODE_128", b"1234567890123"), b"1234567890123");
    }

    #[test]
    fn non_utf8_expectations_keep_their_bytes() {
        let b = barcode("QR_CODE", &[0xff, 0x41]);
        assert_eq!(b["bytes_hex"], "ff41");
        let b = barcode("QR_CODE", b"hi");
        assert!(b.get("bytes_hex").is_none());
    }
}
