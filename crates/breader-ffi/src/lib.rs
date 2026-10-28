//! C interface to the breader barcode reader (decision D-064).
//!
//! Opaque handles, explicit ownership, no global state. Every function is null-safe and
//! panic-safe: a panic is caught and turned into [`BREADER_PANIC`] (or a null / zero return
//! for accessors), it never unwinds into the caller. Enumerations are plain `uint32_t`
//! values with named constants, so a value the library does not know can never be
//! undefined behaviour on the Rust side and new values can be added without breaking
//! the ABI. The header `include/breader.h` is generated from this file by cbindgen.

// The C ABI needs raw pointers. This is the only crate of the workspace that allows unsafe
// code; every block is small and has a SAFETY comment (D-064).
#![allow(unsafe_code)]
#![allow(clippy::missing_safety_doc)]

use std::ffi::{CString, c_char};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use breader::{
    Content, EanAddonPolicy, Effort, Error, PixelFormat, Reader, ScanOptions, Symbol, Symbology,
    SymbologySet, gs1,
};

/// A status code: one of the `BREADER_*` status constants.
pub type BreaderStatus = i32;
/// The call succeeded.
pub const BREADER_OK: BreaderStatus = 0;
/// A pointer was null, a value was out of range or the pixel buffer does not match the
/// described image.
pub const BREADER_INVALID_ARGUMENT: BreaderStatus = 1;
/// The request is valid but not available: an image file that cannot be decoded, or
/// `breader_scan_file` in a build without the `image` feature.
pub const BREADER_UNSUPPORTED: BreaderStatus = 2;
/// A panic was caught inside the library. The handles involved stay valid but the call
/// did nothing useful; this is a bug, please report it.
pub const BREADER_PANIC: BreaderStatus = 3;
/// An image file could not be opened or read.
pub const BREADER_IO_ERROR: BreaderStatus = 4;

/// Scan effort: `BREADER_EFFORT_*`.
pub type BreaderEffort = u32;
/// Cheapest preset, for video and live preview.
pub const BREADER_EFFORT_FAST: BreaderEffort = 0;
/// The default preset.
pub const BREADER_EFFORT_BALANCED: BreaderEffort = 1;
/// Slowest preset, most reads.
pub const BREADER_EFFORT_EXHAUSTIVE: BreaderEffort = 2;

/// EAN/UPC add-on policy: `BREADER_ADDON_*`.
pub type BreaderAddonPolicy = u32;
/// Do not look for add-ons (default).
pub const BREADER_ADDON_IGNORE: BreaderAddonPolicy = 0;
/// Report an add-on when present.
pub const BREADER_ADDON_OPTIONAL: BreaderAddonPolicy = 1;
/// Reject EAN/UPC symbols without an add-on.
pub const BREADER_ADDON_REQUIRED: BreaderAddonPolicy = 2;

/// Pixel layout of the buffer given to `breader_scan`: `BREADER_PIXEL_*`.
pub type BreaderPixelFormat = u32;
/// One byte per pixel, grey.
pub const BREADER_PIXEL_LUMA8: BreaderPixelFormat = 0;
/// R, G, B.
pub const BREADER_PIXEL_RGB8: BreaderPixelFormat = 1;
/// R, G, B, A.
pub const BREADER_PIXEL_RGBA8: BreaderPixelFormat = 2;
/// B, G, R, A.
pub const BREADER_PIXEL_BGRA8: BreaderPixelFormat = 3;
/// Y plane followed by an interleaved U/V plane at half resolution.
pub const BREADER_PIXEL_NV12: BreaderPixelFormat = 4;
/// Y plane followed by an interleaved V/U plane at half resolution.
pub const BREADER_PIXEL_NV21: BreaderPixelFormat = 5;
/// Y plane followed by the U and V planes at half resolution.
pub const BREADER_PIXEL_I420: BreaderPixelFormat = 6;

