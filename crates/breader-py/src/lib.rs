use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use breader::{
    Code39Options, Content, EanAddonPolicy, Effort, Escalation, ItfOptions, PixelFormat,
    Reader as CoreReader, ScanOptions, Symbol as CoreSymbol, SymbolMeta, Symbology, SymbologySet,
    gs1,
};
use pyo3::exceptions::{PyOSError, PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBytes, PyDict, PyList, PyMemoryView, PyString, PyTuple};

const FORMATS: &str = "luma8, rgb8, rgba8, bgra8, nv12, nv21, i420";

fn value_err(msg: impl Into<String>) -> PyErr {
    PyValueError::new_err(msg.into())
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
        _ => s.name(),
    }
}

fn parse_symbology(name: &str) -> PyResult<Symbology> {
    let key = name.to_ascii_lowercase().replace(['-', '_', ' '], "");
    Symbology::ALL
        .iter()
        .copied()
        .find(|s| symbology_id(*s).replace('-', "") == key || (key == "qrcode" && *s == Symbology::Qr))
        .ok_or_else(|| {
            let known: Vec<_> = Symbology::ALL.iter().map(|s| symbology_id(*s)).collect();
            value_err(format!(
                "unknown symbology {name:?}, expected one of {}",
                known.join(", ")
            ))
        })
}

fn parse_effort(name: &str) -> PyResult<Effort> {
    match name {
        "fast" => Ok(Effort::Fast),
        "balanced" => Ok(Effort::Balanced),
        "exhaustive" => Ok(Effort::Exhaustive),
        _ => Err(value_err(format!(
            "unknown effort {name:?}, expected fast, balanced or exhaustive"
        ))),
    }
}

fn effort_name(e: Effort) -> &'static str {
    match e {
        Effort::Fast => "fast",
        Effort::Balanced => "balanced",
        Effort::Exhaustive => "exhaustive",
    }
}

fn parse_escalation(name: &str) -> PyResult<Escalation> {
    match name {
        "all" => Ok(Escalation::All),
        "found" => Ok(Escalation::UntilFound),
        "stable" => Ok(Escalation::UntilStable),
        _ => Err(value_err(format!(
            "unknown escalation {name:?}, expected all, found or stable"
        ))),
    }
}

fn escalation_name(e: Escalation) -> &'static str {
    match e {
        Escalation::All => "all",
        Escalation::UntilFound => "found",
        Escalation::UntilStable => "stable",
    }
}

fn parse_addon(name: &str) -> PyResult<EanAddonPolicy> {
    match name {
        "ignore" => Ok(EanAddonPolicy::Ignore),
        "optional" => Ok(EanAddonPolicy::Optional),
        "required" => Ok(EanAddonPolicy::Required),
        _ => Err(value_err(format!(
            "unknown ean_addon {name:?}, expected ignore, optional or required"
        ))),
    }
}

fn addon_name(p: EanAddonPolicy) -> &'static str {
    match p {
        EanAddonPolicy::Ignore => "ignore",
        EanAddonPolicy::Optional => "optional",
        EanAddonPolicy::Required => "required",
    }
}

fn parse_format(name: &str) -> PyResult<PixelFormat> {
    match name {
        "luma8" => Ok(PixelFormat::Luma8),
        "rgb8" => Ok(PixelFormat::Rgb8),
        "rgba8" => Ok(PixelFormat::Rgba8),
        "bgra8" => Ok(PixelFormat::Bgra8),
        "nv12" => Ok(PixelFormat::Nv12),
        "nv21" => Ok(PixelFormat::Nv21),
        "i420" => Ok(PixelFormat::I420),
        _ => Err(value_err(format!(
            "unknown pixel format {name:?}, expected one of {FORMATS}"
        ))),
    }
}

fn positive(name: &str, v: i64) -> PyResult<usize> {
    usize::try_from(v)
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| value_err(format!("{name} must be a positive integer, got {v}")))
}

#[pyclass(frozen, name = "Symbol", module = "breader")]
struct Symbol {
    symbology: &'static str,
    data: Vec<u8>,
    text: Option<String>,
    content: &'static str,
    gs1: Option<Vec<(String, String)>>,
    gs1_hri: Option<String>,
    corners: Vec<(f32, f32)>,
    orientation: f32,
    aim_id: String,
    quality: f32,
    meta: SymbolMeta,
}

