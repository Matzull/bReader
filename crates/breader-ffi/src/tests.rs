use super::*;
use qrcode::{Color, EcLevel, QrCode};
use std::ffi::CStr;
use std::ptr::{null, null_mut};

const TEXT: &str = "HELLO FFI 123";

/// A QR code as a grey image: (pixels, side).
fn qr(text: &str) -> (Vec<u8>, usize) {
    // Miri runs the decoder some thousand times slower: a smaller picture.
    const PX: usize = if cfg!(miri) { 3 } else { 6 };
    const QUIET: usize = 4;
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).unwrap();
    let dim = code.width();
    let side = (dim + 2 * QUIET) * PX;
    let colors = code.to_colors();
    let mut img = vec![240u8; side * side];
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / PX, y / PX);
            let inside = mx >= QUIET && my >= QUIET && mx < QUIET + dim && my < QUIET + dim;
            if inside && colors[(my - QUIET) * dim + (mx - QUIET)] == Color::Dark {
                img[y * side + x] = 20;
            }
        }
    }
    (img, side)
}

/// Miri interprets the decoder some thousand times slower than it runs: the tests that scan use
/// the Fast effort under it (the QR code is read at Fast as well).
fn test_options() -> ScanOptions {
    if cfg!(miri) {
        ScanOptions::with_effort(Effort::Fast)
    } else {
        ScanOptions::default()
    }
}

/// A reader with the options above, through the C interface.
///
/// # Safety
/// The handle is freed by the caller, once.
unsafe fn new_reader() -> *mut BreaderReader {
    // SAFETY: the options handle is live for the call and freed once.
    unsafe {
        let o = breader_options_new();
        (*o).0 = test_options();
        let reader = breader_reader_new(o);
        breader_options_free(o);
        reader
    }
}

fn scan_luma(
    reader: *mut BreaderReader,
    img: &[u8],
    side: usize,
) -> (BreaderStatus, *mut BreaderResults) {
    let mut out = null_mut();
    // SAFETY: the buffer is valid for its length and `out` is writable.
    let st = unsafe {
        breader_scan(
            reader,
            img.as_ptr(),
            img.len(),
            side,
            side,
            side,
            BREADER_PIXEL_LUMA8,
            &mut out,
        )
    };
    (st, out)
}

#[test]
fn version_and_status_strings() {
    // SAFETY: both functions return static NUL-terminated strings.
    unsafe {
        let v = CStr::from_ptr(breader_version()).to_str().unwrap();
        assert_eq!(v, env!("CARGO_PKG_VERSION"));
        for s in [
            BREADER_OK,
            BREADER_INVALID_ARGUMENT,
            BREADER_UNSUPPORTED,
            BREADER_PANIC,
            BREADER_IO_ERROR,
            99,
        ] {
            assert!(
                !CStr::from_ptr(breader_status_string(s))
                    .to_bytes()
                    .is_empty()
            );
        }
    }
}

#[test]
fn scans_a_qr_and_exposes_every_field() {
    let (img, side) = qr(TEXT);
    // SAFETY: handles come from this library and are freed once; pointers are used before
    // the results are freed.
    unsafe {
        let reader = new_reader();
        assert!(!reader.is_null());
        let (st, res) = scan_luma(reader, &img, side);
        assert_eq!(st, BREADER_OK);
        assert_eq!(breader_results_count(res), 1);
        assert_eq!(breader_result_symbology(res, 0), BREADER_SYMBOLOGY_QR);
        let name = CStr::from_ptr(breader_result_symbology_name(res, 0));
        assert_eq!(name.to_str().unwrap(), Symbology::Qr.name());
        let mut len = 0;
        let data = breader_result_data(res, 0, &mut len);
        assert_eq!(std::slice::from_raw_parts(data, len), TEXT.as_bytes());
        let mut tlen = 0;
        let text = breader_result_text(res, 0, &mut tlen);
        assert_eq!(CStr::from_ptr(text).to_str().unwrap(), TEXT);
        assert_eq!(tlen, TEXT.len());
        assert_eq!(breader_result_content_kind(res, 0), BREADER_CONTENT_TEXT);
        assert!(breader_result_gs1_hri(res, 0).is_null());
        let mut corners = [BreaderPoint::default(); 4];
        assert_eq!(
            breader_result_corners(res, 0, corners.as_mut_ptr()),
            BREADER_OK
        );
        for c in corners {
            assert!(c.x > 0.0 && c.x < side as f32 && c.y > 0.0 && c.y < side as f32);
        }
        assert!(breader_result_orientation(res, 0).is_finite());
        let aim = CStr::from_ptr(breader_result_aim_id(res, 0));
        assert_eq!(aim.to_str().unwrap().len(), 3);
        assert!((0.0..=1.0).contains(&breader_result_quality(res, 0)));
        // Index out of range.
        assert_eq!(breader_result_symbology(res, 1), BREADER_SYMBOLOGY_UNKNOWN);
        assert!(breader_result_symbology_name(res, 1).is_null());
        let mut l = 7;
        assert!(breader_result_data(res, 1, &mut l).is_null());
        assert_eq!(l, 0);
        assert!(breader_result_text(res, 1, null_mut()).is_null());
        assert_eq!(
            breader_result_corners(res, 1, corners.as_mut_ptr()),
            BREADER_INVALID_ARGUMENT
        );
        // Results outlive the reader.
        breader_reader_free(reader);
        assert_eq!(breader_results_count(res), 1);
        breader_results_free(res);
    }
}

