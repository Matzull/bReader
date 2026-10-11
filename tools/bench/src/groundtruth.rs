use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use breader::Symbology;
use serde::{Deserialize, Serialize};

use crate::matching::canonical_label;

#[derive(Debug, Clone, Deserialize)]
pub struct Annotations {
    pub format: String,
    pub images: Vec<ImageEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ImageEntry {
    pub file: String,
    pub barcodes: Vec<Barcode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Barcode {
    pub text: String,
    pub format: String,
    /// Lowercase hex of the exact expected bytes; takes precedence over `text` when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_hex: Option<String>,
    #[serde(default)]
    pub points: Vec<[f64; 2]>,
}

impl Barcode {
    pub fn data(&self) -> Vec<u8> {
        self.bytes_hex
            .as_deref()
            .and_then(decode_hex)
            .unwrap_or_else(|| self.text.clone().into_bytes())
    }
}

pub fn encode_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    data.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) || !hex.is_ascii() {
        return None;
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
        .collect()
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

/// Normalized dataset spelling of a symbology: ASCII letters and digits, upper case
/// ("EAN_13" and "ean-13" are both `EAN13`).
fn symbology_key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Maps a dataset's format name to the library symbology. `None` means the symbology is not
/// supported (yet): such labels are counted as unsupported and excluded from the read rates
/// (today MaxiCode, Aztec and others). Supporting a new symbology only needs one more arm
/// here; the test
/// `every_library_symbology_has_a_mapping` fails until it is added.
pub fn map_symbology(name: &str) -> Option<Symbology> {
    match symbology_key(name).as_str() {
        "QRCODE" | "QR" => Some(Symbology::Qr),
        "EAN13" => Some(Symbology::Ean13),
        "EAN8" => Some(Symbology::Ean8),
        "UPCA" => Some(Symbology::UpcA),
        "UPCE" => Some(Symbology::UpcE),
        "CODE128" => Some(Symbology::Code128),
        "CODE39" | "CODE39EXTENDED" => Some(Symbology::Code39),
        "ITF" | "INTERLEAVED2OF5" => Some(Symbology::Itf),
        "CODABAR" => Some(Symbology::Codabar),
        "DATAMATRIX" => Some(Symbology::DataMatrix),
        "PDF417" => Some(Symbology::Pdf417),
        "AZTEC" | "AZTECCODE" => Some(Symbology::Aztec),
        _ => None,
    }
}

const IMAGE_EXTENSIONS: [&str; 7] = ["jpg", "jpeg", "png", "webp", "gif", "bmp", "tif"];

fn is_annotation_dir(dir: &Path) -> bool {
    dir.file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("annotation"))
}

/// Annotation files: `annotations.json` anywhere, or any `.json` inside a directory named
/// `annotation` (Dynamsoft's `UPS-Synthetic-Labels/annotation/annotations_min.json` and
/// `pdf417-small-module-size/annotation/test-results.json`).
pub fn is_annotation_file(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name == "annotations.json"
        || (name.ends_with(".json") && path.parent().is_some_and(is_annotation_dir))
}

/// Directory the image names of an annotation file are relative to: the dataset root, which is
/// the parent of an `annotation` directory.
pub fn dataset_base(ann_path: &Path) -> PathBuf {
    let dir = ann_path.parent().unwrap_or(Path::new("."));
    if is_annotation_dir(dir) {
        dir.parent().unwrap_or(dir).to_path_buf()
    } else {
        dir.to_path_buf()
    }
}

/// Image file stem to path for every image below `base`, the first path winning.
pub fn index_images(base: &Path) -> BTreeMap<String, PathBuf> {
    fn walk(dir: &Path, out: &mut BTreeMap<String, PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for p in paths {
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != ".git") {
                    walk(&p, out);
                }
            } else if let (Some(stem), Some(ext)) = (
                p.file_stem().and_then(|s| s.to_str()),
                p.extension().and_then(|e| e.to_str()),
            ) && IMAGE_EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str())
            {
                out.entry(stem.to_string()).or_insert(p);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(base, &mut out);
    out
}

/// Resolves an annotation's image name. A file next to the annotations is used as is; otherwise
/// the name is matched by stem against the images below the dataset root, ignoring the
/// extension and a trailing `_annotated` (the Dynamsoft annotations name `1-1-4_annotated` for
/// `Images/1-1-4.jpeg` and omit the extension of the UPS labels). `None` when no image exists.
pub fn resolve_image(
    base: &Path,
    file: &str,
    index: &mut Option<BTreeMap<String, PathBuf>>,
) -> Option<PathBuf> {
    let direct = base.join(file);
    if direct.is_file() {
        return Some(direct);
    }
    let index = index.get_or_insert_with(|| index_images(base));
    let stem = Path::new(file).file_stem().and_then(|s| s.to_str())?;
    index
        .get(stem)
        .or_else(|| index.get(stem.strip_suffix("_annotated")?))
        .cloned()
}

/// Versioned label corrections, embedded so the bench works from any directory.
pub const OVERRIDES_JSON: &str = include_str!("../gt-overrides.json");

#[derive(Debug, Clone, Deserialize)]
pub struct Overrides {
    pub datasets: BTreeMap<String, Vec<OverrideEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OverrideEntry {
    pub file: String,
    /// Label to replace; when absent the entry adds `to` as a missing label.
    #[serde(default)]
    pub from: Option<LabelRef>,
    pub to: LabelRef,
    /// Number of identical labels an added entry stands for (several symbols with the same
    /// text in one image); labels already present count towards it.
    #[serde(default = "one")]
    pub count: usize,
    /// Corners of an added label (image coordinates), informational.
    #[serde(default)]
    pub points: Vec<[f64; 2]>,
    #[allow(dead_code)]
    pub reason: String,
}

fn one() -> usize {
    1
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabelRef {
    pub format: String,
    pub text: String,
}

pub fn parse_overrides(json: &str) -> Result<Overrides, String> {
    serde_json::from_str(json).map_err(|e| format!("gt-overrides.json: {e}"))
}

/// Applies the overrides registered for `dataset_dir` (the directory name holding
/// `annotations.json`). Returns how many labels were changed.
pub fn apply_overrides(ann: &mut Annotations, dataset_dir: &str, overrides: &Overrides) -> usize {
    let Some(entries) = overrides.datasets.get(dataset_dir) else {
        return 0;
    };
    let mut applied = 0;
    for entry in entries {
        for img in ann.images.iter_mut().filter(|i| i.file == entry.file) {
            let Some(from) = &entry.from else {
                let present = img
                    .barcodes
                    .iter()
                    .filter(|b| b.format == entry.to.format && b.text == entry.to.text)
                    .count();
                for _ in present..entry.count {
                    img.barcodes.push(Barcode {
                        text: entry.to.text.clone(),
                        format: entry.to.format.clone(),
                        bytes_hex: None,
                        points: entry.points.clone(),
                    });
                    applied += 1;
                }
                continue;
            };
            if let Some(b) = img
                .barcodes
                .iter_mut()
                .find(|b| b.format == from.format && b.text == from.text)
            {
                b.format = entry.to.format.clone();
                b.text = entry.to.text.clone();
                b.bytes_hex = None;
                applied += 1;
            }
        }
    }
    applied
}

fn centroid(points: &[[f64; 2]]) -> Option<[f64; 2]> {
    if points.is_empty() {
        return None;
    }
    let n = points.len() as f64;
    Some([
        points.iter().map(|p| p[0]).sum::<f64>() / n,
        points.iter().map(|p| p[1]).sum::<f64>() / n,
    ])
}

/// Even-odd point-in-polygon test.
fn inside(poly: &[[f64; 2]], p: [f64; 2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut hit = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            hit = !hit;
        }
        j = i;
    }
    hit
}

fn bbox(points: &[[f64; 2]]) -> Option<[f64; 4]> {
    let first = points.first()?;
    Some(
        points
            .iter()
            .fold([first[0], first[1], first[0], first[1]], |b, p| {
                [
                    b[0].min(p[0]),
                    b[1].min(p[1]),
                    b[2].max(p[0]),
                    b[3].max(p[1]),
                ]
            }),
    )
}

fn bbox_iou(a: &[[f64; 2]], b: &[[f64; 2]]) -> f64 {
    let (Some(a), Some(b)) = (bbox(a), bbox(b)) else {
        return 0.0;
    };
    let w = (a[2].min(b[2]) - a[0].max(b[0])).max(0.0);
    let h = (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let inter = w * h;
    let union = (a[2] - a[0]) * (a[3] - a[1]) + (b[2] - b[0]) * (b[3] - b[1]) - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

fn quads_overlap(a: &[[f64; 2]], b: &[[f64; 2]]) -> bool {
    centroid(a).is_some_and(|c| inside(b, c))
        || centroid(b).is_some_and(|c| inside(a, c))
        || bbox_iou(a, b) > 0.5
}

fn same_symbol(a: &Barcode, b: &Barcode) -> bool {
    let key = |x: &Barcode| match map_symbology(&x.format) {
        Some(s) => {
            let (s, d) = canonical_label(s, &x.data());
            (s.name().to_string(), d)
        }
        None => {
            let d = x.data();
            let t = match std::str::from_utf8(&d) {
                Ok(s) => s.trim().as_bytes().to_vec(),
                Err(_) => d.trim_ascii().to_vec(),
            };
            (x.format.clone(), t)
        }
    };
    key(a) == key(b) && quads_overlap(&a.points, &b.points)
}

/// Drops GT entries that label the same physical symbol twice (same normalized text and
/// overlapping quads). Of a UPC-A / EAN-13 pair the UPC-A entry is kept. Returns the
/// number of removed entries.
pub fn dedup_annotations(ann: &mut Annotations) -> usize {
    let mut removed = 0;
    for img in &mut ann.images {
        let mut kept: Vec<Barcode> = Vec::with_capacity(img.barcodes.len());
        for b in img.barcodes.drain(..) {
            match kept.iter().position(|k| same_symbol(k, &b)) {
                Some(i) => {
                    if map_symbology(&b.format) == Some(Symbology::UpcA) {
                        kept[i] = b;
                    }
                    removed += 1;
                }
                None => kept.push(b),
            }
        }
        img.barcodes = kept;
    }
    removed
}

pub fn expected_by_file(ann: &Annotations) -> BTreeMap<String, Vec<Expected>> {
    let mut map: BTreeMap<String, Vec<Expected>> = BTreeMap::new();
    for img in &ann.images {
        map.entry(img.file.clone())
            .or_default()
            .extend(img.barcodes.iter().map(|b| Expected {
                symbology: map_symbology(&b.format),
                label: b.format.clone(),
                data: b.data(),
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
      "total_barcodes": 4,
      "images": [
        {"file": "a.jpg", "barcodes": [
          {"text": "0328056306", "format": "ITF",
           "points": [[896.0, 397.0], [828.0, 239.0], [1022.0, 126.0], [1101.0, 284.0]]},
          {"text": "hello", "format": "QR_CODE", "points": [[1,2],[3,4],[5,6],[7,8]]}
        ]},
        {"file": "b.jpg", "barcodes": [
          {"text": "ab12", "format": "MAXICODE", "points": []},
          {"text": "dm", "format": "DATAMATRIX", "points": []}
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
        assert_eq!(map["b.jpg"][0].label, "MAXICODE");
        assert_eq!(map["b.jpg"][1].symbology, Some(Symbology::DataMatrix));
    }

    #[test]
    fn bytes_hex_overrides_text() {
        let json = r#"{"format":"barcode-benchmark/1.0","images":[{"file":"a.png","barcodes":[
            {"text":"?","format":"QR_CODE","bytes_hex":"ff00"},
            {"text":"t","format":"QR_CODE","bytes_hex":"zz"}]}]}"#;
        let map = expected_by_file(&parse_annotations(json).unwrap());
        assert_eq!(map["a.png"][0].data, vec![0xff, 0x00]);
        assert_eq!(map["a.png"][1].data, b"t");
        assert_eq!(encode_hex(&[0xff, 0x00, 0x0a]), "ff000a");
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
    }

    #[test]
    fn every_library_symbology_has_a_mapping() {
        for &s in Symbology::ALL {
            assert_eq!(map_symbology(s.name()), Some(s), "{}", s.name());
        }
    }

    #[test]
    fn unsupported_symbologies_have_no_symbology() {
        for n in ["MAXICODE", "MICROPDF417"] {
            assert_eq!(map_symbology(n), None, "{n}");
        }
        // PDF417 is unsupported only until the library has it; its arm in `map_symbology`
        // is then the only change needed (checked by `every_library_symbology_has_a_mapping`).
        let supported = Symbology::ALL.iter().any(|s| s.name() == "PDF417");
        assert_eq!(map_symbology("PDF417").is_some(), supported);
    }

    #[test]
    fn annotation_file_layouts() {
        assert!(is_annotation_file(Path::new(
            "d/challenging/annotations.json"
        )));
        assert!(is_annotation_file(Path::new(
            "d/UPS/annotation/annotations_min.json"
        )));
        assert!(is_annotation_file(Path::new(
            "d/p/annotation/test-results.json"
        )));
        assert!(!is_annotation_file(Path::new("d/p/annotation/sheet.xlsx")));
        assert!(!is_annotation_file(Path::new(
            "d/p/other/test-results.json"
        )));
        assert_eq!(
            dataset_base(Path::new("d/UPS/annotation/annotations_min.json")),
            Path::new("d/UPS")
        );
        assert_eq!(
            dataset_base(Path::new("d/c/annotations.json")),
            Path::new("d/c")
        );
    }

    #[test]
    fn images_resolve_by_stem() {
        let dir = std::env::temp_dir().join(format!("bench-resolve-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Images")).unwrap();
        for f in ["Images/1-1-4.jpeg", "Images/IMG_1.JPG", "a.png"] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let mut idx = None;
        assert_eq!(
            resolve_image(&dir, "a.png", &mut idx),
            Some(dir.join("a.png"))
        );
        assert!(idx.is_none());
        assert_eq!(
            resolve_image(&dir, "1-1-4_annotated", &mut idx),
            Some(dir.join("Images/1-1-4.jpeg"))
        );
        assert_eq!(
            resolve_image(&dir, "IMG_1", &mut idx),
            Some(dir.join("Images/IMG_1.JPG"))
        );
        assert_eq!(resolve_image(&dir, "IMG_2", &mut idx), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn bc(format: &str, text: &str, pts: [[f64; 2]; 4]) -> Barcode {
        Barcode {
            text: text.into(),
            format: format.into(),
            bytes_hex: None,
            points: pts.to_vec(),
        }
    }

    fn ann_of(barcodes: Vec<Barcode>) -> Annotations {
        Annotations {
            format: "barcode-benchmark/1.0".into(),
            images: vec![ImageEntry {
                file: "a.jpg".into(),
                barcodes,
            }],
        }
    }

    const SQ: [[f64; 2]; 4] = [[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]];
    const SQ_FAR: [[f64; 2]; 4] = [[500.0, 0.0], [600.0, 0.0], [600.0, 50.0], [500.0, 50.0]];

    #[test]
    fn dedup_keeps_upca_over_ean13() {
        for order in [0, 1] {
            let mut v = vec![
                bc("EAN13", "0000123456890", SQ),
                bc(
                    "UPC_A",
                    "000123456890",
                    [[2.0, 2.0], [98.0, 2.0], [98.0, 48.0], [2.0, 48.0]],
                ),
            ];
            if order == 1 {
                v.reverse();
            }
            let mut ann = ann_of(v);
            assert_eq!(dedup_annotations(&mut ann), 1);
            assert_eq!(ann.images[0].barcodes.len(), 1);
            assert_eq!(ann.images[0].barcodes[0].format, "UPC_A");
        }
    }

    #[test]
    fn dedup_requires_overlap_and_same_text() {
        let mut ann = ann_of(vec![
            bc("UPC_A", "000123456890", SQ),
            bc("EAN13", "0000123456890", SQ_FAR),
            bc("QR_CODE", "x", SQ),
            bc("QR_CODE", "y", SQ),
        ]);
        assert_eq!(dedup_annotations(&mut ann), 0);
        assert_eq!(ann.images[0].barcodes.len(), 4);
    }

    #[test]
    fn dedup_by_iou_and_missing_points() {
        let a = [[0.0, 0.0], [100.0, 0.0], [100.0, 10.0], [0.0, 10.0]];
        let mut ann = ann_of(vec![bc("QR_CODE", "x", a), bc("QR_CODE", "x", a)]);
        assert_eq!(dedup_annotations(&mut ann), 1);
        let mut ann = ann_of(vec![bc("QR_CODE", "x", SQ), bc("QR_CODE", "x", SQ)]);
        ann.images[0].barcodes[0].points.clear();
        assert_eq!(dedup_annotations(&mut ann), 0);
    }

    #[test]
    fn overrides_apply_by_dataset_dir_and_file() {
        let ov = parse_overrides(
            r#"{"datasets": {"ds": [{"file": "a.jpg",
                "from": {"format": "UPC_A", "text": "1"},
                "to": {"format": "UPC_A", "text": "2"}, "reason": "r"}]}}"#,
        )
        .unwrap();
        let mut ann = ann_of(vec![bc("UPC_A", "1", SQ)]);
        assert_eq!(apply_overrides(&mut ann, "other", &ov), 0);
        assert_eq!(ann.images[0].barcodes[0].text, "1");
        assert_eq!(apply_overrides(&mut ann, "ds", &ov), 1);
        assert_eq!(ann.images[0].barcodes[0].text, "2");
        assert_eq!(apply_overrides(&mut ann, "ds", &ov), 0);
    }

    #[test]
    fn overrides_can_add_a_missing_label_once() {
        let ov = parse_overrides(
            r#"{"datasets": {"ds": [{"file": "a.jpg",
                "to": {"format": "DATAMATRIX", "text": "n"}, "reason": "r"}]}}"#,
        )
        .unwrap();
        let mut ann = ann_of(vec![]);
        assert_eq!(apply_overrides(&mut ann, "ds", &ov), 1);
        assert_eq!(apply_overrides(&mut ann, "ds", &ov), 0);
        assert_eq!(ann.images[0].barcodes.len(), 1);
    }

    #[test]
    fn overrides_can_add_several_identical_labels() {
        let ov = parse_overrides(
            r#"{"datasets": {"ds": [{"file": "a.jpg", "count": 2,
                "to": {"format": "CODE_128", "text": "Q8"}, "reason": "r"}]}}"#,
        )
        .unwrap();
        let mut ann = ann_of(vec![bc("CODE_128", "Q8", SQ)]);
        assert_eq!(apply_overrides(&mut ann, "ds", &ov), 1);
        assert_eq!(ann.images[0].barcodes.len(), 2);
        assert_eq!(apply_overrides(&mut ann, "ds", &ov), 0);
        assert_eq!(ann.images[0].barcodes.len(), 2);
    }

    #[test]
    fn embedded_overrides_parse() {
        parse_overrides(OVERRIDES_JSON).unwrap();
    }
}
