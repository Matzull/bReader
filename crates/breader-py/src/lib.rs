use std::sync::{Mutex, MutexGuard};

use breader::{Effort, PixelFormat, Reader as CoreReader, ScanOptions, Symbol as CoreSymbol};
use breader::{Symbology, SymbologySet};
use pyo3::exceptions::{PyOSError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::PyBytes;

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
        _ => s.name(),
    }
}

fn parse_symbology(name: &str) -> PyResult<Symbology> {
    let key = name.to_ascii_lowercase().replace(['-', '_', ' '], "");
    Symbology::ALL
        .iter()
        .copied()
        .find(|s| symbology_id(*s) == key || (key == "qrcode" && *s == Symbology::Qr))
        .ok_or_else(|| PyValueError::new_err(format!("unknown symbology {name:?}")))
}

fn parse_effort(name: &str) -> PyResult<Effort> {
    match name {
        "fast" => Ok(Effort::Fast),
        "balanced" => Ok(Effort::Balanced),
        "exhaustive" => Ok(Effort::Exhaustive),
        _ => Err(PyValueError::new_err(format!(
            "unknown effort {name:?}, expected fast, balanced or exhaustive"
        ))),
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
        _ => Err(PyValueError::new_err(format!(
            "unknown pixel format {name:?}"
        ))),
    }
}

#[pyclass(frozen, name = "Symbol", module = "breader")]
struct Symbol {
    symbology: &'static str,
    data: Vec<u8>,
    text: Option<String>,
    corners: Vec<(f32, f32)>,
    orientation: f32,
    aim_id: String,
    quality: f32,
}

impl From<CoreSymbol> for Symbol {
    fn from(s: CoreSymbol) -> Self {
        let aim: Vec<u8> = s.aim_id.iter().copied().filter(|b| *b != 0).collect();
        Self {
            symbology: symbology_id(s.symbology),
            data: s.data,
            text: s.text,
            corners: s.quad.corners.iter().map(|p| (p.x, p.y)).collect(),
            orientation: s.orientation_deg,
            aim_id: String::from_utf8_lossy(&aim).into_owned(),
            quality: s.quality,
        }
    }
}

#[pymethods]
impl Symbol {
    #[getter]
    fn symbology(&self) -> &str {
        self.symbology
    }

    #[getter]
    fn data<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &self.data)
    }

    #[getter]
    fn text(&self) -> Option<&str> {
        self.text.as_deref()
    }

    #[getter]
    fn corners(&self) -> Vec<(f32, f32)> {
        self.corners.clone()
    }

    #[getter]
    fn orientation(&self) -> f32 {
        self.orientation
    }

    #[getter]
    fn aim_id(&self) -> &str {
        &self.aim_id
    }

    #[getter]
    fn quality(&self) -> f32 {
        self.quality
    }

    fn __repr__(&self) -> String {
        let payload = match &self.text {
            Some(t) => format!("{t:?}"),
            None => format!("<{} bytes>", self.data.len()),
        };
        format!("Symbol(symbology={:?}, data={payload})", self.symbology)
    }
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

#[pymethods]
impl Reader {
    #[new]
    #[pyo3(signature = (effort = "balanced", symbologies = None))]
    fn new(effort: &str, symbologies: Option<Vec<String>>) -> PyResult<Self> {
        let mut options = ScanOptions::with_effort(parse_effort(effort)?);
        if let Some(names) = symbologies {
            options.symbologies = names
                .iter()
                .map(|n| parse_symbology(n))
                .collect::<PyResult<SymbologySet>>()?;
        }
        Ok(Self {
            inner: Mutex::new(CoreReader::with_options(options)),
        })
    }

    #[pyo3(signature = (data, width, height, format = "luma8", stride = None))]
    fn scan(
        &self,
        py: Python<'_>,
        data: &[u8],
        width: usize,
        height: usize,
        format: &str,
        stride: Option<usize>,
    ) -> PyResult<Vec<Symbol>> {
        let format = parse_format(format)?;
        let stride = stride.unwrap_or_else(|| width.saturating_mul(format.bytes_per_pixel()));
        let symbols = py
            .detach(|| self.lock().scan_pixels(data, format, width, height, stride))
            .map_err(|e| PyValueError::new_err(e.to_string()))?;
        Ok(symbols.into_iter().map(Symbol::from).collect())
    }

    fn scan_file(&self, py: Python<'_>, path: &str) -> PyResult<Vec<Symbol>> {
        let symbols = py
            .detach(|| self.lock().scan_file(path))
            .map_err(|e| PyOSError::new_err(e.to_string()))?;
        Ok(symbols.into_iter().map(Symbol::from).collect())
    }
}

#[pymodule(name = "breader")]
fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<Reader>()?;
    m.add_class::<Symbol>()?;
    Ok(())
}
