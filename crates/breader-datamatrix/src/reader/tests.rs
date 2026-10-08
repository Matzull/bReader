use breader_core::{
    Binarizer, BitMatrix, Effort, ImageView, Pipeline, ScanOptions, Scratch, Symbology,
};

use super::*;
use crate::testutil::*;

struct Threshold;

impl Binarizer for Threshold {
    fn id(&self) -> &'static str {
        "threshold"
    }
    fn binarize(&self, src: ImageView<'_>, out: &mut BitMatrix) {
        for (y, row) in src.rows().enumerate() {
            for (x, &p) in row.iter().enumerate() {
                out.set(x, y, p < 128);
            }
        }
    }
}

fn pipeline() -> Pipeline {
    Pipeline::builder()
        .binarizer(Threshold)
        .reader(DataMatrixReader)
        .build()
        .unwrap()
}

fn scan(luma: &[u8], w: usize, h: usize, effort: Effort) -> Vec<Symbol> {
    let img = ImageView::from_packed(luma, w, h).unwrap();
    pipeline().scan(img, &ScanOptions::with_effort(effort), &mut Scratch::new())
}

fn render_scene(
    m: &Modules,
    px: f32,
    deg: f32,
    mirrored: bool,
    blur: usize,
    invert: bool,
) -> (Vec<u8>, usize) {
    let diag = ((m.rows * m.rows + m.cols * m.cols) as f32).sqrt() * px;
    let side = (diag + 16.0 * px).ceil() as usize + 8;
    let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
    let quad = similarity_quad(m.rows, m.cols, center, px, deg);
    let r = Render {
        w: side,
        h: side,
        quad,
        mirrored,
        blur,
        invert,
    };
    (r.draw(m), side)
}

#[test]
fn reads_all_sizes_rotated() {
    let mut failures = Vec::new();
    for size in all_sizes() {
        let m = encode_size(b"reader", size);
        for deg in [0.0, 23.0, 90.0, 161.0, 270.0] {
            let px = if m.cols > 100 { 3.5 } else { 5.0 };
            let (luma, side) = render_scene(&m, px, deg, false, 0, false);
            let found = scan(&luma, side, side, Effort::Balanced);
            let ok = found.len() == 1
                && found[0].symbology == Symbology::DataMatrix
                && found[0].data == m.payload;
            if !ok {
                failures.push(format!(
                    "{}x{} deg {deg}: {} results",
                    m.rows,
                    m.cols,
                    found.len()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn reads_inverted_blurred_and_mirrored() {
    for (label, blur, invert, mirrored) in [
        ("inverted", 0, true, false),
        ("blurred", 1, false, false),
        ("mirrored", 0, false, true),
    ] {
        for size in [
            datamatrix::SymbolSize::Square20,
            datamatrix::SymbolSize::Rect16x48,
            datamatrix::SymbolSize::Square52,
        ] {
            let m = encode_size(b"variants", size);
            let (luma, side) = render_scene(&m, 5.0, 31.0, mirrored, blur, invert);
            let found = scan(&luma, side, side, Effort::Balanced);
            assert_eq!(found.len(), 1, "{label} {}x{}", m.rows, m.cols);
            assert_eq!(found[0].data, m.payload, "{label} {}x{}", m.rows, m.cols);
            let SymbolMeta::DataMatrix(meta) = &found[0].meta else {
                panic!("wrong meta");
            };
            assert_eq!((meta.rows, meta.cols), (m.rows as u16, m.cols as u16));
            if mirrored {
                assert!(meta.mirrored, "{label}");
            }
        }
    }
}

#[test]
fn quad_is_in_original_coordinates() {
    let m = encode_size(b"quad", datamatrix::SymbolSize::Square24);
    let (luma, side) = render_scene(&m, 6.0, 15.0, false, 0, false);
    let found = scan(&luma, side, side, Effort::Fast);
    assert_eq!(found.len(), 1);
    let center = found[0].quad.center();
    assert!(center.distance(Point::new(side as f32 / 2.0, side as f32 / 2.0)) < 3.0);
}

#[test]
fn junk_yields_nothing() {
    let (w, h) = (300usize, 200usize);
    let mut seed = 7u64;
    let mut noise = vec![0u8; w * h];
    for p in &mut noise {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        *p = if (seed >> 40) & 1 == 1 { 255 } else { 0 };
    }
    let stripes: Vec<u8> = (0..w * h)
        .map(|i| if (i % w / 3) % 2 == 0 { 0 } else { 255 })
        .collect();
    let checker: Vec<u8> = (0..w * h)
        .map(|i| {
            if (i % w / 4 + i / w / 4) % 2 == 0 {
                0
            } else {
                255
            }
        })
        .collect();
    for img in [noise, stripes, checker] {
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            assert!(scan(&img, w, h, effort).is_empty());
        }
    }
}

#[test]
fn reads_the_longest_rectangle() {
    let m = encode_size(b"reader", datamatrix::SymbolSize::Rect8x120);
    let (luma, side) = render_scene(&m, 3.5, 0.0, false, 0, false);
    let found = scan(&luma, side, side, Effort::Balanced);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].data, m.payload);
}