/// Symbology of a result: `BREADER_SYMBOLOGY_*`.
pub type BreaderSymbology = u32;
/// QR Code.
pub const BREADER_SYMBOLOGY_QR: BreaderSymbology = 0;
/// EAN-13.
pub const BREADER_SYMBOLOGY_EAN13: BreaderSymbology = 1;
/// EAN-8.
pub const BREADER_SYMBOLOGY_EAN8: BreaderSymbology = 2;
/// UPC-A.
pub const BREADER_SYMBOLOGY_UPCA: BreaderSymbology = 3;
/// UPC-E.
pub const BREADER_SYMBOLOGY_UPCE: BreaderSymbology = 4;
/// Code 128.
pub const BREADER_SYMBOLOGY_CODE128: BreaderSymbology = 5;
/// Code 39.
pub const BREADER_SYMBOLOGY_CODE39: BreaderSymbology = 6;
/// Interleaved 2 of 5.
pub const BREADER_SYMBOLOGY_ITF: BreaderSymbology = 7;
/// Data Matrix.
pub const BREADER_SYMBOLOGY_DATAMATRIX: BreaderSymbology = 8;
/// PDF417.
pub const BREADER_SYMBOLOGY_PDF417: BreaderSymbology = 9;
/// Aztec Code.
pub const BREADER_SYMBOLOGY_AZTEC: BreaderSymbology = 10;
/// Codabar.
pub const BREADER_SYMBOLOGY_CODABAR: BreaderSymbology = 11;
/// Micro QR Code.
pub const BREADER_SYMBOLOGY_MICROQR: BreaderSymbology = 12;
/// rMQR Code.
pub const BREADER_SYMBOLOGY_RMQR: BreaderSymbology = 13;
/// GS1 DataBar Omnidirectional / Truncated.
pub const BREADER_SYMBOLOGY_DATABAR: BreaderSymbology = 14;
/// GS1 DataBar Limited.
pub const BREADER_SYMBOLOGY_DATABAR_LIMITED: BreaderSymbology = 15;
/// GS1 DataBar Expanded.
pub const BREADER_SYMBOLOGY_DATABAR_EXPANDED: BreaderSymbology = 16;
/// Code 93.
pub const BREADER_SYMBOLOGY_CODE93: BreaderSymbology = 17;
/// Returned for an invalid result or a symbology newer than this header.
pub const BREADER_SYMBOLOGY_UNKNOWN: BreaderSymbology = 0xFFFF_FFFF;

/// Bit mask of symbologies for `breader_options_set_symbologies`: `1 << BREADER_SYMBOLOGY_x`.
pub type BreaderSymbologyMask = u32;
/// QR Code.
pub const BREADER_MASK_QR: BreaderSymbologyMask = 1 << 0;
/// EAN-13.
pub const BREADER_MASK_EAN13: BreaderSymbologyMask = 1 << 1;
/// EAN-8.
pub const BREADER_MASK_EAN8: BreaderSymbologyMask = 1 << 2;
/// UPC-A.
pub const BREADER_MASK_UPCA: BreaderSymbologyMask = 1 << 3;
/// UPC-E.
pub const BREADER_MASK_UPCE: BreaderSymbologyMask = 1 << 4;
/// Code 128.
pub const BREADER_MASK_CODE128: BreaderSymbologyMask = 1 << 5;
/// Code 39.
pub const BREADER_MASK_CODE39: BreaderSymbologyMask = 1 << 6;
/// Interleaved 2 of 5.
pub const BREADER_MASK_ITF: BreaderSymbologyMask = 1 << 7;
/// Data Matrix.
pub const BREADER_MASK_DATAMATRIX: BreaderSymbologyMask = 1 << 8;
/// PDF417.
pub const BREADER_MASK_PDF417: BreaderSymbologyMask = 1 << 9;
/// Aztec Code.
pub const BREADER_MASK_AZTEC: BreaderSymbologyMask = 1 << 10;
/// Codabar.
pub const BREADER_MASK_CODABAR: BreaderSymbologyMask = 1 << 11;
/// Micro QR Code.
pub const BREADER_MASK_MICROQR: BreaderSymbologyMask = 1 << 12;
/// rMQR Code.
pub const BREADER_MASK_RMQR: BreaderSymbologyMask = 1 << 13;
/// GS1 DataBar Omnidirectional / Truncated.
pub const BREADER_MASK_DATABAR: BreaderSymbologyMask = 1 << 14;
/// GS1 DataBar Limited.
pub const BREADER_MASK_DATABAR_LIMITED: BreaderSymbologyMask = 1 << 15;
/// GS1 DataBar Expanded.
pub const BREADER_MASK_DATABAR_EXPANDED: BreaderSymbologyMask = 1 << 16;
/// Code 93.
pub const BREADER_MASK_CODE93: BreaderSymbologyMask = 1 << 17;
/// Every symbology (the default).
pub const BREADER_MASK_ALL: BreaderSymbologyMask = (1 << 18) - 1;

