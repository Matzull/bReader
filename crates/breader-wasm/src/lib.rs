//! WebAssembly / JavaScript bindings for breader.
//!
//! The browser decodes image files; this crate scans pixel buffers (`ImageData` from a canvas, or
//! luma / RGB / YUV bytes). Option names are the Python ones in camelCase.

#![forbid(unsafe_code)]

use breader::{
    Content, EanAddonPolicy, Effort, PixelFormat, Reader as CoreReader, ScanOptions,
    Symbol as CoreSymbol, Symbology, SymbologySet, gs1,
};
use js_sys::{Array, Object, Reflect, Uint8Array};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
export type SymbologyId = "qr" | "ean13" | "ean8" | "upca" | "upce" | "code128" | "code39" | "itf" | "datamatrix" | "pdf417" | "aztec" | "codabar" | "micro-qr" | "rmqr" | "databar" | "databar-limited" | "databar-expanded" | "code93";
export type PixelFormatId = "luma8" | "rgb8" | "rgba8" | "bgra8" | "nv12" | "nv21" | "i420";

export interface ReaderOptions {
  /** "fast", "balanced" (default) or "exhaustive". */
  effort?: "fast" | "balanced" | "exhaustive";
  /** Symbologies to look for (default: all). */
  symbologies?: SymbologyId[];
  /** Also look for light-on-dark symbols; default: decided by the effort. */
  tryInvert?: boolean;
  /** Stop after this many symbols; 0 (default) means no limit. */
  maxSymbols?: number;
  /** Not supported on WebAssembly (no clock): setting it throws. */
  timeBudgetMs?: number;
  /** "ignore" (default), "optional" or "required". */
  eanAddon?: "ignore" | "optional" | "required";
  code39FullAscii?: boolean;
  code39Checksum?: boolean;
}

export interface Point { x: number; y: number }

export interface BarcodeSymbol {
  symbology: SymbologyId;
  /** Payload as text, or null when the charset is ambiguous. */
  text: string | null;
  /** Raw payload bytes. */
  data: Uint8Array;
  content: "text" | "binary" | "url" | "gs1" | "unparsed";
  /** Human readable GS1 form such as "(01)09506000134352(17)261231", or null. */
  gs1Hri: string | null;
  /** Four corner points in image coordinates. */
  corners: Point[];
  /** Rotation in degrees. */
  orientation: number;
  /** ISO/IEC 15424 symbology identifier, e.g. "]Q1". */
  aimId: string;
  /** Decoder quality estimate in [0, 1]; informational only. */
  quality: number;
}

export interface ImageDataLike {
  readonly data: Uint8ClampedArray | Uint8Array;
  readonly width: number;
  readonly height: number;
}
"#;

#[wasm_bindgen]
extern "C" {
    /// A plain JS object with the reader options.
    #[wasm_bindgen(typescript_type = "ReaderOptions")]
    pub type JsOptions;
    /// An array of symbols.
    #[wasm_bindgen(typescript_type = "BarcodeSymbol[]")]
    pub type JsSymbols;
    /// A pixel format id.
    #[wasm_bindgen(typescript_type = "PixelFormatId")]
    pub type JsFormat;
    /// Anything with `data`, `width` and `height`, as `ImageData`.
    #[wasm_bindgen(typescript_type = "ImageDataLike")]
    pub type JsImageData;
}

const FORMATS: &str = "luma8, rgb8, rgba8, bgra8, nv12, nv21, i420";
const OPTION_KEYS: &[&str] = &[
    "effort",
    "symbologies",
    "tryInvert",
    "maxSymbols",
    "timeBudgetMs",
    "eanAddon",
    "code39FullAscii",
    "code39Checksum",
];

/// The options as read from JS, before validation. Plain Rust so that it can be tested natively.
#[derive(Debug, Default, Clone, PartialEq)]
struct RawOptions {
    effort: Option<String>,
    symbologies: Option<Vec<String>>,
    try_invert: Option<bool>,
    max_symbols: Option<f64>,
    time_budget_ms: Option<f64>,
    ean_addon: Option<String>,
    code39_full_ascii: Option<bool>,
    code39_checksum: Option<bool>,
}

