use std::hint::black_box;

use breader_core::{Binarizer, BitMatrix, ImageBuffer, ImageView, PixelFormat, Preprocessor};
use breader_imgproc::{Downscale, LocalBinarizer, OtsuBinarizer, to_luma};
use criterion::{Criterion, criterion_group, criterion_main};

fn synthetic_luma(w: usize, h: usize) -> Vec<u8> {
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let gradient = (x * 100 / w + y * 60 / h) as u64;
            let pattern = if ((x / 6) ^ (y / 6)) & 1 == 0 { 40 } else { 0 };
            out.push((gradient + pattern + (state & 31)) as u8);
        }
    }
    out
}

fn bench_all(c: &mut Criterion) {
    let (w, h) = (1920usize, 1080usize);
    let luma = synthetic_luma(w, h);
    let rgba: Vec<u8> = luma.iter().flat_map(|&v| [v, v / 2, v / 3, 255]).collect();

    let mut dst = ImageBuffer::default();
    c.bench_function("to_luma_rgba_1920x1080", |b| {
        b.iter(|| {
            to_luma(black_box(&rgba), PixelFormat::Rgba8, w, h, w * 4, &mut dst).unwrap();
        });
    });

    let view = ImageView::from_packed(&luma, w, h).unwrap();
    let mut bits = BitMatrix::new(w, h);
    c.bench_function("otsu_1920x1080", |b| {
        b.iter(|| OtsuBinarizer.binarize(black_box(view), &mut bits));
    });
    c.bench_function("local_1920x1080", |b| {
        b.iter(|| LocalBinarizer::default().binarize(black_box(view), &mut bits));
    });

    let small = synthetic_luma(640, 480);
    let small_view = ImageView::from_packed(&small, 640, 480).unwrap();
    let mut small_bits = BitMatrix::new(640, 480);
    c.bench_function("local_640x480", |b| {
        b.iter(|| LocalBinarizer::default().binarize(black_box(small_view), &mut small_bits));
    });

    let mut scaled = ImageBuffer::default();
    c.bench_function("downscale2_1920x1080", |b| {
        b.iter(|| Downscale::new(2).apply(black_box(view), &mut scaled));
    });
}

criterion_group!(benches, bench_all);
criterion_main!(benches);