/// Kind of payload: `BREADER_CONTENT_*`.
pub type BreaderContentKind = u32;
/// Not classified.
pub const BREADER_CONTENT_UNPARSED: BreaderContentKind = 0;
/// Human readable text.
pub const BREADER_CONTENT_TEXT: BreaderContentKind = 1;
/// Non-textual bytes.
pub const BREADER_CONTENT_BINARY: BreaderContentKind = 2;
/// A URL.
pub const BREADER_CONTENT_URL: BreaderContentKind = 3;
/// GS1 element strings; `breader_result_gs1_hri` returns the readable form.
pub const BREADER_CONTENT_GS1: BreaderContentKind = 4;

/// A point in image coordinates (pixels).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BreaderPoint {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

/// Scan options (opaque). Create with `breader_options_new`, change with the setters, free
/// with `breader_options_free`. A reader copies the options when it is created.
#[derive(Debug)]
pub struct BreaderOptions(ScanOptions);

/// A reader (opaque). Owns scratch buffers that are reused between scans.
///
/// Thread safety: a reader must be used by one thread at a time (different readers may be
/// used by different threads; the library has no global state).
#[derive(Debug)]
pub struct BreaderReader(Reader);

#[derive(Debug)]
struct Item {
    symbology: BreaderSymbology,
    name: CString,
    data: Vec<u8>,
    text: Option<CString>,
    text_len: usize,
    content: BreaderContentKind,
    gs1_hri: Option<CString>,
    corners: [BreaderPoint; 4],
    orientation: f32,
    aim_id: CString,
    quality: f32,
}

/// The symbols found by one scan (opaque). Every pointer returned by the `breader_result_*`
/// accessors stays valid until `breader_results_free`.
#[derive(Debug)]
pub struct BreaderResults(Vec<Item>);

/// Runs `f`, turning a panic into `on_panic`.
fn guard<T>(on_panic: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(on_panic)
}

fn status_guard(f: impl FnOnce() -> BreaderStatus) -> BreaderStatus {
    guard(BREADER_PANIC, f)
}

fn symbology_id(s: Symbology) -> BreaderSymbology {
    match s {
        Symbology::Qr => BREADER_SYMBOLOGY_QR,
        Symbology::Ean13 => BREADER_SYMBOLOGY_EAN13,
        Symbology::Ean8 => BREADER_SYMBOLOGY_EAN8,
        Symbology::UpcA => BREADER_SYMBOLOGY_UPCA,
        Symbology::UpcE => BREADER_SYMBOLOGY_UPCE,
        Symbology::Code128 => BREADER_SYMBOLOGY_CODE128,
        Symbology::Code39 => BREADER_SYMBOLOGY_CODE39,
        Symbology::Itf => BREADER_SYMBOLOGY_ITF,
        Symbology::DataMatrix => BREADER_SYMBOLOGY_DATAMATRIX,
        Symbology::Pdf417 => BREADER_SYMBOLOGY_PDF417,
        Symbology::Aztec => BREADER_SYMBOLOGY_AZTEC,
        Symbology::Codabar => BREADER_SYMBOLOGY_CODABAR,
        Symbology::MicroQr => BREADER_SYMBOLOGY_MICROQR,
        Symbology::Rmqr => BREADER_SYMBOLOGY_RMQR,
        Symbology::DataBar => BREADER_SYMBOLOGY_DATABAR,
        Symbology::DataBarLimited => BREADER_SYMBOLOGY_DATABAR_LIMITED,
        Symbology::DataBarExpanded => BREADER_SYMBOLOGY_DATABAR_EXPANDED,
        Symbology::Code93 => BREADER_SYMBOLOGY_CODE93,
        _ => BREADER_SYMBOLOGY_UNKNOWN,
    }
}

