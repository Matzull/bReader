//! End-to-end tests of the rMQR path through the pipeline, and the negative
//! set: other symbologies and noise must never read as rMQR.

use breader_core::{
    Binarizer, BitMatrix, EcLevel, Effort, ImageView, Perspective, Pipeline, Point, ScanOptions,
    Scratch, Symbol, SymbolMeta, Symbology, SymbologySet,
};
use qrcode::{Color, EcLevel as QEc, QrCode, Version};
use qrcode_generator::Segment;

use super::spec::{SIZES, VERSIONS};
use super::tests::{encode, grid_from};
use crate::QrReader;
use crate::testrender::{RectRender, similarity_rect};

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
        .reader(QrReader)
        .build()
        .unwrap()
}

/// The symbol with a quiet zone of two modules, rotated by `deg`, in a
/// square image with room for any rotation.
fn draw(
    g: &BitMatrix,
    px: f32,
    deg: f32,
    blur: usize,
    mirrored: bool,
    invert: bool,
) -> (Vec<u8>, usize, usize) {
    let side = ((g.width().max(g.height()) as f32 + 8.0) * px * 1.2) as usize + 20;
    let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
    let r = RectRender {
        w: side,
        h: side,
        quad: similarity_rect(g.width(), g.height(), center, px, deg),
        mirrored,
        blur,
        invert,
    };
    (r.draw(g), side, side)
}

fn scan(img: &[u8], w: usize, h: usize, effort: Effort) -> Vec<Symbol> {
    let view = ImageView::from_packed(img, w, h).unwrap();
    pipeline().scan(view, &ScanOptions::with_effort(effort), &mut Scratch::new())
}

fn rmqr_count(out: &[Symbol]) -> usize {
    out.iter()
        .filter(|s| s.symbology == Symbology::Rmqr)
        .count()
}

fn digits(n: usize) -> String {
    (0..n)
        .map(|k| char::from(b'0' + ((k * 7 + k / 3) % 10) as u8))
        .collect()
}

/// A numeric payload of about half the capacity of `version` at `level`.
fn symbol(version: usize, high: bool) -> (BitMatrix, String) {
    let mut n = 1;
    while encode(version, high, &[Segment::numeric(digits(n + 1)).unwrap()]).is_some() {
        n += 1;
    }
    let s = digits((n / 2).max(1));
    let g = encode(version, high, &[Segment::numeric(&s).unwrap()]).unwrap();
    (g, s)
}

fn check(out: &[Symbol], text: &str, version: usize, high: bool, what: &str) {
    assert_eq!(rmqr_count(out), 1, "{what}: {out:?}");
    let s = out.iter().find(|s| s.symbology == Symbology::Rmqr).unwrap();
    assert_eq!(s.data, text.as_bytes(), "{what}");
    assert_eq!(&s.aim_id, b"]Q1");
    let SymbolMeta::Qr(m) = &s.meta else {
        panic!("meta")
    };
    assert_eq!(usize::from(m.version), version + 1, "{what}");
    assert_eq!(m.ec_level, if high { EcLevel::H } else { EcLevel::M });
    assert_eq!(
        (usize::from(m.height), usize::from(m.width)),
        (usize::from(SIZES[version].0), usize::from(SIZES[version].1))
    );
    assert_eq!(m.mask, 0);
}

#[test]
fn every_version_and_level_upright() {
    for v in 0..VERSIONS {
        for high in [false, true] {
            let (g, text) = symbol(v, high);
            let (img, w, h) = draw(&g, 4.0, 0.0, 0, false, false);
            for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
                let out = scan(&img, w, h, effort);
                check(
                    &out,
                    &text,
                    v,
                    high,
                    &format!("v{v} high={high} {effort:?}"),
                );
            }
        }
    }
}

