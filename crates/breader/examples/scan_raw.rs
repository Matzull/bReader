//! Scans raw pixel buffers: a packed luma plane and an NV12 camera frame.
//!
//! The image file is only used to obtain pixels; in a real application the
//! buffers come from a camera, a framebuffer or another decoder.
//!
//! cargo run --example scan_raw --features image -- data/synth/clean/clean_0001.png

use breader::{PixelFormat, Reader};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: scan_raw <image>");
        std::process::exit(2);
    };
    let gray = image::open(&path).expect("readable image").into_luma8();
    let (w, h) = (gray.width() as usize, gray.height() as usize);
    let mut reader = Reader::new();

    // Luma8: a tightly packed buffer, stride == width.
    let found = reader
        .scan_pixels(gray.as_raw(), PixelFormat::Luma8, w, h, w)
        .expect("valid geometry");
    println!("luma8: {} symbol(s)", found.len());

    // NV12: the Y plane (stride bytes per row) followed by interleaved UV at
    // half height. Only the Y plane is read, so neutral chroma is enough here.
    // The stride is padded the way camera drivers often do.
    let stride = w.next_multiple_of(64);
    let mut nv12 = vec![128u8; stride * h + stride * h.div_ceil(2)];
    for (y, row) in gray.as_raw().chunks_exact(w).enumerate() {
        nv12[y * stride..y * stride + w].copy_from_slice(row);
    }
    let found_nv12 = reader
        .scan_pixels(&nv12, PixelFormat::Nv12, w, h, stride)
        .expect("valid geometry");
    println!("nv12:  {} symbol(s)", found_nv12.len());

    for s in &found_nv12 {
        println!("  {} {:?}", s.symbology.name(), s.text);
    }
    assert_eq!(found, found_nv12, "same pixels, same result");
}
