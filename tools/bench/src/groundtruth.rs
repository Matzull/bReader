use std::collections::BTreeMap;
use std::path::Path;

use breader::Symbology;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Annotations {
    pub format: String,
    pub images: Vec<ImageEntry>,
}

#[derive(Debug, Deserialize)]
pub struct ImageEntry {
    pub file: String,
    pub barcodes: Vec<Barcode>,
}

#[derive(Debug, Deserialize)]
pub struct Barcode {
    pub text: String,
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expected {
    /// `None` when the dataset's symbology has no `Symbology` counterpart.
    pub symbology: Option<Symbology>,
    pub label: String,
    pub data: Vec<u8>,
}

pub fn parse_annotations(json: &str) -> Result<Annotations, String> {
    let ann: Annotations = serde_json::from_str(json).map_err(|e| e.to_string())?;
    if !ann.format.starts_with("barcode-benchmark/") {
        return Err(format!("unknown annotation format {:?}", ann.format));
    }
    Ok(ann)
}

pub fn load_annotations(path: &Path) -> Result<Annotations, String> {
    let json = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_annotations(&json)
}

/// Dataset spellings vary ("EAN13" and "EAN_13", "QRCode" and "QR_CODE").
pub fn map_symbology(name: &str) -> Option<Symbology> {
    let key: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    match key.as_str() {
        "QRCODE" | "QR" => Some(Symbology::Qr),
        "EAN13" => Some(Symbology::Ean13),
        "EAN8" => Some(Symbology::Ean8),
        "UPCA" => Some(Symbology::UpcA),
        "UPCE" => Some(Symbology::UpcE),
        "CODE128" => Some(Symbology::Code128),
        "CODE39" | "CODE39EXTENDED" => Some(Symbology::Code39),
        "ITF" | "INTERLEAVED2OF5" => Some(Symbology::Itf),
        _ => None,
    }
}

pub fn expected_by_file(ann: &Annotations) -> BTreeMap<String, Vec<Expected>> {
    let mut map: BTreeMap<String, Vec<Expected>> = BTreeMap::new();
    for img in &ann.images {
        map.entry(img.file.clone())
            .or_default()
            .extend(img.barcodes.iter().map(|b| Expected {
                symbology: map_symbology(&b.format),
                label: b.format.clone(),
                data: b.text.clone().into_bytes(),
            }));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "format": "barcode-benchmark/1.0",
      "dataset": "Verified Collection",
      "total_images": 2,
      "total_barcodes": 3,
      "images": [
        {"file": "a.jpg", "barcodes": [
          {"text": "0328056306", "format": "ITF",
           "points": [[896.0, 397.0], [828.0, 239.0], [1022.0, 126.0], [1101.0, 284.0]]},
          {"text": "hello", "format": "QR_CODE", "points": [[1,2],[3,4],[5,6],[7,8]]}
        ]},
        {"file": "b.jpg", "barcodes": [
          {"text": "ab12", "format": "DATAMATRIX", "points": []}
        ]}
      ]
    }"#;

    #[test]
    fn parses_sample() {
        let ann = parse_annotations(SAMPLE).unwrap();
        assert_eq!(ann.images.len(), 2);
        let map = expected_by_file(&ann);
        let a = &map["a.jpg"];
        assert_eq!(a[0].symbology, Some(Symbology::Itf));
        assert_eq!(a[0].data, b"0328056306");
        assert_eq!(a[1].symbology, Some(Symbology::Qr));
        assert_eq!(map["b.jpg"][0].symbology, None);
        assert_eq!(map["b.jpg"][0].label, "DATAMATRIX");
    }

    #[test]
    fn rejects_unknown_format() {
        assert!(parse_annotations(r#"{"format":"other/2","images":[]}"#).is_err());
        assert!(parse_annotations("not json").is_err());
    }

    #[test]
    fn symbology_aliases() {
        for n in ["EAN13", "EAN_13", "ean-13"] {
            assert_eq!(map_symbology(n), Some(Symbology::Ean13));
        }
        assert_eq!(map_symbology("QRCode"), Some(Symbology::Qr));
        assert_eq!(map_symbology("CODE_39_EXTENDED"), Some(Symbology::Code39));
        assert_eq!(map_symbology("Code128"), Some(Symbology::Code128));
        assert_eq!(map_symbology("UPC_A"), Some(Symbology::UpcA));
        assert_eq!(map_symbology("PDF417"), None);
    }
}