fn symbology_id(s: Symbology) -> &'static str {
    match s {
        Symbology::Qr => "qr",
        Symbology::Ean13 => "ean13",
        Symbology::Ean8 => "ean8",
        Symbology::UpcA => "upca",
        Symbology::UpcE => "upce",
        Symbology::Code128 => "code128",
        Symbology::Code39 => "code39",
        Symbology::Itf => "itf",
        Symbology::DataMatrix => "datamatrix",
        Symbology::Pdf417 => "pdf417",
        Symbology::Aztec => "aztec",
        Symbology::Codabar => "codabar",
        Symbology::MicroQr => "micro-qr",
        Symbology::Rmqr => "rmqr",
        Symbology::DataBar => "databar",
        Symbology::DataBarLimited => "databar-limited",
        Symbology::DataBarExpanded => "databar-expanded",
        Symbology::Code93 => "code93",
        _ => s.name(),
    }
}

fn parse_symbology(name: &str) -> Result<Symbology, String> {
    let key = name.to_ascii_lowercase().replace(['-', '_', ' '], "");
    Symbology::ALL
        .iter()
        .copied()
        .find(|s| {
            symbology_id(*s).replace('-', "") == key || (key == "qrcode" && *s == Symbology::Qr)
        })
        .ok_or_else(|| {
            let known: Vec<_> = Symbology::ALL.iter().map(|s| symbology_id(*s)).collect();
            format!(
                "unknown symbology {name:?}, expected one of {}",
                known.join(", ")
            )
        })
}

fn parse_effort(name: &str) -> Result<Effort, String> {
    match name {
        "fast" => Ok(Effort::Fast),
        "balanced" => Ok(Effort::Balanced),
        "exhaustive" => Ok(Effort::Exhaustive),
        _ => Err(format!(
            "unknown effort {name:?}, expected fast, balanced or exhaustive"
        )),
    }
}

fn parse_addon(name: &str) -> Result<EanAddonPolicy, String> {
    match name {
        "ignore" => Ok(EanAddonPolicy::Ignore),
        "optional" => Ok(EanAddonPolicy::Optional),
        "required" => Ok(EanAddonPolicy::Required),
        _ => Err(format!(
            "unknown eanAddon {name:?}, expected ignore, optional or required"
        )),
    }
}

fn parse_format(name: &str) -> Result<PixelFormat, String> {
    match name {
        "luma8" => Ok(PixelFormat::Luma8),
        "rgb8" => Ok(PixelFormat::Rgb8),
        "rgba8" => Ok(PixelFormat::Rgba8),
        "bgra8" => Ok(PixelFormat::Bgra8),
        "nv12" => Ok(PixelFormat::Nv12),
        "nv21" => Ok(PixelFormat::Nv21),
        "i420" => Ok(PixelFormat::I420),
        _ => Err(format!(
            "unknown pixel format {name:?}, expected one of {FORMATS}"
        )),
    }
}

/// A whole number of at least `min`, as the JS number it came in.
fn whole(name: &str, v: f64, min: f64) -> Result<usize, String> {
    if v.is_finite() && v.fract() == 0.0 && v >= min && v <= f64::from(u32::MAX) {
        Ok(v as usize)
    } else {
        Err(format!("{name} must be an integer >= {min}, got {v}"))
    }
}

fn build_options(raw: &RawOptions) -> Result<ScanOptions, String> {
    let effort = raw
        .effort
        .as_deref()
        .map_or(Ok(Effort::Balanced), parse_effort)?;
    let mut options = ScanOptions::with_effort(effort);
    if let Some(names) = &raw.symbologies {
        if names.is_empty() {
            return Err("symbologies must not be empty".into());
        }
        options.symbologies = names
            .iter()
            .map(|n| parse_symbology(n))
            .collect::<Result<SymbologySet, _>>()?;
    }
    options.try_invert = raw.try_invert;
    if let Some(v) = raw.max_symbols {
        options.max_symbols = whole("maxSymbols", v, 0.0)?;
    }
    if raw.time_budget_ms.is_some() {
        return Err(
            "timeBudgetMs is not supported on WebAssembly (there is no monotonic clock); \
             bound the work with effort, symbologies and maxSymbols instead"
                .into(),
        );
    }
    if let Some(a) = &raw.ean_addon {
        options.ean_addon = parse_addon(a)?;
    }
    options.code39.full_ascii = raw.code39_full_ascii.unwrap_or(false);
    options.code39.require_checksum = raw.code39_checksum.unwrap_or(false);
    Ok(options)
}