fn mask_to_set(mask: BreaderSymbologyMask) -> SymbologySet {
    Symbology::ALL
        .iter()
        .copied()
        .filter(|s| {
            let id = symbology_id(*s);
            id < 32 && mask & (1 << id) != 0
        })
        .collect()
}

fn cstring(bytes: impl Into<Vec<u8>>) -> CString {
    // Interior NULs are cut: the C string ends at the first one.
    let mut v: Vec<u8> = bytes.into();
    if let Some(pos) = v.iter().position(|b| *b == 0) {
        v.truncate(pos);
    }
    CString::new(v).unwrap_or_default()
}

fn item(s: Symbol) -> Item {
    let (content, hri) = match &s.content {
        Content::Text => (BREADER_CONTENT_TEXT, None),
        Content::Binary => (BREADER_CONTENT_BINARY, None),
        Content::Url => (BREADER_CONTENT_URL, None),
        Content::Gs1(elements) => (BREADER_CONTENT_GS1, Some(cstring(gs1::hri(elements)))),
        _ => (BREADER_CONTENT_UNPARSED, None),
    };
    let aim: Vec<u8> = s.aim_id.iter().copied().filter(|b| *b != 0).collect();
    let mut corners = [BreaderPoint::default(); 4];
    for (dst, p) in corners.iter_mut().zip(s.quad.corners.iter()) {
        *dst = BreaderPoint { x: p.x, y: p.y };
    }
    // Text with an interior NUL is a valid Rust string but not a C string: it is returned
    // as NULL and the caller uses the data bytes.
    let (text, text_len) = match s.text {
        Some(t) if !t.as_bytes().contains(&0) => {
            let len = t.len();
            (CString::new(t).ok(), len)
        }
        _ => (None, 0),
    };
    Item {
        symbology: symbology_id(s.symbology),
        name: cstring(s.symbology.name()),
        data: s.data,
        text,
        text_len,
        content,
        gs1_hri: hri,
        corners,
        orientation: s.orientation_deg,
        aim_id: cstring(aim),
        quality: s.quality,
    }
}

fn map_error(e: &Error) -> BreaderStatus {
    match e {
        Error::InvalidDimensions { .. }
        | Error::InvalidStride { .. }
        | Error::BufferTooSmall { .. } => BREADER_INVALID_ARGUMENT,
        _ => BREADER_UNSUPPORTED,
    }
}

fn results_into(out: *mut *mut BreaderResults, symbols: Vec<Symbol>) {
    let results = Box::new(BreaderResults(symbols.into_iter().map(item).collect()));
    // SAFETY: the caller checked `out` is non-null; the contract says it is writable.
    unsafe { *out = Box::into_raw(results) };
}

/// Shared reference behind a handle pointer, or `None` for null.
///
/// SAFETY: the pointer is null or was returned by this library and not yet freed.
unsafe fn handle<'a, T>(p: *const T) -> Option<&'a T> {
    unsafe { p.as_ref() }
}

fn result_item<'a>(r: *const BreaderResults, index: usize) -> Option<&'a Item> {
    // SAFETY: `r` is null or a live BreaderResults (API contract).
    unsafe { handle(r) }?.0.get(index)
}

// ---------------------------------------------------------------------------------------
// Version and status
// ---------------------------------------------------------------------------------------

/// The library version as a static, NUL-terminated string such as "0.1.0".
#[unsafe(no_mangle)]
pub extern "C" fn breader_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0")
        .as_ptr()
        .cast::<c_char>()
}

/// A short static description of a status code.
#[unsafe(no_mangle)]
pub extern "C" fn breader_status_string(status: BreaderStatus) -> *const c_char {
    let s: &'static str = match status {
        BREADER_OK => "ok\0",
        BREADER_INVALID_ARGUMENT => "invalid argument\0",
        BREADER_UNSUPPORTED => "unsupported\0",
        BREADER_PANIC => "internal error (panic caught)\0",
        BREADER_IO_ERROR => "i/o error\0",
        _ => "unknown status\0",
    };
    s.as_ptr().cast::<c_char>()
}

