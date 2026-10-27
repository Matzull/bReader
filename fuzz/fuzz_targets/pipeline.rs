// Input: width-1, height-1, flags (effort, inversion), symbology mask, mode,
// 4 stride bytes, then pixels (packed luma or raw buffer for scan_pixels).

#![no_main]

use breader::{Effort, PixelFormat, Reader, ScanOptions, Symbol, Symbology, SymbologySet};
use libfuzzer_sys::fuzz_target;

const FORMATS: [PixelFormat; 7] = [
    PixelFormat::Luma8,
    PixelFormat::Rgb8,
    PixelFormat::Rgba8,
    PixelFormat::Bgra8,
    PixelFormat::Nv12,
    PixelFormat::Nv21,
    PixelFormat::I420,
];

fn check(symbols: &[Symbol]) {
    for s in symbols {
        assert!(!s.data.is_empty(), "empty symbol data: {s:?}");
        assert!(
            s.quad
                .corners
                .iter()
                .all(|p| p.x.is_finite() && p.y.is_finite()),
            "non-finite quad: {s:?}"
        );
        assert!(s.orientation_deg.is_finite());
        assert!(s.quality.is_finite());
    }
}

fuzz_target!(|data: &[u8]| {
    let [w, h, flags, subset, mode, s0, s1, s2, s3, rest @ ..] = data else {
        return;
    };
    let (w, h) = (usize::from(*w) % 256 + 1, usize::from(*h) % 256 + 1);
    let effort = match flags % 3 {
        0 => Effort::Fast,
        1 => Effort::Balanced,
        _ => Effort::Exhaustive,
    };
    let mut set = SymbologySet::EMPTY;
    for (i, s) in Symbology::ALL.iter().enumerate() {
        if subset >> (i % 8) & 1 == 1 {
            set = set.with(*s);
        }
    }
    if set.is_empty() {
        set = SymbologySet::all();
    }
    let options = ScanOptions {
        effort,
        symbologies: set,
        try_invert: match flags >> 2 & 3 {
            0 => None,
            1 => Some(false),
            _ => Some(true),
        },
        ..ScanOptions::default()
    };
    let mut reader = Reader::with_options(options);

    if mode & 1 == 0 {
        // Packed luma image; short pixel data is zero-padded.
        let mut pixels = vec![0u8; w * h];
        let n = rest.len().min(pixels.len());
        pixels[..n].copy_from_slice(&rest[..n]);
        if let Ok(view) = breader::ImageView::from_packed(&pixels, w, h) {
            check(&reader.scan(view));
        }
    } else {
        let format = FORMATS[usize::from(mode >> 1) % FORMATS.len()];
        // Mostly small paddings; sometimes absurd strides, which must be
        // rejected without overflow. Too-small buffers are on purpose too.
        let extra = u32::from_le_bytes([*s0, *s1, *s2, *s3]);
        let extra = if mode & 0x80 != 0 {
            extra as usize * 65_537
        } else {
            usize::from(extra as u8)
        };
        let stride = (w * format.bytes_per_pixel()).wrapping_add(extra);
        if let Ok(symbols) = reader.scan_pixels(rest, format, w, h, stride) {
            check(&symbols);
        }
    }
});