fn err(msg: impl AsRef<str>) -> JsError {
    JsError::new(msg.as_ref())
}

fn get(obj: &JsValue, key: &str) -> Result<JsValue, JsError> {
    Reflect::get(obj, &JsValue::from_str(key)).map_err(|_| err(format!("cannot read {key:?}")))
}

fn opt_field<T>(
    obj: &JsValue,
    key: &str,
    expected: &str,
    convert: impl FnOnce(&JsValue) -> Option<T>,
) -> Result<Option<T>, JsError> {
    let v = get(obj, key)?;
    if v.is_undefined() {
        return Ok(None);
    }
    convert(&v)
        .map(Some)
        .ok_or_else(|| err(format!("{key} must be {expected}")))
}

fn read_options(value: &JsValue) -> Result<RawOptions, JsError> {
    if value.is_undefined() || value.is_null() {
        return Ok(RawOptions::default());
    }
    if !value.is_object() || Array::is_array(value) {
        return Err(err("options must be a plain object"));
    }
    for key in Object::keys(&Object::from(value.clone())) {
        let key = key.as_string().unwrap_or_default();
        if !OPTION_KEYS.contains(&key.as_str()) {
            return Err(err(format!(
                "unknown option {key:?}, expected one of {}",
                OPTION_KEYS.join(", ")
            )));
        }
    }
    let string = |v: &JsValue| v.as_string();
    let boolean = |v: &JsValue| v.as_bool();
    let number = |v: &JsValue| v.as_f64();
    let symbologies = opt_field(value, "symbologies", "an array of strings", |v| {
        if !Array::is_array(v) {
            return None;
        }
        Array::from(v).iter().map(|x| x.as_string()).collect()
    })?;
    Ok(RawOptions {
        effort: opt_field(value, "effort", "a string", string)?,
        symbologies,
        try_invert: opt_field(value, "tryInvert", "a boolean", boolean)?,
        max_symbols: opt_field(value, "maxSymbols", "a number", number)?,
        time_budget_ms: opt_field(value, "timeBudgetMs", "a number", number)?,
        ean_addon: opt_field(value, "eanAddon", "a string", string)?,
        code39_full_ascii: opt_field(value, "code39FullAscii", "a boolean", boolean)?,
        code39_checksum: opt_field(value, "code39Checksum", "a boolean", boolean)?,
    })
}

fn set(obj: &Object, key: &str, value: impl Into<JsValue>) {
    // Setting a property on a fresh plain object cannot fail.
    let _ = Reflect::set(obj, &JsValue::from_str(key), &value.into());
}

fn symbol_to_js(s: CoreSymbol) -> JsValue {
    let (content, hri) = match &s.content {
        Content::Text => ("text", None),
        Content::Binary => ("binary", None),
        Content::Url => ("url", None),
        Content::Gs1(elements) => ("gs1", Some(gs1::hri(elements))),
        _ => ("unparsed", None),
    };
    let aim: Vec<u8> = s.aim_id.iter().copied().filter(|b| *b != 0).collect();
    let corners = Array::new();
    for p in &s.quad.corners {
        let c = Object::new();
        set(&c, "x", f64::from(p.x));
        set(&c, "y", f64::from(p.y));
        corners.push(&c);
    }
    let o = Object::new();
    set(&o, "symbology", symbology_id(s.symbology));
    set(&o, "text", s.text.map_or(JsValue::NULL, JsValue::from));
    set(&o, "data", Uint8Array::from(s.data.as_slice()));
    set(&o, "content", content);
    set(&o, "gs1Hri", hri.map_or(JsValue::NULL, JsValue::from));
    set(&o, "corners", corners);
    set(&o, "orientation", f64::from(s.orientation_deg));
    set(&o, "aimId", String::from_utf8_lossy(&aim).into_owned());
    set(&o, "quality", f64::from(s.quality));
    o.into()
}