#[test]
fn rotations_mirror_inversion_and_blur() {
    for v in [0usize, 4, 10, 16, 21, 27, 31] {
        let (g, text) = symbol(v, v % 2 == 0);
        let high = v % 2 == 0;
        for deg in [0.0, 17.0, 45.0, 90.0, 133.0, 180.0, 271.0, 340.0] {
            let (img, w, h) = draw(&g, 4.0, deg, 0, false, false);
            let out = scan(&img, w, h, Effort::Balanced);
            check(&out, &text, v, high, &format!("v{v} {deg}"));
            assert!(
                !matches!(&out[0].meta, SymbolMeta::Qr(m) if m.mirrored),
                "v{v} {deg}"
            );
        }
        for deg in [0.0, 23.0, 100.0, 211.0] {
            let (img, w, h) = draw(&g, 4.0, deg, 0, true, false);
            let out = scan(&img, w, h, Effort::Balanced);
            check(&out, &text, v, high, &format!("mirrored v{v} {deg}"));
            assert!(matches!(&out[0].meta, SymbolMeta::Qr(m) if m.mirrored));
        }
        let (img, w, h) = draw(&g, 4.0, 10.0, 0, false, true);
        check(&scan(&img, w, h, Effort::Fast), &text, v, high, "inverted");
        let (img, w, h) = draw(&g, 5.0, 8.0, 1, false, false);
        check(
            &scan(&img, w, h, Effort::Balanced),
            &text,
            v,
            high,
            &format!("blurred v{v}"),
        );
    }
}

#[test]
fn perspective_view() {
    let (g, text) = symbol(26, false);
    let (w, h) = (700usize, 400usize);
    let r = RectRender {
        w,
        h,
        quad: [
            Point::new(50.0, 120.0),
            Point::new(650.0, 100.0),
            Point::new(640.0, 168.0),
            Point::new(60.0, 190.0),
        ],
        mirrored: false,
        blur: 0,
        invert: false,
    };
    let img = r.draw(&g);
    let out = scan(&img, w, h, Effort::Balanced);
    check(&out, &text, 26, false, "perspective");
    let q = out[0].quad.corners;
    assert!(q[0].distance(Point::new(50.0, 120.0)) < 8.0, "{q:?}");
    assert!(q[2].distance(Point::new(640.0, 168.0)) < 8.0, "{q:?}");
    let _ = Perspective::quad_to_quad;
}

#[test]
fn the_real_samples_as_images() {
    for (rows, text, px) in [
        (super::samples::R7X43_H, ",,", 2.0f32),
        (super::samples::R7X43_H, ",,", 5.0),
        (super::samples::R17X139_M, PI, 4.0),
    ] {
        let g = grid_from(rows);
        let (img, w, h) = draw(&g, px, 0.0, 0, false, false);
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            let out = scan(&img, w, h, effort);
            assert_eq!(rmqr_count(&out), 1, "{px} {effort:?}");
            assert_eq!(out[0].data, text.as_bytes());
        }
    }
}

const PI: &str = "3.1415926535897932384626433832795028841971693993751058209749445923078164062862089986280348253421170679821480865132823066470938446095505822317253594081";

#[test]
fn small_modules() {
    let mut missed = Vec::new();
    for v in [0usize, 5, 11, 17, 27, 31] {
        let (g, text) = symbol(v, false);
        for px in [2.5, 3.0, 3.5] {
            for deg in [0.0, 12.0] {
                let (img, w, h) = draw(&g, px, deg, 0, false, false);
                let out = scan(&img, w, h, Effort::Balanced);
                if rmqr_count(&out) != 1 || out[0].data != text.as_bytes() {
                    missed.push((v, px, deg));
                }
            }
        }
    }
    eprintln!("missed {missed:?}");
    assert!(
        missed.iter().all(|m| m.1 <= 3.0 && m.2 != 0.0),
        "{missed:?}"
    );
}

#[test]
fn the_symbology_option_selects_rmqr() {
    let (g, _) = symbol(3, false);
    let (img, w, h) = draw(&g, 4.0, 10.0, 0, false, false);
    let view = ImageView::from_packed(&img, w, h).unwrap();
    let p = pipeline();
    let only = |set: SymbologySet| {
        let o = ScanOptions::default().with_symbologies(set);
        p.scan(view, &o, &mut Scratch::new())
    };
    assert_eq!(rmqr_count(&only(SymbologySet::all())), 1);
    assert_eq!(
        rmqr_count(&only(SymbologySet::EMPTY.with(Symbology::Rmqr))),
        1
    );
    assert!(only(SymbologySet::EMPTY.with(Symbology::Qr)).is_empty());
    assert!(only(SymbologySet::EMPTY.with(Symbology::MicroQr)).is_empty());
}

#[test]
fn damaged_symbols_within_the_guard_still_read() {
    let (g, text) = symbol(12, false);
    let t = super::spec::template(12);
    let mut h = g.clone();
    for k in 0..3 {
        let (x, y) = t.data[k * 55 + 10];
        h.set(
            usize::from(x),
            usize::from(y),
            !h.get(usize::from(x), usize::from(y)),
        );
    }
    let (img, w, ht) = draw(&h, 4.0, 15.0, 0, false, false);
    check(
        &scan(&img, w, ht, Effort::Balanced),
        &text,
        12,
        false,
        "damaged",
    );
}

