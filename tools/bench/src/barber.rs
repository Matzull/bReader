//! BarBeR dataset: ground truth read directly from the VGG Image Annotator (VIA) project files
//! (`Annotations/VIA/*.json`, one per source dataset) and converted in memory into the bench
//! annotation format. See docs/datasets.md for the format findings and the rules below.
//!
//! * `Type` is a short code (`EAN13`, `C128`, `UPCS`, `I2O5`, `UCC128`, ...), not the spelling of
//!   the dataset's README.
//! * Regions whose value is unknown (`String` -1, the `^` placeholder, empty) and the `1D` / `-1`
//!   types are *undecodable*: they are not expected reads and a read overlapping one is not a
//!   false positive (its value is unknown). They are emitted with the format [`UNDECODABLE`].
//! * Types without a library symbology (postal codes, IATA 2 of 5, EAN-2) are emitted under their
//!   own name and so count as unsupported (excluded).
//! * An image named in several VIA files (the Dubska and Szentandrasi-Dubska QR sets share 24)
//!   is scanned once, under the first source in file-name order, with the union of the regions.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::groundtruth::{Annotations, Barcode, ImageEntry, encode_hex};

/// Format of a region whose value is unknown (not an expected read, masks false positives).
pub const UNDECODABLE: &str = "UNDECODABLE";

/// Name of the group of overrides in `gt-overrides.json`.
pub const OVERRIDES_KEY: &str = "barber";

/// Source dataset annotations and the directory their image names are relative to.
#[derive(Debug)]
pub struct Source {
    pub name: String,
    pub annotations: Annotations,
}

/// Finds the VIA directory of a BarBeR checkout: `root` itself, or below it
/// (`<root>/BarBeR - Dataset/Annotations/VIA`, or `<root>/Annotations/VIA`).
pub fn find_via_dir(root: &Path) -> Option<PathBuf> {
    let candidates = [
        root.to_path_buf(),
        root.join("Annotations").join("VIA"),
        root.join("BarBeR - Dataset")
            .join("Annotations")
            .join("VIA"),
    ];
    candidates.into_iter().find(|d| {
        d.file_name().is_some_and(|n| n == "VIA")
            && d.join("Artelab.json").is_file()
            && d.parent()
                .and_then(Path::parent)
                .is_some_and(|b| b.join("dataset").join("images").is_dir())
    })
}