// ---------------------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------------------

/// Creates options with the defaults: Balanced effort, every symbology, automatic
/// inversion, no symbol limit, no time budget, add-ons ignored, Code 39 without Full ASCII
/// or check character. Free with `breader_options_free`. Returns NULL only after a panic.
#[unsafe(no_mangle)]
pub extern "C" fn breader_options_new() -> *mut BreaderOptions {
    guard(std::ptr::null_mut(), || {
        Box::into_raw(Box::new(BreaderOptions(ScanOptions::default())))
    })
}

/// Frees options. NULL is accepted and does nothing. The options of a reader already
/// created from them are not affected.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_free(options: *mut BreaderOptions) {
    guard((), || {
        if !options.is_null() {
            // SAFETY: non-null and returned by `breader_options_new`, freed once (contract).
            drop(unsafe { Box::from_raw(options) });
        }
    });
}

fn with_options(
    options: *mut BreaderOptions,
    f: impl FnOnce(&mut ScanOptions) -> BreaderStatus,
) -> BreaderStatus {
    status_guard(|| {
        // SAFETY: `options` is null or a live BreaderOptions, used by one thread (contract).
        match unsafe { options.as_mut() } {
            Some(o) => f(&mut o.0),
            None => BREADER_INVALID_ARGUMENT,
        }
    })
}

/// Sets the effort (`BREADER_EFFORT_*`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_set_effort(
    options: *mut BreaderOptions,
    effort: BreaderEffort,
) -> BreaderStatus {
    with_options(options, |o| {
        o.effort = match effort {
            BREADER_EFFORT_FAST => Effort::Fast,
            BREADER_EFFORT_BALANCED => Effort::Balanced,
            BREADER_EFFORT_EXHAUSTIVE => Effort::Exhaustive,
            _ => return BREADER_INVALID_ARGUMENT,
        };
        BREADER_OK
    })
}

/// Sets the symbologies to look for, as an OR of `BREADER_MASK_*` (bits outside the known
/// ones are rejected, an empty mask is rejected).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_set_symbologies(
    options: *mut BreaderOptions,
    mask: BreaderSymbologyMask,
) -> BreaderStatus {
    with_options(options, |o| {
        if mask == 0 || mask & !BREADER_MASK_ALL != 0 {
            return BREADER_INVALID_ARGUMENT;
        }
        o.symbologies = mask_to_set(mask);
        BREADER_OK
    })
}

/// Light-on-dark passes: negative for the effort's default (on unless Fast), 0 off,
/// positive on.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_set_try_invert(
    options: *mut BreaderOptions,
    mode: i32,
) -> BreaderStatus {
    with_options(options, |o| {
        o.try_invert = match mode {
            m if m < 0 => None,
            0 => Some(false),
            _ => Some(true),
        };
        BREADER_OK
    })
}

/// Stops after this many symbols; 0 means unlimited.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_set_max_symbols(
    options: *mut BreaderOptions,
    max_symbols: usize,
) -> BreaderStatus {
    with_options(options, |o| {
        o.max_symbols = max_symbols;
        BREADER_OK
    })
}

/// Time budget of a scan in milliseconds, checked between passes (a pass in progress is
/// not interrupted); 0 means no budget.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_set_time_budget_ms(
    options: *mut BreaderOptions,
    milliseconds: u64,
) -> BreaderStatus {
    with_options(options, |o| {
        o.time_budget = (milliseconds > 0).then(|| Duration::from_millis(milliseconds));
        BREADER_OK
    })
}

/// Sets the EAN/UPC add-on policy (`BREADER_ADDON_*`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_set_ean_addon(
    options: *mut BreaderOptions,
    policy: BreaderAddonPolicy,
) -> BreaderStatus {
    with_options(options, |o| {
        o.ean_addon = match policy {
            BREADER_ADDON_IGNORE => EanAddonPolicy::Ignore,
            BREADER_ADDON_OPTIONAL => EanAddonPolicy::Optional,
            BREADER_ADDON_REQUIRED => EanAddonPolicy::Required,
            _ => return BREADER_INVALID_ARGUMENT,
        };
        BREADER_OK
    })
}