#[test]
fn a_damaged_left_format_is_read_from_the_right_copy() {
    let (g, text) = symbol(21, true);
    let mut h = g.clone();
    for &(x, y) in super::spec::format_left().iter().take(7) {
        h.set(x, y, !h.get(x, y));
    }
    let (img, w, ht) = draw(&h, 4.0, 20.0, 0, false, false);
    check(
        &scan(&img, w, ht, Effort::Balanced),
        &text,
        21,
        true,
        "right copy",
    );
    // Both copies destroyed: no result.
    for &(x, y) in super::spec::format_right(h.width(), h.height())
        .iter()
        .take(7)
    {
        h.set(x, y, !h.get(x, y));
    }
    let (img, w, ht) = draw(&h, 4.0, 20.0, 0, false, false);
    assert_eq!(rmqr_count(&scan(&img, w, ht, Effort::Exhaustive)), 0);
}

/// QR codes of several versions, intact and damaged, never read as rMQR.
#[test]
fn qr_codes_are_never_rmqr() {
    for version in [1i16, 2, 3, 5, 7, 12] {
        let data: Vec<u8> = (0..version as usize * 5)
            .map(|i| b'A' + (i % 26) as u8)
            .collect();
        let code = QrCode::with_version(&data, Version::Normal(version), QEc::M).unwrap();
        let dim = code.width();
        let colors = code.to_colors();
        let mut base = BitMatrix::new(dim, dim);
        for (i, c) in colors.iter().enumerate() {
            if *c == Color::Dark {
                base.set(i % dim, i / dim, true);
            }
        }
        let mut variants = vec![base.clone()];
        let mut wiped = base.clone();
        for y in 9..dim {
            for x in 9..dim {
                wiped.set(x, y, false);
            }
        }
        variants.push(wiped);
        let mut occ = base.clone();
        for y in 0..9 {
            for x in dim - 9..dim {
                occ.set(x, y, false);
            }
        }
        for y in dim - 9..dim {
            for x in 0..9 {
                occ.set(x, y, false);
            }
        }
        variants.push(occ);
        for g in variants {
            for deg in [0.0, 30.0, 90.0, 200.0] {
                for px in [3.0, 5.0] {
                    let (img, w, h) = draw(&g, px, deg, 0, false, false);
                    for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
                        let out = scan(&img, w, h, effort);
                        assert_eq!(rmqr_count(&out), 0, "v{version} {deg} {px} {effort:?}");
                    }
                }
            }
        }
    }
}

fn lcg(seed: &mut u64) -> u8 {
    *seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
    (*seed >> 56) as u8
}

#[test]
fn noise_and_patterns_are_never_rmqr() {
    let (w, h) = (400usize, 240usize);
    let mut seed = 5u64;
    let mut images: Vec<Vec<u8>> = vec![(0..w * h).map(|_| lcg(&mut seed)).collect()];
    for cell in [2usize, 3, 5, 8] {
        images.push(
            (0..w * h)
                .map(|i| {
                    if (i % w / cell + i / w / cell) % 2 == 0 {
                        0
                    } else {
                        255
                    }
                })
                .collect(),
        );
    }
    images.push(
        (0..w * h)
            .map(|i| if (i % w) / 3 % 2 == 0 { 0 } else { 255 })
            .collect(),
    );
    // Rings followed by random timing-like rows and columns.
    let mut ringy = vec![255u8; w * h];
    for k in 0..40usize {
        let (cx, cy) = (10 + (k * 37) % (w - 40), 10 + (k * 53) % (h - 40));
        for dy in 0..21usize {
            for dx in 0..21usize {
                let (i, j) = (dx / 3, dy / 3);
                let r = (i as i32 - 3).abs().max((j as i32 - 3).abs());
                if r != 2 {
                    ringy[(cy + dy) * w + cx + dx] = 0;
                }
            }
        }
        for t in 0..30usize {
            if t % 2 == 0 || lcg(&mut seed) < 40 {
                for d in 0..3usize {
                    ringy[(cy + d) * w + (cx + 24 + t * 3).min(w - 4)] = 0;
                }
            }
        }
    }
    images.push(ringy);
    for img in &images {
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            assert_eq!(rmqr_count(&scan(img, w, h, effort)), 0, "{effort:?}");
        }
    }
}