impl From<CoreSymbol> for Symbol {
    fn from(s: CoreSymbol) -> Self {
        let (content, gs1, gs1_hri) = match &s.content {
            Content::Text => ("text", None, None),
            Content::Binary => ("binary", None, None),
            Content::Url => ("url", None, None),
            Content::Gs1(elements) => (
                "gs1",
                Some(
                    elements
                        .iter()
                        .map(|e| (e.ai.clone(), e.value.clone()))
                        .collect(),
                ),
                Some(gs1::hri(elements)),
            ),
            _ => ("unparsed", None, None),
        };
        let aim: Vec<u8> = s.aim_id.iter().copied().filter(|b| *b != 0).collect();
        Self {
            symbology: symbology_id(s.symbology),
            data: s.data,
            text: s.text,
            content,
            gs1,
            gs1_hri,
            corners: s.quad.corners.iter().map(|p| (p.x, p.y)).collect(),
            orientation: s.orientation_deg,
            aim_id: String::from_utf8_lossy(&aim).into_owned(),
            quality: s.quality,
            meta: s.meta,
        }
    }
}

#[pymethods]
impl Symbol {
    /// Short symbology id: "qr", "ean13", "ean8", "upca", "upce", "code128", "code39", "itf", "datamatrix", "pdf417", "aztec" or "codabar".
    #[getter]
    fn symbology(&self) -> &str {
        self.symbology
    }

    /// Raw payload bytes.
    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    /// Payload as text, or None when the charset is ambiguous.
    #[getter]
    fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// One of "text", "binary", "url", "gs1" or "unparsed".
    #[getter]
    fn content(&self) -> &str {
        self.content
    }

    /// GS1 element strings as (AI, value) pairs, or None when not GS1.
    #[getter]
    fn gs1(&self) -> Option<Vec<(String, String)>> {
        self.gs1.clone()
    }

    /// Human readable GS1 form such as "(01)09506000134352(17)261231", or None.
    #[getter]
    fn gs1_hri(&self) -> Option<&str> {
        self.gs1_hri.as_deref()
    }

    /// Four (x, y) corner points in image coordinates.
    #[getter]
    fn corners(&self) -> Vec<(f32, f32)> {
        self.corners.clone()
    }

    /// Rotation in degrees.
    #[getter]
    fn orientation(&self) -> f32 {
        self.orientation
    }

    /// ISO/IEC 15424 symbology identifier, e.g. "]Q1".
    #[getter]
    fn aim_id(&self) -> &str {
        &self.aim_id
    }

    /// Decoder quality estimate in [0, 1]; informational only.
    #[getter]
    fn quality(&self) -> f32 {
        self.quality
    }

    /// Symbology specific details as a dict with a "kind" key ("qr", "linear", "datamatrix", "pdf417" or "aztec").
    #[getter]
    fn meta<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        match &self.meta {
            SymbolMeta::Qr(m) => {
                d.set_item("kind", "qr")?;
                d.set_item("version", m.version)?;
                d.set_item("width", m.width)?;
                d.set_item("height", m.height)?;
                d.set_item("ec_level", format!("{:?}", m.ec_level))?;
                d.set_item("mask", m.mask)?;
                d.set_item("errors_corrected", m.errors_corrected)?;
                d.set_item("eci", m.eci.clone())?;
                d.set_item("mirrored", m.mirrored)?;
            }
            SymbolMeta::Linear(m) => {
                d.set_item("kind", "linear")?;
                d.set_item("addon", m.addon.as_deref())?;
                d.set_item("checksum_verified", m.checksum_verified)?;
                d.set_item("lines_agreeing", m.lines_agreeing)?;
                d.set_item(
                    "codabar_start_stop",
                    m.codabar_start_stop
                        .map(|(a, b)| (char::from(a).to_string(), char::from(b).to_string())),
                )?;
            }
            SymbolMeta::DataMatrix(m) => {
                d.set_item("kind", "datamatrix")?;
                d.set_item("rows", m.rows)?;
                d.set_item("cols", m.cols)?;
                d.set_item("errors_corrected", m.errors_corrected)?;
                d.set_item("mirrored", m.mirrored)?;
                d.set_item("eci", m.eci.clone())?;
                d.set_item("structured_append", m.structured_append)?;
                d.set_item("reader_init", m.reader_init)?;
            }
            SymbolMeta::Pdf417(m) => {
                d.set_item("kind", "pdf417")?;
                d.set_item("rows", m.rows)?;
                d.set_item("cols", m.cols)?;
                d.set_item("ec_level", m.ec_level)?;
                d.set_item("errors_corrected", m.errors_corrected)?;
                d.set_item("erasures", m.erasures)?;
                d.set_item("compact", m.compact)?;
                d.set_item("reader_init", m.reader_init)?;
                match &m.macro_pdf417 {
                    Some(mc) => {
                        let md = PyDict::new(py);
                        md.set_item("segment_index", mc.segment_index)?;
                        md.set_item("file_id", &mc.file_id)?;
                        md.set_item("last_segment", mc.last_segment)?;
                        md.set_item("file_name", mc.file_name.as_deref())?;
                        md.set_item("segment_count", mc.segment_count)?;
                        md.set_item("timestamp", mc.timestamp)?;
                        md.set_item("sender", mc.sender.as_deref())?;
                        md.set_item("addressee", mc.addressee.as_deref())?;
                        md.set_item("file_size", mc.file_size)?;
                        md.set_item("checksum", mc.checksum)?;
                        d.set_item("macro", md)?;
                    }
                    None => d.set_item("macro", py.None())?,
                }
            }
            SymbolMeta::Aztec(m) => {
                d.set_item("kind", "aztec")?;
                d.set_item("compact", m.compact)?;
                d.set_item("layers", m.layers)?;
                d.set_item("data_codewords", m.data_codewords)?;
                d.set_item("ec_codewords", m.ec_codewords)?;
                d.set_item("errors_corrected", m.errors_corrected)?;
                d.set_item("erasures", m.erasures)?;
                d.set_item("mirrored", m.mirrored)?;
                d.set_item("rune", m.rune)?;
                d.set_item("reader_init", m.reader_init)?;
                d.set_item("eci", m.eci.clone())?;
            }
            _ => d.set_item("kind", "unknown")?,
        }
        Ok(d)
    }

    fn __eq__(&self, other: &Bound<'_, PyAny>) -> bool {
        other
            .cast::<Symbol>()
            .is_ok_and(|o| o.get().symbology == self.symbology && o.get().data == self.data)
    }

    fn __hash__(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.symbology.hash(&mut h);
        self.data.hash(&mut h);
        h.finish()
    }

    fn __repr__(&self) -> String {
        let payload = match &self.text {
            Some(t) => format!("{t:?}"),
            None => format!("<{} bytes>", self.data.len()),
        };
        format!("Symbol(symbology={:?}, data={payload})", self.symbology)
    }
}

