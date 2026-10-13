//! NFR-4: steady-state scanning does not allocate beyond the returned results.
//!
//! A counting global allocator measures one scan after warm-up scans of the
//! same frame, per reader and effort. Frames without symbols must not
//! allocate; the frame with one QR reports what the result costs. Run
//! `cargo test --release -p breader --test allocation -- --nocapture` for the
//! table (allocations / KiB).
//!
//! The counting allocator needs `unsafe`, which the workspace denies; the
//! exception is confined to this test binary and forwards to `System`.
//!
//! Release only: allocation counts differ between debug and release builds and
//! the documented NFR-4 numbers are release ones, so the test is ignored in
//! builds with debug assertions. Run it with
//! `cargo test --release -p breader --test allocation` (tiers a and b of
//! docs/testing.md, and one CI step). To run it in a debug build anyway, add
//! `-- --ignored` (slow).
#![allow(unsafe_code)]

use breader::modules::*;
use breader::{Effort, ImageView, Pipeline, Reader, ScanOptions};
use qrcode::{Color, EcLevel, QrCode, Version};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

struct Counting;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

fn note(size: usize) {
    if COUNTING.load(Ordering::Relaxed) {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(size, Ordering::Relaxed);
    }
}

// SAFETY: every call is forwarded unchanged to the system allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size());
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

const WARMUP: usize = 3;
const EFFORTS: [Effort; 3] = [Effort::Fast, Effort::Balanced, Effort::Exhaustive];

struct Frame {
    name: &'static str,
    px: Vec<u8>,
    w: usize,
    h: usize,
    /// Symbols a QR-capable pipeline must find (0 for the frames without any).
    symbols: usize,
}

fn blank(w: usize, h: usize) -> Frame {
    Frame {
        name: "blank",
        px: vec![200; w * h],
        w,
        h,
        symbols: 0,
    }
}

/// Deterministic texture without symbols: random gray, blocks and stripes,
/// enough to produce edges, runs and candidates in every reader.
fn texture(w: usize, h: usize) -> Frame {
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        s
    };
    let mut px = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let r = next();
            let block = ((x / 7 * 31 + y / 5 * 17) % 5) as u8 * 40;
            px[y * w + x] = match (y / 64) % 3 {
                0 => (r >> 56) as u8,
                1 => block.saturating_add((r >> 60) as u8 * 3),
                _ if (x / 3 + y / 11) % 2 == 0 => 30 + (r >> 61) as u8,
                _ => 220 - (r >> 61) as u8,
            };
        }
    }
    Frame {
        name: "texture",
        px,
        w,
        h,
        symbols: 0,
    }
}

fn qr_frame(w: usize, h: usize) -> Frame {
    let code =
        QrCode::with_version(b"HTTPS://EXAMPLE.ORG/1", Version::Normal(3), EcLevel::M).unwrap();
    let dim = code.width();
    let scale = 5;
    let mut px = vec![255u8; w * h];
    let (x0, y0) = (w / 3, h / 4);
    for (i, c) in code.to_colors().into_iter().enumerate() {
        if c == Color::Dark {
            let (mx, my) = (i % dim, i / dim);
            for dy in 0..scale {
                for dx in 0..scale {
                    px[(y0 + my * scale + dy) * w + x0 + mx * scale + dx] = 0;
                }
            }
        }
    }
    Frame {
        name: "one QR",
        px,
        w,
        h,
        symbols: 1,
    }
}

/// The default pipeline, or the default modules with only one reader.
fn pipeline(only: Option<&str>) -> Pipeline {
    let on = |name: &str| only.is_none_or(|o| o == name);
    let mut b = Pipeline::builder()
        .binarizer(LocalBinarizer::default())
        .binarizer(OtsuBinarizer)
        .preprocessor(Downscale::new(2))
        .upscale(Upscale);
    if on("qr") {
        b = b.reader(QrReader);
    }
    if on("datamatrix") {
        b = b.reader(DataMatrixReader);
    }
    if on("linear") || on("pdf417") {
        b = b.reader(LinearReader);
    }
    if on("pdf417") {
        b = b.reader(Pdf417Reader);
    }
    if on("aztec") {
        b = b.reader(AztecReader);
    }
    b.post_processor(Dedup)
        .post_processor(TextDecoder)
        .post_processor(ContentClassifier)
        .build()
        .unwrap()
}

