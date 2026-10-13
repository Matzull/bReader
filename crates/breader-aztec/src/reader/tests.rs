use breader_core::{
    Binarizer, BitMatrix, Effort, ImageView, Pipeline, Point, ScanOptions, Scratch, Symbol,
    SymbolMeta, Symbology,
};

use super::*;
use crate::build::{Bits, render_flagged, render_rune, stuff};
use crate::spec::Layout;
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
        .reader(AztecReader)
        .build()
        .unwrap()
}

pub(super) fn scan(luma: &[u8], w: usize, h: usize, effort: Effort) -> Vec<Symbol> {
    let img = ImageView::from_packed(luma, w, h).unwrap();
    pipeline().scan(img, &ScanOptions::with_effort(effort), &mut Scratch::new())
}

fn meta(s: &Symbol) -> &AztecMeta {
    let SymbolMeta::Aztec(m) = &s.meta else {
        panic!("wrong meta");
    };
    m
}

fn assert_read(found: &[Symbol], data: &[u8], what: &str) {
    assert_eq!(found.len(), 1, "{what}: {} symbols", found.len());
    assert_eq!(found[0].symbology, Symbology::Aztec, "{what}");
    assert_eq!(found[0].data, data, "{what}");
}

#[test]
fn reads_compact_and_full_symbols_of_every_size_rotated() {
    let mut failures = Vec::new();
    // The stage tests (detect, sample, decode) cover every layer count; this keeps five.
    for layers in [-1i32, -4, 1, 6, 32] {
        let (data, r) = rxing_fill(layers.unsigned_abs() as u64 + 40, 4000, layers).unwrap();
        let px = if r.grid.width() > 100 { 3.5 } else { 4.0 };
        for deg in [0.0, 23.0, 161.0] {
            let (luma, side, _) = scene(&r.grid, px, deg, 0.0, |_| {});
            let found = scan(&luma, side, side, Effort::Balanced);
            let ok = found.len() == 1 && found[0].data == data;
            if !ok {
                failures.push(format!(
                    "layers {layers} deg {deg}: {} results",
                    found.len()
                ));
            } else {
                let m = meta(&found[0]);
                assert_eq!(m.compact, layers < 0);
                assert_eq!(i32::from(m.layers), layers.abs());
                assert!(!m.mirrored && m.rune.is_none() && !m.reader_init);
                assert_eq!(found[0].aim_id, *b"]z0");
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn a_compact_and_a_full_symbol_of_the_same_size_are_told_apart() {
    // 19 modules each: compact with two layers and full-range with one.
    for layers in [-2, 1] {
        let (data, r) = rxing_fill(7, 40, layers).unwrap();
        assert_eq!(r.grid.width(), 19);
        let (luma, side, _) = scene(&r.grid, 5.0, 12.0, 0.0, |_| {});
        let found = scan(&luma, side, side, Effort::Balanced);
        assert_read(&found, &data, "same size");
        assert_eq!(meta(&found[0]).compact, layers < 0);
    }
}

#[test]
fn reads_in_perspective() {
    let mut failures = Vec::new();
    for layers in [-4i32, 5, 20] {
        let (data, r) = rxing_fill(60 + layers.unsigned_abs() as u64, 4000, layers).unwrap();
        for (deg, tilt) in [(-15.0, 0.2), (200.0, 0.15)] {
            let (luma, side, _) = scene(&r.grid, 4.5, deg, tilt, |_| {});
            let found = scan(&luma, side, side, Effort::Balanced);
            if found.len() != 1 || found[0].data != data {
                failures.push(format!("layers {layers} deg {deg} tilt {tilt}"));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn reads_blurred_noisy_and_small_symbols() {
    let (data, r) = rxing_fill(21, 60, 4).unwrap();
    for (label, px, blur, noise) in [
        ("blur", 5.0, 2, 0),
        ("noise", 5.0, 0, 40),
        ("both", 5.0, 1, 30),
        ("small", 3.0, 0, 0),
        ("small blur", 3.0, 1, 0),
    ] {
        let (luma, side, _) = scene(&r.grid, px, 8.0, 0.0, |s| {
            s.blur = blur;
            s.noise = noise;
        });
        assert_read(&scan(&luma, side, side, Effort::Balanced), &data, label);
    }
}

#[test]
fn reads_inverted_and_mirrored_symbols() {
    for layers in [-2, 3, 12] {
        let (data, r) = rxing_fill(31, 200, layers).unwrap();
        let (luma, side, _) = scene(&r.grid, 4.0, 31.0, 0.0, |s| s.invert = true);
        let found = scan(&luma, side, side, Effort::Balanced);
        assert_read(&found, &data, "inverted");
        assert!(!meta(&found[0]).mirrored);
        // A mirrored symbol is its transposed module grid.
        let mut mirrored = BitMatrix::new(r.grid.width(), r.grid.width());
        for y in 0..r.grid.width() {
            for x in 0..r.grid.width() {
                mirrored.set(y, x, r.grid.get(x, y));
            }
        }
        for deg in [0.0, 57.0, 180.0] {
            let (luma, side, _) = scene(&mirrored, 4.0, deg, 0.0, |_| {});
            let found = scan(&luma, side, side, Effort::Balanced);
            assert_read(&found, &data, "mirrored");
            assert!(meta(&found[0]).mirrored, "layers {layers} deg {deg}");
        }
        let (luma, side, _) = scene(&mirrored, 4.0, 90.0, 0.0, |s| s.invert = true);
        assert_read(
            &scan(&luma, side, side, Effort::Balanced),
            &data,
            "mirrored and inverted",
        );
    }
}

#[test]
fn fast_reads_clean_symbols_and_skips_inverted_ones() {
    let (data, r) = rxing_fill(5, 50, 3).unwrap();
    let (luma, side, _) = scene(&r.grid, 6.0, 15.0, 0.0, |_| {});
    assert_read(&scan(&luma, side, side, Effort::Fast), &data, "fast");
    let (luma, side, _) = scene(&r.grid, 6.0, 15.0, 0.0, |s| s.invert = true);
    assert!(scan(&luma, side, side, Effort::Fast).is_empty());
    assert_read(
        &scan(&luma, side, side, Effort::Exhaustive),
        &data,
        "exhaustive inverted",
    );
}

#[test]
fn reads_runes() {
    for value in [0u8, 1, 64, 68, 127, 200, 255] {
        let g = render_rune(value);
        for (deg, invert) in [(0.0, false), (37.0, false), (90.0, true)] {
            let (luma, side, _) = scene(&g, 8.0, deg, 0.0, |s| s.invert = invert);
            let found = scan(&luma, side, side, Effort::Balanced);
            assert_read(&found, format!("{value:03}").as_bytes(), "rune");
            let m = meta(&found[0]);
            assert_eq!(m.rune, Some(value));
            assert_eq!(found[0].aim_id, *b"]zC");
            assert_eq!(m.layers, 0);
        }
    }
}

#[test]
fn reads_reader_initialisation_symbols() {
    for compact in [true, false] {
        let layout = Layout { compact, layers: 1 };
        let mut bits = Bits::default();
        bits.push(1, 5).push(2, 5).push(3, 5); // " AB"
        let words = stuff(&bits.0, layout.word_size());
        let grid = render_flagged(layout, &words, true);
        let (luma, side, _) = scene(&grid, 6.0, 20.0, 0.0, |_| {});
        let found = scan(&luma, side, side, Effort::Balanced);
        assert_read(&found, b" AB", "reader init");
        assert!(meta(&found[0]).reader_init, "{compact}");
        // The same symbol without the flag is an ordinary one.
        let grid = render_flagged(layout, &words, false);
        let (luma, side, _) = scene(&grid, 6.0, 20.0, 0.0, |_| {});
        let found = scan(&luma, side, side, Effort::Balanced);
        assert_read(&found, b" AB", "no flag");
        assert!(!meta(&found[0]).reader_init);
    }
}

#[test]
fn a_symbol_at_the_image_border_is_read() {
    // No quiet zone: the symbol fills the image.
    let (data, r) = rxing_fill(9, 80, 2).unwrap();
    let n = r.grid.width();
    let px = 6usize;
    let mut luma = vec![235u8; n * px * n * px];
    for y in 0..n * px {
        for x in 0..n * px {
            if r.grid.get(x / px, y / px) {
                luma[y * n * px + x] = 25;
            }
        }
    }
    assert_read(
        &scan(&luma, n * px, n * px, Effort::Balanced),
        &data,
        "border",
    );
}

#[test]
fn reads_several_symbols_and_reports_where_they_are() {
    let (d1, r1) = rxing_fill(1, 50, -3).unwrap();
    let (d2, r2) = rxing_fill(2, 300, 8).unwrap();
    let (w, h) = (900usize, 500usize);
    let mut luma = vec![235u8; w * h];
    let mut quads = Vec::new();
    for (grid, cx, cy, deg) in [
        (&r1.grid, 200.0, 250.0, 20.0),
        (&r2.grid, 620.0, 250.0, -10.0),
    ] {
        let quad = quad_at(grid.width(), Point::new(cx, cy), 5.0, deg, 0.0);
        let r = Render {
            w,
            h,
            quad,
            blur: 0,
            invert: false,
            noise: 0,
            seed: 1,
        };
        let layer = r.draw(grid);
        for (dst, src) in luma.iter_mut().zip(&layer) {
            *dst = (*dst).min(*src);
        }
        quads.push(quad);
    }
    let found = scan(&luma, w, h, Effort::Balanced);
    assert_eq!(found.len(), 2);
    for (data, quad) in [(&d1, quads[0]), (&d2, quads[1])] {
        let s = found.iter().find(|s| &s.data == data).expect("both read");
        let want = Quad::new(quad).center();
        assert!(
            s.quad.center().distance(want) < 3.0,
            "{:?}",
            s.quad.center()
        );
        // The quad is the symbol outline, corner by corner, up to a quarter turn.
        let err = (0..4)
            .map(|k| {
                (0..4)
                    .map(|i| s.quad.corners[i].distance(quad[(i + k) % 4]))
                    .fold(0.0f32, f32::max)
            })
            .fold(f32::MAX, f32::min);
        assert!(err < 6.0, "{err}");
    }
}

#[test]
fn quad_is_in_original_coordinates() {
    let (_, r) = rxing_fill(4, 30, -2).unwrap();
    let (luma, side, _) = scene(&r.grid, 6.0, 15.0, 0.0, |_| {});
    let found = scan(&luma, side, side, Effort::Fast);
    assert_eq!(found.len(), 1);
    let center = found[0].quad.center();
    assert!(center.distance(Point::new(side as f32 / 2.0, side as f32 / 2.0)) < 3.0);
}

#[test]
fn corrects_damaged_modules_but_never_returns_wrong_data() {
    let (data, r) = rxing_fill(8, 120, 4).unwrap();
    let mut rng = Rng(99);
    let n = r.grid.width();
    // A few damaged modules in the outer layers are corrected.
    let mut g = r.grid.clone();
    for _ in 0..6 {
        let (x, y) = (rng.below(n), rng.below(3));
        let v = g.get(x, y);
        g.set(x, y, !v);
    }
    let (luma, side, _) = scene(&g, 5.0, 10.0, 0.0, |_| {});
    let found = scan(&luma, side, side, Effort::Balanced);
    assert_read(&found, &data, "damaged");
    assert!(meta(&found[0]).errors_corrected > 0);
    // A symbol with half of its modules randomised reads as nothing.
    for _ in 0..40 {
        let (x, y) = (rng.below(n), rng.below(n));
        if x.abs_diff(n / 2).max(y.abs_diff(n / 2)) > 7 {
            let v = g.get(x, y);
            g.set(x, y, !v);
        }
    }
    for _ in 0..n * n / 3 {
        let (x, y) = (rng.below(n), rng.below(n));
        if x.abs_diff(n / 2).max(y.abs_diff(n / 2)) > 7 {
            g.set(x, y, rng.below(2) == 1);
        }
    }
    let (luma, side, _) = scene(&g, 5.0, 10.0, 0.0, |_| {});
    for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
        assert!(scan(&luma, side, side, effort).is_empty(), "{effort:?}");
    }
}

fn qr_grid(text: &[u8]) -> BitMatrix {
    let qr = qrcode::QrCode::new(text).unwrap();
    let n = qr.width();
    let mut g = BitMatrix::new(n, n);
    for (i, c) in qr.to_colors().into_iter().enumerate() {
        if c == qrcode::Color::Dark {
            g.set(i % n, i / n, true);
        }
    }
    g
}

fn dm_grid(text: &[u8]) -> BitMatrix {
    let dm = datamatrix::DataMatrix::encode(text, datamatrix::SymbolList::default()).unwrap();
    let bm = dm.bitmap();
    let n = bm.width();
    let mut g = BitMatrix::new(n, n);
    for (i, &b) in bm.bits().iter().enumerate() {
        if b {
            g.set(i % n, i / n, true);
        }
    }
    g
}

#[test]
fn other_symbologies_and_targets_give_no_aztec() {
    let mut images: Vec<(String, Vec<u8>, usize)> = Vec::new();
    for (name, g) in [
        ("qr small", qr_grid(b"HELLO")),
        (
            "qr",
            qr_grid(b"https://example.org/not-an-aztec-code/0123456789"),
        ),
        ("qr large", qr_grid(&[b'x'; 400])),
        ("datamatrix", dm_grid(b"not an aztec 123456")),
        ("datamatrix small", dm_grid(b"1")),
    ] {
        for (deg, invert) in [(0.0, false), (33.0, false), (0.0, true)] {
            let (luma, side, _) = scene(&g, 5.0, deg, 0.0, |s| s.invert = invert);
            images.push((format!("{name} {deg} {invert}"), luma, side));
        }
    }
    // Concentric circles and squares, a target with a missing ring.
    for (name, square) in [("circles", false), ("squares", true), ("thin", false)] {
        let side = 240usize;
        let mut luma = vec![235u8; side * side];
        let width = if name == "thin" { 4.0 } else { 7.0 };
        for y in 0..side {
            for x in 0..side {
                let (dx, dy) = (x as f32 - 120.0, y as f32 - 120.0);
                let d = if square {
                    dx.abs().max(dy.abs())
                } else {
                    dx.hypot(dy)
                };
                let r = (d / width).floor() as i32;
                if r % 2 == 0 && r < 14 {
                    luma[y * side + x] = 25;
                }
            }
        }
        images.push((name.into(), luma, side));
    }
    // Textures: noise, stripes, a checkerboard, a grid of dots.
    let side = 260usize;
    let mut rng = Rng(5);
    let noise: Vec<u8> = (0..side * side)
        .map(|_| if rng.below(2) == 1 { 235 } else { 25 })
        .collect();
    let stripes: Vec<u8> = (0..side * side)
        .map(|i| {
            if (i % side / 4).is_multiple_of(2) {
                25
            } else {
                235
            }
        })
        .collect();
    let checker: Vec<u8> = (0..side * side)
        .map(|i| {
            if (i % side / 5 + i / side / 5).is_multiple_of(2) {
                25
            } else {
                235
            }
        })
        .collect();
    let dots: Vec<u8> = (0..side * side)
        .map(|i| {
            let (x, y) = (i % side % 12, i / side % 12);
            if (x as i32 - 6).pow(2) + (y as i32 - 6).pow(2) < 12 {
                25
            } else {
                235
            }
        })
        .collect();
    for (name, luma) in [
        ("noise", noise),
        ("stripes", stripes),
        ("checker", checker),
        ("dots", dots),
    ] {
        images.push((name.into(), luma, side));
    }
    for (name, luma, side) in &images {
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            let found = scan(luma, *side, *side, effort);
            assert!(found.is_empty(), "{name} {effort:?}: {found:?}");
        }
    }
}

#[test]
fn tiny_blank_and_degenerate_images_are_fine() {
    for (w, h) in [(1usize, 1usize), (14, 14), (15, 200), (200, 15), (64, 64)] {
        let luma = vec![200u8; w * h];
        assert!(scan(&luma, w, h, Effort::Exhaustive).is_empty());
        let dark = vec![10u8; w * h];
        assert!(scan(&dark, w, h, Effort::Exhaustive).is_empty());
    }
}

#[test]
fn a_symbol_is_found_only_once_across_passes_and_efforts() {
    let (data, r) = rxing_fill(77, 60, 2).unwrap();
    let (luma, side, _) = scene(&r.grid, 4.0, 5.0, 0.0, |_| {});
    for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
        assert_read(&scan(&luma, side, side, effort), &data, "once");
    }
}