struct Geometry {
    format: PixelFormat,
    width: usize,
    height: usize,
    stride: usize,
}

fn is_yuv(f: PixelFormat) -> bool {
    matches!(f, PixelFormat::Nv12 | PixelFormat::Nv21 | PixelFormat::I420)
}

fn check_given(name: &str, given: Option<i64>, actual: usize) -> PyResult<()> {
    match given {
        Some(v) if usize::try_from(v).ok() != Some(actual) => Err(value_err(format!(
            "{name}={v} does not match the buffer shape ({actual})"
        ))),
        _ => Ok(()),
    }
}

fn geometry(
    shape: &[usize],
    format: Option<PixelFormat>,
    width: Option<i64>,
    height: Option<i64>,
    stride: Option<i64>,
) -> PyResult<Geometry> {
    if shape.len() > 1 && stride.is_some() {
        return Err(value_err("stride is only accepted for 1D buffers"));
    }
    let fixed = |format: PixelFormat, w: usize, h: usize| {
        check_given("width", width, w)?;
        check_given("height", height, h)?;
        Ok(Geometry {
            format,
            width: w,
            height: h,
            stride: w * format.bytes_per_pixel(),
        })
    };
    match *shape {
        [_] => {
            let (Some(w), Some(h)) = (width, height) else {
                return Err(value_err(
                    "a 1D buffer needs width and height (or pass a 2D/3D array)",
                ));
            };
            let format = format.unwrap_or(PixelFormat::Luma8);
            let (w, h) = (positive("width", w)?, positive("height", h)?);
            let stride = match stride {
                Some(s) => positive("stride", s)?,
                None => w.saturating_mul(format.bytes_per_pixel()),
            };
            Ok(Geometry {
                format,
                width: w,
                height: h,
                stride,
            })
        }
        [rows, cols] => match format.unwrap_or(PixelFormat::Luma8) {
            PixelFormat::Luma8 => fixed(PixelFormat::Luma8, cols, rows),
            f if is_yuv(f) => {
                if rows % 3 != 0 {
                    return Err(value_err(
                        "a 2D YUV buffer must have 3/2 times as many rows as the image height",
                    ));
                }
                fixed(f, cols, rows / 3 * 2)
            }
            f => Err(value_err(format!(
                "a 2D buffer cannot hold {f:?} pixels, pass a (height, width, channels) array"
            ))),
        },
        [rows, cols, channels] => {
            let f = match (channels, format) {
                (1, None | Some(PixelFormat::Luma8)) => PixelFormat::Luma8,
                (3, None | Some(PixelFormat::Rgb8)) => PixelFormat::Rgb8,
                (4, None) => PixelFormat::Rgba8,
                (4, Some(f @ (PixelFormat::Rgba8 | PixelFormat::Bgra8))) => f,
                _ => {
                    return Err(value_err(format!(
                        "a (height, width, {channels}) buffer does not match format {:?}; \
                         3 channels mean rgb8, 4 mean rgba8 or bgra8 (bgr8 is not supported)",
                        format.map_or("default".to_owned(), |f| format!("{f:?}"))
                    )));
                }
            };
            fixed(f, cols, rows)
        }
        _ => Err(value_err(format!(
            "unsupported buffer rank {}, expected 1, 2 or 3 dimensions",
            shape.len()
        ))),
    }
}