/// Allocations and bytes of one scan after the warm-up scans.
fn measure(only: Option<&str>, effort: Effort, frame: &Frame) -> (usize, usize) {
    let mut reader = Reader::from_pipeline(pipeline(only), ScanOptions::with_effort(effort));
    let view = ImageView::from_packed(&frame.px, frame.w, frame.h).unwrap();
    for _ in 0..WARMUP {
        let _ = reader.scan(view);
    }
    ALLOCS.store(0, Ordering::SeqCst);
    BYTES.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    let found = reader.scan(view);
    COUNTING.store(false, Ordering::SeqCst);
    let counts = (ALLOCS.load(Ordering::SeqCst), BYTES.load(Ordering::SeqCst));
    if only.is_none() || only == Some("qr") {
        assert_eq!(found.len(), frame.symbols, "{} {effort:?}", frame.name);
    } else if frame.symbols == 0 {
        assert!(found.is_empty(), "{} {effort:?}", frame.name);
    }
    drop(found);
    counts
}

/// Allocations tolerated per scan of a frame without symbols. Blank frames
/// never allocate. On texture, the linear and Data Matrix readers do not
/// either; the QR, PDF417 and Aztec readers still allocate inside their
/// candidate verification (geometry fits, boundary growth), a few times per
/// candidate, so their bounds are measured values with some margin
/// (docs/REQUIREMENTS.md, NFR-4).
fn allowed(only: Option<&str>, effort: Effort, texture: bool) -> usize {
    if !texture {
        return 0;
    }
    let (fast, balanced, exhaustive) = match only {
        Some("linear" | "datamatrix") => (0, 0, 0),
        Some("qr") => (4, 16, 24),
        Some("pdf417") => (0, 60, 100),
        Some("aztec") => (600, 10_000, 25_000),
        _ => (700, 11_000, 26_000),
    };
    match effort {
        Effort::Fast => fast,
        Effort::Balanced => balanced,
        Effort::Exhaustive => exhaustive,
    }
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "release only: cargo test --release -p breader --test allocation"
)]
fn steady_state_scans_do_not_allocate() {
    let readers = [
        Some("linear"),
        Some("qr"),
        Some("datamatrix"),
        Some("pdf417"),
        Some("aztec"),
        None,
    ];
    // 800x600 is above the up-scale retry limit; 320x240 takes the retry. The
    // small frames are measured for the full pipeline (`all`) only.
    let frames = [
        blank(800, 600),
        texture(800, 600),
        blank(320, 240),
        texture(320, 240),
        qr_frame(800, 600),
    ];
    let mut failures = Vec::new();
    println!(
        "{:<11} {:<8} {:<8} {:>12} {:>12} {:>12}",
        "reader", "frame", "size", "Fast", "Balanced", "Exhaustive"
    );
    for only in readers {
        for frame in &frames {
            if only.is_some() && frame.w < 800 {
                continue;
            }
            let mut cells = Vec::new();
            for effort in EFFORTS {
                let (n, bytes) = measure(only, effort, frame);
                cells.push(format!("{n}/{}K", bytes.div_ceil(1024)));
                // With rayon, the 3x up-scaled retry copy of a small frame is large
                // enough for the parallel kernels, whose tasks allocate.
                let parallel_retry = cfg!(feature = "rayon") && frame.w <= 600;
                if frame.symbols == 0
                    && !parallel_retry
                    && n > allowed(only, effort, frame.name == "texture")
                {
                    failures.push(format!(
                        "{} {} {}x{} {effort:?}: {n} allocations, {bytes} bytes",
                        only.unwrap_or("all"),
                        frame.name,
                        frame.w,
                        frame.h
                    ));
                }
            }
            println!(
                "{:<11} {:<8} {:<8} {:>12} {:>12} {:>12}",
                only.unwrap_or("all"),
                frame.name,
                format!("{}x{}", frame.w, frame.h),
                cells[0],
                cells[1],
                cells[2]
            );
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