fn symbols_to_js(symbols: Vec<CoreSymbol>) -> JsSymbols {
    let out = Array::new();
    for s in symbols {
        out.push(&symbol_to_js(s));
    }
    out.unchecked_into()
}

fn dimension(name: &str, v: u32) -> Result<usize, JsError> {
    if v == 0 {
        return Err(err(format!("{name} must be a positive integer, got 0")));
    }
    Ok(v as usize)
}

/// Scans pixel buffers for barcodes and QR codes. Reuse one instance; it keeps scratch buffers.
#[wasm_bindgen]
#[derive(Debug)]
pub struct Reader {
    inner: CoreReader,
}

#[wasm_bindgen]
impl Reader {
    /// Creates a reader. `options` is an optional plain object (see `ReaderOptions`); a bad value
    /// throws an `Error`.
    #[wasm_bindgen(constructor)]
    pub fn new(options: Option<JsOptions>) -> Result<Reader, JsError> {
        let raw = match options {
            Some(o) => read_options(&o)?,
            None => RawOptions::default(),
        };
        let options = build_options(&raw).map_err(err)?;
        Ok(Self {
            inner: CoreReader::with_options(options),
        })
    }

    /// Scans a canvas `ImageData` (RGBA).
    #[wasm_bindgen(js_name = scanImageData)]
    pub fn scan_image_data(&mut self, image: JsImageData) -> Result<JsSymbols, JsError> {
        let image: &JsValue = image.as_ref();
        let number = |key: &str| -> Result<u32, JsError> {
            get(image, key)?
                .as_f64()
                .filter(|v| v.fract() == 0.0 && *v >= 0.0 && *v <= f64::from(u32::MAX))
                .map(|v| v as u32)
                .ok_or_else(|| err(format!("image.{key} must be a non-negative integer")))
        };
        let (width, height) = (number("width")?, number("height")?);
        let data = get(image, "data")?;
        let bytes = if let Some(a) = data.dyn_ref::<js_sys::Uint8ClampedArray>() {
            a.to_vec()
        } else if let Some(a) = data.dyn_ref::<Uint8Array>() {
            a.to_vec()
        } else {
            return Err(err("image.data must be a Uint8ClampedArray or Uint8Array"));
        };
        self.scan_bytes(&bytes, width, height, PixelFormat::Rgba8)
    }

    /// Scans a pixel buffer. `format` is one of luma8 (default), rgb8, rgba8, bgra8, nv12, nv21 and
    /// i420; rows are tightly packed.
    pub fn scan(
        &mut self,
        bytes: &[u8],
        width: u32,
        height: u32,
        format: Option<JsFormat>,
    ) -> Result<JsSymbols, JsError> {
        let format = match format {
            None => PixelFormat::Luma8,
            Some(f) => {
                let name = f
                    .as_string()
                    .ok_or_else(|| err("format must be a string"))?;
                parse_format(&name).map_err(err)?
            }
        };
        self.scan_bytes(bytes, width, height, format)
    }
}

impl Reader {
    fn scan_bytes(
        &mut self,
        bytes: &[u8],
        width: u32,
        height: u32,
        format: PixelFormat,
    ) -> Result<JsSymbols, JsError> {
        let (w, h) = (dimension("width", width)?, dimension("height", height)?);
        let stride = w.saturating_mul(format.bytes_per_pixel());
        let symbols = self
            .inner
            .scan_pixels(bytes, format, w, h, stride)
            .map_err(|e| err(e.to_string()))?;
        Ok(symbols_to_js(symbols))
    }
}