/// Code 39: decode Full ASCII (two-character escape sequences).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_set_code39_full_ascii(
    options: *mut BreaderOptions,
    enabled: bool,
) -> BreaderStatus {
    with_options(options, |o| {
        o.code39.full_ascii = enabled;
        BREADER_OK
    })
}

/// Code 39: require a valid modulo-43 check character (kept in the payload).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_options_set_code39_checksum(
    options: *mut BreaderOptions,
    required: bool,
) -> BreaderStatus {
    with_options(options, |o| {
        o.code39.require_checksum = required;
        BREADER_OK
    })
}

// ---------------------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------------------

/// Creates a reader with a copy of `options`; NULL means the defaults. Free with
/// `breader_reader_free`. Returns NULL only after a panic.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_reader_new(options: *const BreaderOptions) -> *mut BreaderReader {
    guard(std::ptr::null_mut(), || {
        // SAFETY: `options` is null or a live BreaderOptions (contract).
        let o = unsafe { handle(options) }.map_or_else(ScanOptions::default, |o| o.0.clone());
        Box::into_raw(Box::new(BreaderReader(Reader::with_options(o))))
    })
}

/// Frees a reader. NULL is accepted and does nothing. Results of earlier scans stay valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_reader_free(reader: *mut BreaderReader) {
    guard((), || {
        if !reader.is_null() {
            // SAFETY: non-null and returned by `breader_reader_new`, freed once (contract).
            drop(unsafe { Box::from_raw(reader) });
        }
    });
}

fn pixel_format(f: BreaderPixelFormat) -> Option<PixelFormat> {
    Some(match f {
        BREADER_PIXEL_LUMA8 => PixelFormat::Luma8,
        BREADER_PIXEL_RGB8 => PixelFormat::Rgb8,
        BREADER_PIXEL_RGBA8 => PixelFormat::Rgba8,
        BREADER_PIXEL_BGRA8 => PixelFormat::Bgra8,
        BREADER_PIXEL_NV12 => PixelFormat::Nv12,
        BREADER_PIXEL_NV21 => PixelFormat::Nv21,
        BREADER_PIXEL_I420 => PixelFormat::I420,
        _ => return None,
    })
}

/// Scans a pixel buffer of `len` bytes and stores the results in `*out`.
///
/// `stride` is the row pitch in bytes of the first plane (0 means tightly packed). For
/// NV12, NV21 and I420 the chroma planes follow the luma plane. `*out` is set to NULL
/// first and to a new results object on `BREADER_OK` (possibly with zero symbols), which the
/// caller frees with `breader_results_free`. The buffer is only read during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_scan(
    reader: *mut BreaderReader,
    data: *const u8,
    len: usize,
    width: usize,
    height: usize,
    stride: usize,
    format: BreaderPixelFormat,
    out: *mut *mut BreaderResults,
) -> BreaderStatus {
    if out.is_null() {
        return BREADER_INVALID_ARGUMENT;
    }
    // SAFETY: `out` is non-null and writable (contract).
    unsafe { *out = std::ptr::null_mut() };
    status_guard(|| {
        // SAFETY: `reader` is null or a live BreaderReader used by this thread only (contract).
        let Some(reader) = (unsafe { reader.as_mut() }) else {
            return BREADER_INVALID_ARGUMENT;
        };
        let Some(format) = pixel_format(format) else {
            return BREADER_INVALID_ARGUMENT;
        };
        if data.is_null() || len == 0 {
            return BREADER_INVALID_ARGUMENT;
        }
        // SAFETY: the caller guarantees `data` points to `len` readable bytes (contract).
        let pixels = unsafe { std::slice::from_raw_parts(data, len) };
        let stride = if stride == 0 {
            width.saturating_mul(format.bytes_per_pixel())
        } else {
            stride
        };
        match reader.0.scan_pixels(pixels, format, width, height, stride) {
            Ok(symbols) => {
                results_into(out, symbols);
                BREADER_OK
            }
            Err(e) => map_error(&e),
        }
    })
}