fn load_pixels<'py>(image: &Bound<'py, PyAny>) -> PyResult<(Bound<'py, PyBytes>, Vec<usize>)> {
    if let Ok(b) = image.cast::<PyBytes>() {
        let len = b.as_bytes().len();
        return Ok((b.clone(), vec![len]));
    }
    let not_u8 = |detail: String| {
        value_err(format!(
            "image must support the buffer protocol with unsigned 8-bit items \
             (e.g. a uint8 numpy array, bytes or bytearray): {detail}"
        ))
    };
    let view = PyMemoryView::from(image).map_err(|e| not_u8(e.to_string()))?;
    let item_format: String = view.getattr("format")?.extract()?;
    if item_format != "B" && item_format != "=B" {
        return Err(not_u8(format!("got item format {item_format:?}")));
    }
    let shape: Vec<usize> = view.getattr("shape")?.extract()?;
    if shape.is_empty() {
        return Err(value_err("0-dimensional buffers are not supported"));
    }
    // One copy in C order: detaches from the GIL safely and handles any strides.
    let packed = view.call_method0("tobytes")?.cast_into::<PyBytes>()?;
    Ok((packed, shape))
}

#[pyclass(name = "Reader", module = "breader")]
struct Reader {
    inner: Mutex<CoreReader>,
}

impl Reader {
    fn lock(&self) -> MutexGuard<'_, CoreReader> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn finish(
    py: Python<'_>,
    f: impl FnOnce() -> PyResult<Vec<CoreSymbol>> + Send,
) -> PyResult<Vec<Symbol>> {
    let outcome = py.detach(|| catch_unwind(AssertUnwindSafe(f)));
    match outcome {
        Ok(result) => Ok(result?.into_iter().map(Symbol::from).collect()),
        Err(_) => Err(PyRuntimeError::new_err("internal error while scanning")),
    }
}