#[test]
fn options_apply() {
    let (img, side) = qr(TEXT);
    // SAFETY: handles come from this library and are freed once.
    unsafe {
        let o = breader_options_new();
        assert_eq!(
            breader_options_set_effort(o, BREADER_EFFORT_FAST),
            BREADER_OK
        );
        assert_eq!(breader_options_set_try_invert(o, 0), BREADER_OK);
        assert_eq!(breader_options_set_max_symbols(o, 1), BREADER_OK);
        assert_eq!(breader_options_set_time_budget_ms(o, 5000), BREADER_OK);
        assert_eq!(
            breader_options_set_ean_addon(o, BREADER_ADDON_OPTIONAL),
            BREADER_OK
        );
        assert_eq!(breader_options_set_code39_full_ascii(o, true), BREADER_OK);
        assert_eq!(breader_options_set_code39_checksum(o, true), BREADER_OK);
        assert_eq!(
            (*o).0.ean_addon,
            EanAddonPolicy::Optional,
            "setters write through"
        );
        // Only EAN-13 enabled: the QR is not reported.
        assert_eq!(
            breader_options_set_symbologies(o, BREADER_MASK_EAN13),
            BREADER_OK
        );
        let reader = breader_reader_new(o);
        let (st, res) = scan_luma(reader, &img, side);
        assert_eq!((st, breader_results_count(res)), (BREADER_OK, 0));
        breader_results_free(res);
        breader_reader_free(reader);
        // The options are copied by the reader: change them and create another.
        assert_eq!(
            breader_options_set_symbologies(o, BREADER_MASK_QR | BREADER_MASK_CODE93),
            BREADER_OK
        );
        let reader = breader_reader_new(o);
        let (st, res) = scan_luma(reader, &img, side);
        assert_eq!((st, breader_results_count(res)), (BREADER_OK, 1));
        breader_results_free(res);
        breader_reader_free(reader);
        // Invalid values are rejected.
        assert_eq!(breader_options_set_effort(o, 9), BREADER_INVALID_ARGUMENT);
        assert_eq!(
            breader_options_set_ean_addon(o, 9),
            BREADER_INVALID_ARGUMENT
        );
        assert_eq!(
            breader_options_set_symbologies(o, 0),
            BREADER_INVALID_ARGUMENT
        );
        assert_eq!(
            breader_options_set_symbologies(o, 1 << 30),
            BREADER_INVALID_ARGUMENT
        );
        breader_options_free(o);
    }
}

#[test]
fn symbology_masks_match_ids() {
    for s in Symbology::ALL {
        let id = symbology_id(*s);
        assert!(id < 18, "{s:?}");
        assert_eq!(mask_to_set(1 << id).iter().collect::<Vec<_>>(), vec![*s]);
    }
    assert_eq!(
        mask_to_set(BREADER_MASK_ALL).iter().count(),
        Symbology::ALL.len()
    );
    assert_eq!(BREADER_MASK_ALL.count_ones() as usize, Symbology::ALL.len());
}

#[test]
fn every_pixel_format_scans_a_blank_frame() {
    // SAFETY: the buffer outlives each call; the handles are freed once.
    unsafe {
        let reader = new_reader();
        for f in [
            BREADER_PIXEL_LUMA8,
            BREADER_PIXEL_RGB8,
            BREADER_PIXEL_RGBA8,
            BREADER_PIXEL_BGRA8,
            BREADER_PIXEL_NV12,
            BREADER_PIXEL_NV21,
            BREADER_PIXEL_I420,
        ] {
            let buf = vec![255u8; 32 * 48 * 4];
            let mut out = null_mut();
            let st = breader_scan(reader, buf.as_ptr(), buf.len(), 32, 32, 0, f, &mut out);
            assert_eq!(st, BREADER_OK, "format {f}");
            assert_eq!(breader_results_count(out), 0);
            breader_results_free(out);
        }
        breader_reader_free(reader);
    }
}