/// Decodes the PNG or JPEG file at `path` (NUL-terminated, UTF-8 or the platform's byte
/// path on Unix) and scans it; results as for `breader_scan`. Returns `BREADER_IO_ERROR`
/// when the file cannot be read, `BREADER_UNSUPPORTED` when it cannot be decoded or the
/// library was built without the `image` feature.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_scan_file(
    reader: *mut BreaderReader,
    path: *const c_char,
    out: *mut *mut BreaderResults,
) -> BreaderStatus {
    if out.is_null() {
        return BREADER_INVALID_ARGUMENT;
    }
    // SAFETY: `out` is non-null and writable (contract).
    unsafe { *out = std::ptr::null_mut() };
    status_guard(|| scan_file(reader, path, out))
}

#[cfg(feature = "image")]
fn scan_file(
    reader: *mut BreaderReader,
    path: *const c_char,
    out: *mut *mut BreaderResults,
) -> BreaderStatus {
    // SAFETY: `reader` is null or a live BreaderReader used by this thread only (contract).
    let Some(reader) = (unsafe { reader.as_mut() }) else {
        return BREADER_INVALID_ARGUMENT;
    };
    if path.is_null() {
        return BREADER_INVALID_ARGUMENT;
    }
    // SAFETY: `path` is non-null and NUL-terminated (contract).
    let bytes = unsafe { std::ffi::CStr::from_ptr(path) }.to_bytes();
    #[cfg(unix)]
    let path = {
        use std::os::unix::ffi::OsStrExt;
        std::path::Path::new(std::ffi::OsStr::from_bytes(bytes)).to_path_buf()
    };
    #[cfg(not(unix))]
    let path = match std::str::from_utf8(bytes) {
        Ok(s) => std::path::PathBuf::from(s),
        Err(_) => return BREADER_INVALID_ARGUMENT,
    };
    // A file that cannot be opened is an I/O error; anything the decoder then rejects
    // (format, corruption, zero size) is "unsupported".
    if std::fs::File::open(&path).is_err() {
        return BREADER_IO_ERROR;
    }
    match reader.0.scan_file(&path) {
        Ok(symbols) => {
            results_into(out, symbols);
            BREADER_OK
        }
        Err(_) => BREADER_UNSUPPORTED,
    }
}

#[cfg(not(feature = "image"))]
fn scan_file(
    reader: *mut BreaderReader,
    path: *const c_char,
    _out: *mut *mut BreaderResults,
) -> BreaderStatus {
    if reader.is_null() || path.is_null() {
        BREADER_INVALID_ARGUMENT
    } else {
        BREADER_UNSUPPORTED
    }
}

// ---------------------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------------------

/// Frees a results object and every pointer obtained from it. NULL is accepted.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_results_free(results: *mut BreaderResults) {
    guard((), || {
        if !results.is_null() {
            // SAFETY: non-null and returned by a scan function, freed once (contract).
            drop(unsafe { Box::from_raw(results) });
        }
    });
}

/// Number of symbols; 0 for NULL.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_results_count(results: *const BreaderResults) -> usize {
    guard(0, || {
        // SAFETY: null or a live BreaderResults (contract).
        unsafe { handle(results) }.map_or(0, |r| r.0.len())
    })
}

/// Symbology of result `index` (`BREADER_SYMBOLOGY_*`), or `BREADER_SYMBOLOGY_UNKNOWN`
/// for NULL or an index out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_symbology(
    results: *const BreaderResults,
    index: usize,
) -> BreaderSymbology {
    guard(BREADER_SYMBOLOGY_UNKNOWN, || {
        result_item(results, index).map_or(BREADER_SYMBOLOGY_UNKNOWN, |i| i.symbology)
    })
}

/// Human readable symbology name such as "QR Code" (NUL-terminated), or NULL for NULL or an
/// index out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_symbology_name(
    results: *const BreaderResults,
    index: usize,
) -> *const c_char {
    guard(std::ptr::null(), || {
        result_item(results, index).map_or(std::ptr::null(), |i| i.name.as_ptr())
    })
}