#[pymethods]
impl Reader {
    #[new]
    #[pyo3(signature = (
        effort = "balanced",
        symbologies = None,
        *,
        try_invert = None,
        escalation = None,
        max_symbols = 0,
        time_budget_ms = None,
        upscale_max_side = None,
        min_line_agreement = 2,
        ean_addon = "ignore",
        code39_full_ascii = false,
        code39_checksum = false,
        code39_min_length = 3,
        itf_lengths = None,
        itf_min_length = 6,
    ))]
    #[allow(clippy::too_many_arguments)]
    fn new(
        effort: &str,
        symbologies: Option<&Bound<'_, PyAny>>,
        try_invert: Option<bool>,
        escalation: Option<&str>,
        max_symbols: i64,
        time_budget_ms: Option<f64>,
        upscale_max_side: Option<i64>,
        min_line_agreement: i64,
        ean_addon: &str,
        code39_full_ascii: bool,
        code39_checksum: bool,
        code39_min_length: i64,
        itf_lengths: Option<Vec<i64>>,
        itf_min_length: i64,
    ) -> PyResult<Self> {
        let mut options = ScanOptions::with_effort(parse_effort(effort)?);
        if let Some(sel) = symbologies {
            let names: Vec<String> = if sel.is_instance_of::<PyString>() {
                vec![sel.extract()?]
            } else {
                sel.extract().map_err(|_| {
                    value_err("symbologies must be a string or a sequence of strings")
                })?
            };
            if names.is_empty() {
                return Err(value_err("symbologies must not be empty"));
            }
            options.symbologies = names
                .iter()
                .map(|n| parse_symbology(n))
                .collect::<PyResult<SymbologySet>>()?;
        }
        options.try_invert = try_invert;
        options.escalation = escalation.map(parse_escalation).transpose()?;
        options.max_symbols = usize::try_from(max_symbols)
            .map_err(|_| value_err(format!("max_symbols must be >= 0, got {max_symbols}")))?;
        options.time_budget = match time_budget_ms {
            None => None,
            Some(ms) if ms.is_finite() && ms > 0.0 => Some(Duration::from_secs_f64(ms / 1000.0)),
            Some(ms) => {
                return Err(value_err(format!(
                    "time_budget_ms must be a positive number, got {ms}"
                )));
            }
        };
        options.upscale_retry = upscale_max_side
            .map(|v| {
                usize::try_from(v)
                    .map_err(|_| value_err(format!("upscale_max_side must be >= 0, got {v}")))
            })
            .transpose()?;
        options.min_line_agreement =
            u16::try_from(positive("min_line_agreement", min_line_agreement)?)
                .map_err(|_| value_err("min_line_agreement is too large"))?;
        options.ean_addon = parse_addon(ean_addon)?;
        options.code39 = Code39Options {
            full_ascii: code39_full_ascii,
            require_checksum: code39_checksum,
            min_length: positive("code39_min_length", code39_min_length)?,
        };
        let mut itf = ItfOptions {
            min_length: positive("itf_min_length", itf_min_length)?,
            ..ItfOptions::default()
        };
        if let Some(lengths) = itf_lengths {
            itf.allowed_lengths = lengths
                .into_iter()
                .map(|l| {
                    let l = positive("itf_lengths entries", l)?;
                    if l % 2 != 0 {
                        return Err(value_err(format!(
                            "itf_lengths entries must be even, got {l}"
                        )));
                    }
                    Ok(l)
                })
                .collect::<PyResult<_>>()?;
        }
        options.itf = itf;
        Ok(Self {
            inner: Mutex::new(CoreReader::with_options(options)),
        })
    }

    /// Effective options as a new dict (read-only snapshot).
    #[getter]
    fn options<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let o = self.lock().options().clone();
        let d = PyDict::new(py);
        d.set_item("effort", effort_name(o.effort))?;
        let names: Vec<_> = o.symbologies.iter().map(symbology_id).collect();
        d.set_item("symbologies", PyList::new(py, names)?)?;
        d.set_item("try_invert", o.try_invert)?;
        d.set_item("escalation", o.escalation.map(escalation_name))?;
        d.set_item("max_symbols", o.max_symbols)?;
        d.set_item(
            "time_budget_ms",
            o.time_budget.map(|t| t.as_secs_f64() * 1000.0),
        )?;
        d.set_item("upscale_max_side", o.upscale_retry)?;
        d.set_item("min_line_agreement", o.min_line_agreement)?;
        d.set_item("ean_addon", addon_name(o.ean_addon))?;
        d.set_item("code39_full_ascii", o.code39.full_ascii)?;
        d.set_item("code39_checksum", o.code39.require_checksum)?;
        d.set_item("code39_min_length", o.code39.min_length)?;
        d.set_item(
            "itf_lengths",
            PyTuple::new(py, o.itf.allowed_lengths.iter().copied())?,
        )?;
        d.set_item("itf_min_length", o.itf.min_length)?;
        Ok(d)
    }

    /// Scans an image given as any uint8 buffer (numpy array, bytes, bytearray, memoryview).
    #[pyo3(signature = (image, *, format = None, width = None, height = None, stride = None))]
    fn scan<'py>(
        &self,
        py: Python<'py>,
        image: &Bound<'py, PyAny>,
        format: Option<&str>,
        width: Option<i64>,
        height: Option<i64>,
        stride: Option<i64>,
    ) -> PyResult<Vec<Symbol>> {
        let format = format.map(parse_format).transpose()?;
        let (pixels, shape) = load_pixels(image)?;
        let g = geometry(&shape, format, width, height, stride)?;
        let data = pixels.as_bytes();
        finish(py, || {
            self.lock()
                .scan_pixels(data, g.format, g.width, g.height, g.stride)
                .map_err(|e| value_err(e.to_string()))
        })
    }

    /// Decodes an image file (anything the `image` crate supports) and scans it.
    fn scan_file(&self, py: Python<'_>, path: PathBuf) -> PyResult<Vec<Symbol>> {
        finish(py, || {
            self.lock()
                .scan_file(&path)
                .map_err(|e| PyOSError::new_err(e.to_string()))
        })
    }

    fn __repr__(&self) -> String {
        let l = self.lock();
        let o = l.options();
        format!("Reader(effort={:?})", effort_name(o.effort))
    }
}

#[pymodule(name = "_breader")]
fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Reader>()?;
    m.add_class::<Symbol>()?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