/// Directory holding the images of the BarBeR checkout whose VIA directory is `via`.
pub fn image_dir(via: &Path) -> PathBuf {
    via.parent()
        .and_then(Path::parent)
        .map_or_else(PathBuf::new, |b| b.join("dataset").join("images"))
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Replaces the XML character references the ZVZ-real strings carry (`&amp;`, `&gt;`, ...).
pub fn html_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let entity = rest.find(';').filter(|&e| e <= 8).map(|e| (&rest[1..e], e));
        let decoded = entity.and_then(|(name, e)| {
            let c = match name {
                "amp" => '&',
                "lt" => '<',
                "gt" => '>',
                "quot" => '"',
                "apos" => '\'',
                _ => {
                    let num = name.strip_prefix('#')?;
                    let v = match num.strip_prefix(['x', 'X']) {
                        Some(h) => u32::from_str_radix(h, 16).ok()?,
                        None => num.parse().ok()?,
                    };
                    char::from_u32(v)?
                }
            };
            Some((c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// A string that carries no value: -1 (undecodable), the `^` placeholder of ZVZ-real (also
/// followed by whitespace only) or nothing.
fn is_unknown_value(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t == "-1" || t.strip_prefix('^').is_some_and(|r| r.trim().is_empty())
}

/// `(01)123(10)ab` -> `01123` + `10ab`: the GS1 notation with parenthesised application
/// identifiers, as the dataset's (rare) GS1-128 strings might use it.
fn strip_gs1_parentheses(s: &str) -> String {
    if !s.starts_with('(') {
        return s.to_string();
    }
    let mut out = String::new();
    let mut in_ai = false;
    for c in s.chars() {
        match (c, in_ai) {
            ('(', false) => in_ai = true,
            (')', true) => in_ai = false,
            _ => out.push(c),
        }
    }
    out
}

/// Maps a VIA region (`Type`, `String`) to a bench `(format, text)`. The format is
/// [`UNDECODABLE`] for regions without a usable value.
///
/// * `EAN13` strings of 12 or 8 digits are UPC-A / EAN-8 symbols the annotators filed under
///   EAN-13; any other length is not a well-formed EAN-13 and is treated as unknown.
/// * `UPCS` and `UPCA` are UPC-A (12 digits) or UPC-E (8 digits).
/// * `UCC128` (GS1-128) is Code 128 with GS1 content; the bench folds a leading FNC1 away.
/// * Code 39 strings lose their `*` start/stop delimiters.
pub fn map_region(ty: &str, string: &str) -> (String, String) {
    let text = html_unescape(string);
    let undecodable = || (UNDECODABLE.to_string(), text.clone());
    if is_unknown_value(&text) {
        return undecodable();
    }
    let digits = is_digits(text.trim());
    let n = text.trim().len();
    let (format, text) = match ty {
        "EAN13" => match (digits, n) {
            (true, 13) => ("EAN_13", text),
            (true, 12) => ("UPC_A", text),
            (true, 8) => ("EAN_8", text),
            _ => return undecodable(),
        },
        "EAN8" => ("EAN_8", text),
        "UPCS" | "UPCA" => match (digits, n) {
            (true, 12) => ("UPC_A", text),
            (true, 8) => ("UPC_E", text),
            _ => return undecodable(),
        },
        "C128" => ("CODE_128", text),
        "UCC128" => ("CODE_128", strip_gs1_parentheses(&text)),
        "C39" => {
            let t = text.trim();
            let inner = t
                .strip_prefix('*')
                .and_then(|r| r.strip_suffix('*'))
                .unwrap_or(t);
            ("CODE_39", inner.to_string())
        }
        "I2O5" => ("ITF", text),
        "QR" => ("QR_CODE", text),
        "DATAMATRIX" => ("DATAMATRIX", text),
        "PDF417" => ("PDF417", text),
        "AZTEC" => ("AZTEC", text),
        "2-DIGIT" => ("EAN_2", text),
        "IATA25" => ("IATA_2_OF_5", text),
        "INTELLIGENTMAIL" => ("INTELLIGENT_MAIL", text),
        "JAPANPOST" => ("JAPAN_POST", text),
        "KIX" => ("KIX", text),
        "POSTNET" => ("POSTNET", text),
        "ROYALMAILCODE" => ("ROYAL_MAIL", text),
        // "1D", "-1" and anything unknown: not a decodable symbol.
        _ => return undecodable(),
    };
    (format.to_string(), text)
}

/// IBM code page 437, bytes 0x80 to 0xFF.
const CP437_HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";

fn cp437_byte(c: char) -> Option<u8> {
    if c.is_ascii() {
        return Some(c as u8);
    }
    CP437_HIGH
        .chars()
        .position(|h| h == c)
        .and_then(|i| u8::try_from(0x80 + i).ok())
}

/// The ZVZ-real annotation tool shows the payload bytes of PDF417, Data Matrix and Aztec symbols
/// as IBM437 text (`M÷glinger` for `M\xf6glinger`), and binary payloads as the hex of those
/// characters' UTF-16 code units (`0002005E` for the bytes 02 00). A NUL is shown as `^`, like a
/// real `^`, so a string with a `^` unit does not identify its bytes ([`Payload::Ambiguous`]).
/// `None` when the string is plain text (or not convertible, as a digit string such as
/// `91700467` that only looks like such hex).
fn legacy_payload(ty: &str, text: &str) -> Option<Payload> {
    if !matches!(ty, "PDF417" | "DATAMATRIX" | "AZTEC") {
        return None;
    }
    let t = text.trim_end_matches(['\n', '\r']);
    let hex_form = t.len() >= 8
        && t.len().is_multiple_of(4)
        && t.bytes()
            .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b));
    if hex_form {
        let units: Option<Vec<char>> = (0..t.len())
            .step_by(4)
            .map(|i| char::from_u32(u32::from_str_radix(&t[i..i + 4], 16).ok()?))
            .collect();
        let units = units?;
        // Units that all fit a byte are the byte values; otherwise the symbol's bytes are
        // shown as IBM437 characters.
        let latin1 = units.iter().all(|&c| u32::from(c) <= 0xff);
        let bytes: Vec<u8> = units
            .iter()
            .filter_map(|&c| {
                if latin1 {
                    u8::try_from(c).ok()
                } else {
                    cp437_byte(c)
                }
            })
            .collect();
        if bytes.len() != units.len() {
            return None;
        }
        return Some(if units.contains(&'^') {
            Payload::Ambiguous
        } else {
            Payload::Bytes(bytes)
        });
    }
    // Text with box-drawing, Greek or division characters is bytes shown as IBM437; other non-ASCII text
    // (accented letters) is a real UTF-8 payload.
    let shows_cp437 = t
        .chars()
        .any(|c| matches!(c, '\u{f7}' | '\u{393}'..='\u{3c9}' | '\u{2190}'..='\u{25ff}'));
    if ty == "PDF417" && shows_cp437 {
        return t
            .chars()
            .map(cp437_byte)
            .collect::<Option<Vec<u8>>>()
            .map(Payload::Bytes);
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
enum Payload {
    Bytes(Vec<u8>),
    Ambiguous,
}

/// Polygon of a VIA shape; an ellipse becomes its bounding box. Empty for unknown shapes.
fn shape_points(shape: &Value) -> Vec<[f64; 2]> {
    let num = |v: &Value| v.as_f64();
    match shape.get("name").and_then(Value::as_str) {
        Some("polygon") => {
            let list = |k: &str| -> Vec<f64> {
                shape
                    .get(k)
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(num).collect())
                    .unwrap_or_default()
            };
            let (xs, ys) = (list("all_points_x"), list("all_points_y"));
            xs.into_iter().zip(ys).map(|(x, y)| [x, y]).collect()
        }
        Some("ellipse") => {
            let get = |k: &str| shape.get(k).and_then(num).unwrap_or(0.0);
            let (cx, cy, rx, ry) = (get("cx"), get("cy"), get("rx"), get("ry"));
            vec![
                [cx - rx, cy - ry],
                [cx + rx, cy - ry],
                [cx + rx, cy + ry],
                [cx - rx, cy + ry],
            ]
        }
        _ => Vec::new(),
    }
}

/// `String` is normally a JSON string but numbers occur in the undecodable regions.
fn attr_string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

/// One image entry of a VIA project: file name, the `size` key suffix and the barcodes.
#[derive(Debug)]
pub struct ViaImage {
    pub file: String,
    pub size: u64,
    pub barcodes: Vec<Barcode>,
}

/// Parses one VIA project file into its image entries, in the file's image order.
pub fn parse_via(json: &str) -> Result<Vec<ViaImage>, String> {
    let doc: Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let meta = doc
        .get("_via_img_metadata")
        .and_then(Value::as_object)
        .ok_or("no _via_img_metadata")?;
    let mut out = Vec::with_capacity(meta.len());
    for entry in meta.values() {
        let file = entry
            .get("filename")
            .and_then(Value::as_str)
            .ok_or("image entry without filename")?
            .to_string();
        let size = entry.get("size").and_then(Value::as_u64).unwrap_or(0);
        let regions = entry.get("regions").and_then(Value::as_array);
        let barcodes = regions
            .into_iter()
            .flatten()
            .map(|r| {
                let attrs = r.get("region_attributes");
                let ty = attr_string(attrs.and_then(|a| a.get("Type")));
                let string = attr_string(attrs.and_then(|a| a.get("String")));
                let (mut format, text) = map_region(ty.trim(), &string);
                let mut bytes_hex = None;
                if format != UNDECODABLE {
                    match legacy_payload(ty.trim(), &text) {
                        Some(Payload::Bytes(b)) => bytes_hex = Some(encode_hex(&b)),
                        Some(Payload::Ambiguous) => format = UNDECODABLE.to_string(),
                        None => {}
                    }
                }
                Barcode {
                    text,
                    format,
                    bytes_hex,
                    points: r
                        .get("shape_attributes")
                        .map(shape_points)
                        .unwrap_or_default(),
                }
            })
            .collect();
        out.push(ViaImage {
            file,
            size,
            barcodes,
        });
    }
    Ok(out)
}

/// Loads every VIA file below `via` as one [`Source`] per file (named after the file). Images
/// named in several files stay with the first source and receive the union of the regions;
/// regions of the same symbol are merged later by the common GT de-duplication. When a file
/// names the same image twice (different `size` keys), the entries whose `size` equals the
/// image's pixel count win, because the other ones are annotations of another resolution.
pub fn load_sources(via: &Path) -> Result<Vec<Source>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(via)
        .map_err(|e| format!("{}: {e}", via.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .collect();
    files.sort();
    let images = image_dir(via);
    let mut owner: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut sources: Vec<Source> = Vec::new();
    for path in files {
        let json =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let entries = parse_via(&json).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let this = sources.len();
        // Same file twice in one project: keep the entries made for the real resolution.
        let mut per_file: BTreeMap<String, Vec<ViaImage>> = BTreeMap::new();
        for e in entries {
            per_file.entry(e.file.clone()).or_default().push(e);
        }
        let mut annotations = Annotations {
            format: "barcode-benchmark/via".into(),
            images: Vec::new(),
        };
        for (file, mut group) in per_file {
            if group.len() > 1 {
                let pixels = image::ImageReader::open(images.join(&file))
                    .ok()
                    .and_then(|r| r.with_guessed_format().ok())
                    .and_then(|r| r.into_dimensions().ok())
                    .map(|(w, h)| u64::from(w) * u64::from(h));
                if group.iter().any(|g| Some(g.size) == pixels) {
                    group.retain(|g| Some(g.size) == pixels);
                }
            }
            let barcodes: Vec<Barcode> = group.into_iter().flat_map(|g| g.barcodes).collect();
            match owner.get(&file) {
                Some(&(s, i)) => sources[s].annotations.images[i].barcodes.extend(barcodes),
                None => {
                    owner.insert(file.clone(), (this, annotations.images.len()));
                    annotations.images.push(ImageEntry { file, barcodes });
                }
            }
        }
        sources.push(Source { name, annotations });
    }
    Ok(sources)
}

/// Maps polygons from the EXIF-corrected frame the VIA annotations use back to the raw pixel
/// frame of a `width` x `height` image that the engines decode. Only the rotations of 90, 180 and
/// 270 degrees occur in BarBeR (EXIF 6, 3 and 8: 170, 2 and 5 images); mirrored orientations
/// are left unchanged.
pub fn to_raw_frame(
    polys: &[Vec<[f64; 2]>],
    orientation: image::metadata::Orientation,
    width: u32,
    height: u32,
) -> Vec<Vec<[f64; 2]>> {
    use image::metadata::Orientation::{NoTransforms, Rotate90, Rotate180, Rotate270};
    let (w, h) = (f64::from(width), f64::from(height));
    let map = |p: [f64; 2]| match orientation {
        Rotate90 => [p[1], h - p[0]],
        Rotate180 => [w - p[0], h - p[1]],
        Rotate270 => [w - p[1], p[0]],
        NoTransforms => p,
        _ => p,
    };
    polys
        .iter()
        .map(|poly| poly.iter().map(|&p| map(p)).collect())
        .collect()
}

/// Splits an image's barcodes into the polygons of undecodable regions and the others.
pub fn split_undecodable(barcodes: Vec<Barcode>) -> (Vec<Barcode>, Vec<Vec<[f64; 2]>>) {
    let (masked, expected): (Vec<_>, Vec<_>) =
        barcodes.into_iter().partition(|b| b.format == UNDECODABLE);
    (expected, masked.into_iter().map(|b| b.points).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groundtruth::map_symbology;
    use breader::Symbology;

    fn m(ty: &str, s: &str) -> (String, String) {
        map_region(ty, s)
    }

    fn sym(ty: &str, s: &str) -> Option<Symbology> {
        map_symbology(&m(ty, s).0)
    }

    #[test]
    fn types_map_to_symbologies() {
        assert_eq!(sym("EAN13", "8005200006304"), Some(Symbology::Ean13));
        assert_eq!(sym("EAN8", "00050555"), Some(Symbology::Ean8));
        assert_eq!(sym("C128", "99HLC017-00"), Some(Symbology::Code128));
        assert_eq!(sym("UCC128", "200936382755"), Some(Symbology::Code128));
        assert_eq!(sym("C39", "*8802041*"), Some(Symbology::Code39));
        assert_eq!(sym("I2O5", "05017601700272"), Some(Symbology::Itf));
        assert_eq!(sym("QR", "hello"), Some(Symbology::Qr));
        assert_eq!(
            sym("DATAMATRIX", "2A127693515"),
            Some(Symbology::DataMatrix)
        );
        assert_eq!(sym("PDF417", "x"), Some(Symbology::Pdf417));
        assert_eq!(sym("AZTEC", "01;Sparpreis"), Some(Symbology::Aztec));
    }

    #[test]
    fn upc_and_mis_filed_ean13_are_told_apart_by_length() {
        assert_eq!(sym("UPCS", "734646964449"), Some(Symbology::UpcA));
        assert_eq!(sym("UPCA", "191376651924"), Some(Symbology::UpcA));
        assert_eq!(sym("UPCS", "01234565"), Some(Symbology::UpcE));
        assert_eq!(sym("EAN13", "807770190186"), Some(Symbology::UpcA));
        assert_eq!(sym("EAN13", "40111896"), Some(Symbology::Ean8));
        // Neither 13, 12 nor 8 digits: not a well-formed value, so no expectation.
        for s in [
            "06843738971",
            "18804284911092",
            "20519734090751900020",
            "ab",
        ] {
            assert_eq!(m("EAN13", s).0, UNDECODABLE, "{s}");
        }
        assert_eq!(m("UPCS", "1234567").0, UNDECODABLE);
    }

    #[test]
    fn unsupported_types_keep_their_own_name() {
        for (ty, name) in [
            ("2-DIGIT", "EAN_2"),
            ("IATA25", "IATA_2_OF_5"),
            ("INTELLIGENTMAIL", "INTELLIGENT_MAIL"),
            ("JAPANPOST", "JAPAN_POST"),
            ("KIX", "KIX"),
            ("POSTNET", "POSTNET"),
            ("ROYALMAILCODE", "ROYAL_MAIL"),
        ] {
            let (format, _) = m(ty, "1234");
            assert_eq!(format, name);
            assert_eq!(map_symbology(&format), None, "{name}");
        }
    }

    #[test]
    fn undecodable_and_unknown_values_are_masked() {
        assert_eq!(m("1D", "-1").0, UNDECODABLE);
        assert_eq!(m("1D", "9037226").0, UNDECODABLE);
        assert_eq!(m("-1", "-1").0, UNDECODABLE);
        assert_eq!(m("EAN13", "-1").0, UNDECODABLE);
        assert_eq!(m("QR", "-1").0, UNDECODABLE);
        assert_eq!(m("JAPANPOST", "^").0, UNDECODABLE);
        assert_eq!(m("DATAMATRIX", "^\t\t\t").0, UNDECODABLE);
        assert_eq!(m("C128", "").0, UNDECODABLE);
        // A caret inside a value is data.
        assert_eq!(m("I2O5", "^7641801002510").0, "ITF");
    }

    #[test]
    fn strings_are_normalised() {
        assert_eq!(m("C39", "*S015610095C*").1, "S015610095C");
        assert_eq!(m("C39", "ABC*").1, "ABC*");
        assert_eq!(m("PDF417", "A &gt; B &amp; C &lt;").1, "A > B & C <");
        assert_eq!(m("QR", "a&b; &#65;&#x42;").1, "a&b; AB");
        assert_eq!(
            m("UCC128", "(01)12345678901231(10)AB").1,
            "0112345678901231".to_string() + "10AB"
        );
    }

    #[test]
    fn zvz_binary_payloads_are_turned_back_into_bytes() {
        // Hex of UTF-16 units; NUL shown as '^', 0xB4 as the IBM437 character U+2524.
        assert_eq!(
            legacy_payload("PDF417", "000200252524004100FC"),
            Some(Payload::Bytes(vec![2, 0x25, 0xB4, 0x41, 0x81]))
        );
        // Units that all fit a byte are the byte values themselves (Latin-1 style).
        assert_eq!(
            legacy_payload("DATAMATRIX", "00980014004100C9"),
            Some(Payload::Bytes(vec![0x98, 0x14, 0x41, 0xC9]))
        );
        // A '^' may be a NUL or a caret: the bytes are unknown.
        assert_eq!(
            legacy_payload("PDF417", "0002005E0041"),
            Some(Payload::Ambiguous)
        );
        // Characters outside code page 437 cannot be recovered: plain text.
        assert_eq!(legacy_payload("DATAMATRIX", "91700467"), None);
        // IBM437 text: the sigma is 0xE5.
        assert_eq!(
            legacy_payload("PDF417", "A\u{3c3}\u{2524}"),
            Some(Payload::Bytes(vec![0x41, 0xE5, 0xB4]))
        );
        // Accented letters alone are a real UTF-8 payload.
        assert_eq!(legacy_payload("PDF417", "Declaraci\u{f3}"), None);
        // Plain text and other types are left alone.
        assert_eq!(legacy_payload("PDF417", "22N0000"), None);
        assert_eq!(legacy_payload("QR", "0002005E0002005E"), None);
    }

    #[test]
    fn polygons_return_to_the_raw_frame() {
        use image::metadata::Orientation;
        // Raw 40 wide, 30 high; the point (10, 5) of the raw frame is (25, 10) when rotated by
        // 90 degrees clockwise (EXIF 6), (30, 25) by 180 and (5, 30) by 270.
        let raw = [10.0, 5.0];
        for (o, shown) in [
            (Orientation::Rotate90, [25.0, 10.0]),
            (Orientation::Rotate180, [30.0, 25.0]),
            (Orientation::Rotate270, [5.0, 30.0]),
            (Orientation::NoTransforms, raw),
        ] {
            let back = to_raw_frame(&[vec![shown]], o, 40, 30);
            assert_eq!(back, vec![vec![raw]], "{o:?}");
        }
    }

    #[test]
    fn via_regions_become_barcodes() {
        let json = r#"{"_via_img_metadata": {
          "a.jpg100": {"filename": "a.jpg", "size": 100, "regions": [
            {"shape_attributes": {"name": "polygon", "all_points_x": [1,2,3,4], "all_points_y": [5,6,7,8]},
             "region_attributes": {"Type": "UPCS", "PPE": 2.5, "String": "734646964449"}},
            {"shape_attributes": {"name": "ellipse", "cx": 10, "cy": 20, "rx": 2, "ry": 3, "theta": 0},
             "region_attributes": {"Type": "1D", "PPE": -1, "String": "-1"}}
          ]},
          "b.jpg1": {"filename": "b.jpg", "size": 1, "regions": []}
        }}"#;
        let v = parse_via(json).unwrap();
        assert_eq!(v.len(), 2);
        assert_eq!(v[0].barcodes[0].format, "UPC_A");
        assert_eq!(v[0].barcodes[0].points[2], [3.0, 7.0]);
        assert_eq!(v[0].barcodes[1].format, UNDECODABLE);
        assert_eq!(v[0].barcodes[1].points[0], [8.0, 17.0]);
        assert!(v[1].barcodes.is_empty());
        let (expected, masked) = split_undecodable(v.into_iter().next().unwrap().barcodes);
        assert_eq!((expected.len(), masked.len()), (1, 1));
    }
}