/// Raw payload bytes of result `index`; `*len` (if not NULL) receives their count. The
/// pointer is non-NULL for a valid result even when the payload is empty; NULL for NULL or an
/// index out of range (and `*len` is 0). The bytes are not NUL-terminated.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_data(
    results: *const BreaderResults,
    index: usize,
    len: *mut usize,
) -> *const u8 {
    guard(std::ptr::null(), || {
        let it = result_item(results, index);
        // SAFETY: `len` is null or writable (contract).
        if let Some(len) = unsafe { len.as_mut() } {
            *len = it.map_or(0, |i| i.data.len());
        }
        it.map_or(std::ptr::null(), |i| i.data.as_ptr())
    })
}

/// The payload as UTF-8 text, NUL-terminated; `*len` (if not NULL) receives the length in
/// bytes without the terminator. NULL when the symbol has no unambiguous text (use the data
/// bytes), when the text contains a NUL byte, for NULL or an index out of range (`*len` is
/// then 0).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_text(
    results: *const BreaderResults,
    index: usize,
    len: *mut usize,
) -> *const c_char {
    guard(std::ptr::null(), || {
        let it = result_item(results, index);
        // SAFETY: `len` is null or writable (contract).
        if let Some(len) = unsafe { len.as_mut() } {
            *len = it.map_or(0, |i| i.text_len);
        }
        it.and_then(|i| i.text.as_ref())
            .map_or(std::ptr::null(), |t| t.as_ptr())
    })
}

/// Kind of payload (`BREADER_CONTENT_*`); `BREADER_CONTENT_UNPARSED` for NULL or an index
/// out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_content_kind(
    results: *const BreaderResults,
    index: usize,
) -> BreaderContentKind {
    guard(BREADER_CONTENT_UNPARSED, || {
        result_item(results, index).map_or(BREADER_CONTENT_UNPARSED, |i| i.content)
    })
}

/// Human readable GS1 form such as "(01)09506000134352(17)261231", or NULL when the content
/// is not GS1.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_gs1_hri(
    results: *const BreaderResults,
    index: usize,
) -> *const c_char {
    guard(std::ptr::null(), || {
        result_item(results, index)
            .and_then(|i| i.gs1_hri.as_ref())
            .map_or(std::ptr::null(), |t| t.as_ptr())
    })
}

/// Writes the four corners of the symbol in image coordinates to `out[0..4]`. Returns
/// `BREADER_INVALID_ARGUMENT` (and writes nothing) for NULL pointers or a bad index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_corners(
    results: *const BreaderResults,
    index: usize,
    out: *mut BreaderPoint,
) -> BreaderStatus {
    status_guard(|| match (result_item(results, index), out.is_null()) {
        (Some(i), false) => {
            // SAFETY: `out` is non-null and points to room for 4 points (contract).
            unsafe { std::ptr::copy_nonoverlapping(i.corners.as_ptr(), out, 4) };
            BREADER_OK
        }
        _ => BREADER_INVALID_ARGUMENT,
    })
}

/// Rotation of the symbol in degrees; 0 for NULL or an index out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_orientation(
    results: *const BreaderResults,
    index: usize,
) -> f32 {
    guard(0.0, || {
        result_item(results, index).map_or(0.0, |i| i.orientation)
    })
}

/// ISO/IEC 15424 symbology identifier such as "]Q1" (NUL-terminated), or NULL for NULL or an
/// index out of range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_aim_id(
    results: *const BreaderResults,
    index: usize,
) -> *const c_char {
    guard(std::ptr::null(), || {
        result_item(results, index).map_or(std::ptr::null(), |i| i.aim_id.as_ptr())
    })
}

/// Decoder quality estimate in [0, 1]; informational only. 0 for NULL or a bad index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn breader_result_quality(
    results: *const BreaderResults,
    index: usize,
) -> f32 {
    guard(0.0, || {
        result_item(results, index).map_or(0.0, |i| i.quality)
    })
}

#[cfg(test)]
mod tests;