/// The version of the library.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> RawOptions {
        RawOptions::default()
    }

    #[test]
    fn defaults_are_balanced_with_every_symbology() {
        let o = build_options(&raw()).unwrap();
        assert_eq!(o.effort, Effort::Balanced);
        assert_eq!(o.max_symbols, 0);
        assert_eq!(o.try_invert, None);
        assert_eq!(o.ean_addon, EanAddonPolicy::Ignore);
        assert!(!o.code39.full_ascii && !o.code39.require_checksum);
    }

    #[test]
    fn every_field_applies() {
        let o = build_options(&RawOptions {
            effort: Some("exhaustive".into()),
            symbologies: Some(vec!["qr".into(), "EAN-13".into(), "micro_qr".into()]),
            try_invert: Some(true),
            max_symbols: Some(3.0),
            time_budget_ms: None,
            ean_addon: Some("required".into()),
            code39_full_ascii: Some(true),
            code39_checksum: Some(true),
        })
        .unwrap();
        assert_eq!(o.effort, Effort::Exhaustive);
        let ids: Vec<_> = o.symbologies.iter().map(symbology_id).collect();
        assert_eq!(ids.len(), 3);
        for id in ["qr", "ean13", "micro-qr"] {
            assert!(ids.contains(&id), "{id}");
        }
        assert_eq!(o.try_invert, Some(true));
        assert_eq!(o.max_symbols, 3);
        assert_eq!(o.ean_addon, EanAddonPolicy::Required);
        assert!(o.code39.full_ascii && o.code39.require_checksum);
    }

    #[test]
    fn bad_values_have_clear_messages() {
        let bad = |r: RawOptions| build_options(&r).unwrap_err();
        let e = bad(RawOptions {
            effort: Some("slow".into()),
            ..raw()
        });
        assert!(e.contains("unknown effort \"slow\""), "{e}");
        let e = bad(RawOptions {
            symbologies: Some(vec!["qr".into(), "nope".into()]),
            ..raw()
        });
        assert!(
            e.contains("unknown symbology \"nope\"") && e.contains("code93"),
            "{e}"
        );
        let e = bad(RawOptions {
            symbologies: Some(vec![]),
            ..raw()
        });
        assert!(e.contains("must not be empty"), "{e}");
        for v in [-1.0, 1.5, f64::NAN, f64::INFINITY] {
            let e = bad(RawOptions {
                max_symbols: Some(v),
                ..raw()
            });
            assert!(e.contains("maxSymbols"), "{e}");
        }
        let e = bad(RawOptions {
            ean_addon: Some("maybe".into()),
            ..raw()
        });
        assert!(e.contains("eanAddon"), "{e}");
        let e = bad(RawOptions {
            time_budget_ms: Some(50.0),
            ..raw()
        });
        assert!(e.contains("timeBudgetMs is not supported"), "{e}");
    }

    #[test]
    fn symbology_ids_round_trip() {
        for s in Symbology::ALL {
            assert_eq!(parse_symbology(symbology_id(*s)), Ok(*s));
        }
        assert_eq!(parse_symbology("QR Code"), Ok(Symbology::Qr));
        assert_eq!(parse_symbology("qrcode"), Ok(Symbology::Qr));
    }

    #[test]
    fn pixel_formats() {
        for (name, f) in [
            ("luma8", PixelFormat::Luma8),
            ("rgb8", PixelFormat::Rgb8),
            ("rgba8", PixelFormat::Rgba8),
            ("bgra8", PixelFormat::Bgra8),
            ("nv12", PixelFormat::Nv12),
            ("nv21", PixelFormat::Nv21),
            ("i420", PixelFormat::I420),
        ] {
            assert_eq!(parse_format(name), Ok(f));
        }
        assert!(parse_format("bgr8").unwrap_err().contains("luma8, rgb8"));
    }

    #[test]
    fn whole_numbers() {
        assert_eq!(whole("n", 0.0, 0.0), Ok(0));
        assert_eq!(whole("n", 7.0, 1.0), Ok(7));
        assert!(whole("n", 0.0, 1.0).is_err());
        assert!(whole("n", 1e12, 0.0).is_err());
    }
}