#[test]
fn invalid_arguments_and_nulls() {
    let buf = vec![0u8; 64 * 64];
    // SAFETY: the pointers are valid or deliberately null, as the calls document.
    unsafe {
        let reader = breader_reader_new(null());
        let mut out: *mut BreaderResults = null_mut();
        let p = buf.as_ptr();
        let n = buf.len();
        let scan = |r, d, len, w, h, st, f, o| breader_scan(r, d, len, w, h, st, f, o);
        let bad = BREADER_INVALID_ARGUMENT;
        assert_eq!(scan(reader, p, n, 64, 64, 64, 0, null_mut()), bad);
        assert_eq!(scan(null_mut(), p, n, 64, 64, 64, 0, &mut out), bad);
        assert_eq!(scan(reader, null(), n, 64, 64, 64, 0, &mut out), bad);
        assert_eq!(scan(reader, p, 0, 64, 64, 64, 0, &mut out), bad);
        assert_eq!(scan(reader, p, n, 64, 64, 64, 77, &mut out), bad);
        assert_eq!(scan(reader, p, n, 0, 64, 64, 0, &mut out), bad);
        assert_eq!(scan(reader, p, n, 64, 64, 32, 0, &mut out), bad);
        assert_eq!(scan(reader, p, n - 1, 64, 64, 64, 0, &mut out), bad);
        assert_eq!(scan(reader, p, n, usize::MAX, 64, 0, 0, &mut out), bad);
        assert_eq!(scan(reader, p, n, usize::MAX, 64, 5, 1, &mut out), bad);
        assert!(out.is_null(), "out stays NULL on failure");
        // Frees and accessors accept NULL.
        breader_reader_free(null_mut());
        breader_options_free(null_mut());
        breader_results_free(null_mut());
        assert_eq!(breader_results_count(null()), 0);
        assert_eq!(
            breader_result_symbology(null(), 0),
            BREADER_SYMBOLOGY_UNKNOWN
        );
        assert!(breader_result_text(null(), 0, null_mut()).is_null());
        assert!(breader_result_data(null(), 0, null_mut()).is_null());
        assert!(breader_result_aim_id(null(), 0).is_null());
        assert_eq!(breader_result_quality(null(), 0), 0.0);
        assert_eq!(breader_result_corners(null(), 0, null_mut()), bad);
        assert_eq!(breader_options_set_effort(null_mut(), 0), bad);
        assert_eq!(breader_scan_file(reader, null(), &mut out), bad);
        assert_eq!(
            breader_scan_file(null_mut(), c"x.png".as_ptr(), &mut out),
            bad
        );
        breader_reader_free(reader);
    }
}

#[test]
fn a_panic_becomes_a_status() {
    assert_eq!(status_guard(|| panic!("boom")), BREADER_PANIC);
    assert_eq!(guard(5, || -> i32 { panic!("boom") }), 5);
    assert_eq!(status_guard(|| BREADER_OK), BREADER_OK);
}

#[test]
fn text_with_a_nul_is_returned_as_null_and_data_keeps_it() {
    let (img, side) = qr(TEXT);
    let mut reader = Reader::with_options(test_options());
    let mut symbols = reader
        .scan_pixels(&img, PixelFormat::Luma8, side, side, side)
        .unwrap();
    let mut symbol = symbols.remove(0);
    symbol.data = b"a\0b".to_vec();
    symbol.text = Some("a\0b".to_string());
    let it = item(symbol);
    assert!(it.text.is_none());
    assert_eq!(it.data, b"a\0b");
}

#[cfg(feature = "image")]
#[test]
fn scan_file_reports_io_errors_and_undecodable_files() {
    // SAFETY: handles come from this library and are freed once.
    unsafe {
        let reader = breader_reader_new(null());
        let mut out = null_mut();
        let missing = c"/definitely/not/here.png";
        assert_eq!(
            breader_scan_file(reader, missing.as_ptr(), &mut out),
            BREADER_IO_ERROR
        );
        assert!(out.is_null());
        let dir = std::env::temp_dir().join(format!("breader-ffi-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let junk = dir.join("junk.png");
        std::fs::write(&junk, b"not an image").unwrap();
        let c = CString::new(junk.to_str().unwrap()).unwrap();
        assert_eq!(
            breader_scan_file(reader, c.as_ptr(), &mut out),
            BREADER_UNSUPPORTED
        );
        std::fs::remove_dir_all(&dir).unwrap();
        breader_reader_free(reader);
    }
}
