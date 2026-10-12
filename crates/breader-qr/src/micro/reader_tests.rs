//! End-to-end tests of the Micro QR path through the pipeline, and the
//! negative set: other symbologies and noise must never read as Micro QR.

use breader_core::{
    Binarizer, BitMatrix, EcLevel, Effort, ImageView, Perspective, Pipeline, Point, ScanOptions,
    Scratch, Symbol, SymbolMeta, Symbology, SymbologySet,
};
use qrcode::{Color, EcLevel as QEc, QrCode, Version};

use super::tests::{build, encode, numeric_stream, standard_pad};
use crate::QrReader;

use crate::testrender::{Modules, Render, similarity_quad};

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

fn modules(g: &BitMatrix) -> Modules {
    let dim = g.width();
    let dark = (0..dim * dim).map(|i| g.get(i % dim, i / dim)).collect();
    Modules { dim, dark }
}

/// Quiet zone of two modules around the symbol, margin around that.
fn draw(
    m: &Modules,
    px: f32,
    deg: f32,
    blur: usize,
    mirrored: bool,
    invert: bool,
) -> (Vec<u8>, usize, usize) {
    let side = ((m.dim as f32 + 8.0) * px * 1.5) as usize + 20;
    let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
    let r = Render {
        w: side,
        h: side,
        quad: similarity_quad(m.dim, center, px, deg),
        mirrored,
        blur,
        invert,
    };
    (r.draw(m), side, side)
}

fn scan(img: &[u8], w: usize, h: usize, effort: Effort) -> Vec<Symbol> {
    let view = ImageView::from_packed(img, w, h).unwrap();
    pipeline().scan(view, &ScanOptions::with_effort(effort), &mut Scratch::new())
}

fn micro_count(out: &[Symbol]) -> usize {
    out.iter()
        .filter(|s| s.symbology == Symbology::MicroQr)
        .count()
}

const CASES: [(u8, EcLevel, &str); 7] = [
    (1, EcLevel::DetectionOnly, "12345"),
    (2, EcLevel::L, "1234567890"),
    (2, EcLevel::M, "ABC12"),
    (3, EcLevel::M, "Hello1"),
    (4, EcLevel::L, "Micro QR 2015"),
    (4, EcLevel::M, "0123456789012345"),
    (4, EcLevel::Q, "abc123"),
];

#[test]
fn every_version_and_level_at_every_rotation() {
    for (version, level, text) in CASES {
        let g = encode(text.as_bytes(), version, level).unwrap();
        let m = modules(&g);
        for deg in [0.0, 17.0, 45.0, 90.0, 133.0, 180.0, 271.0, 340.0] {
            let (img, w, h) = draw(&m, 4.0, deg, 0, false, false);
            for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
                let out = scan(&img, w, h, effort);
                assert_eq!(out.len(), 1, "M{version} {level:?} {deg} {effort:?}");
                let s = &out[0];
                assert_eq!(s.symbology, Symbology::MicroQr);
                assert_eq!(s.data, text.as_bytes());
                assert_eq!(&s.aim_id, b"]Q1");
                let SymbolMeta::Qr(meta) = &s.meta else {
                    panic!("meta")
                };
                assert_eq!((meta.version, meta.ec_level), (version, level));
                assert!(!meta.mirrored);
            }
        }
    }
}

#[test]
fn mirrored_inverted_and_blurred() {
    for (version, level, text) in CASES {
        let g = encode(text.as_bytes(), version, level).unwrap();
        let m = modules(&g);
        let (img, w, h) = draw(&m, 5.0, 23.0, 0, true, false);
        let out = scan(&img, w, h, Effort::Balanced);
        assert_eq!(out.len(), 1, "mirrored M{version}");
        assert_eq!(out[0].data, text.as_bytes());
        let SymbolMeta::Qr(meta) = &out[0].meta else {
            panic!("meta")
        };
        assert!(meta.mirrored);
        let (img, w, h) = draw(&m, 5.0, 100.0, 0, false, true);
        let out = scan(&img, w, h, Effort::Fast);
        assert_eq!(out.len(), 1, "inverted M{version}");
        assert_eq!(out[0].data, text.as_bytes());
        let (img, w, h) = draw(&m, 6.0, 8.0, 2, false, false);
        let out = scan(&img, w, h, Effort::Balanced);
        assert_eq!(out.len(), 1, "blurred M{version}");
    }
}

#[test]
fn perspective_view() {
    let g = encode(b"MICRO QR 123", 4, EcLevel::L).unwrap();
    let m = modules(&g);
    let (w, h) = (260usize, 260usize);
    let r = Render {
        w,
        h,
        quad: [
            Point::new(50.0, 40.0),
            Point::new(200.0, 62.0),
            Point::new(190.0, 205.0),
            Point::new(60.0, 180.0),
        ],
        mirrored: false,
        blur: 0,
        invert: false,
    };
    let img = r.draw(&m);
    let out = scan(&img, w, h, Effort::Balanced);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].data, b"MICRO QR 123");
    // The quad follows the symbol's own orientation.
    let q = out[0].quad.corners;
    assert!(q[0].distance(Point::new(50.0, 40.0)) < 6.0, "{q:?}");
    let _ = Perspective::quad_to_quad;
}

#[test]
fn small_modules() {
    let mut missed = Vec::new();
    for (version, level, text) in CASES {
        let g = encode(text.as_bytes(), version, level).unwrap();
        let m = modules(&g);
        for px in [2.0, 2.5, 3.0] {
            for deg in [0.0, 12.0] {
                let (img, w, h) = draw(&m, px, deg, 0, false, false);
                let out = scan(&img, w, h, Effort::Balanced);
                if out.len() != 1 || out[0].data != text.as_bytes() {
                    missed.push((version, px, deg));
                }
            }
        }
    }
    eprintln!("missed {missed:?}");
    assert!(missed.iter().all(|m| m.1 < 2.5), "{missed:?}");
}

#[test]
fn the_symbology_option_selects_qr_and_micro_qr() {
    let g = encode(b"12345", 1, EcLevel::DetectionOnly).unwrap();
    let (img, w, h) = draw(&modules(&g), 5.0, 10.0, 0, false, false);
    let view = ImageView::from_packed(&img, w, h).unwrap();
    let p = pipeline();
    let only = |set: SymbologySet| {
        let o = ScanOptions {
            symbologies: set,
            ..ScanOptions::default()
        };
        p.scan(view, &o, &mut Scratch::new())
    };
    assert_eq!(micro_count(&only(SymbologySet::all())), 1);
    assert_eq!(
        micro_count(&only(SymbologySet::EMPTY.with(Symbology::MicroQr))),
        1
    );
    assert!(only(SymbologySet::EMPTY.with(Symbology::Qr)).is_empty());
    // A QR code is still read when only QR is enabled.
    let code = QrCode::with_version(b"HELLO", Version::Normal(1), QEc::M).unwrap();
    let dim = code.width();
    let qm = Modules {
        dim,
        dark: code.to_colors().iter().map(|c| *c == Color::Dark).collect(),
    };
    let (img, w, h) = draw(&qm, 5.0, 10.0, 0, false, false);
    let view = ImageView::from_packed(&img, w, h).unwrap();
    let o = ScanOptions {
        symbologies: SymbologySet::EMPTY.with(Symbology::MicroQr),
        ..ScanOptions::default()
    };
    assert!(p.scan(view, &o, &mut Scratch::new()).is_empty());
}

#[test]
fn symbols_with_bad_padding_or_data_after_the_terminator_are_not_read() {
    let s = numeric_stream(2, "12");
    let g = build(2, EcLevel::L, s.clone(), 2, standard_pad);
    let (img, w, h) = draw(&modules(&g), 5.0, 30.0, 0, false, false);
    assert_eq!(micro_count(&scan(&img, w, h, Effort::Exhaustive)), 1);
    let g = build(2, EcLevel::L, s, 2, |s, total| {
        standard_pad(s, total);
        let n = s.len();
        s[n - 3] = !s[n - 3];
    });
    let (img, w, h) = draw(&modules(&g), 5.0, 30.0, 0, false, false);
    assert_eq!(micro_count(&scan(&img, w, h, Effort::Exhaustive)), 0);
}

/// QR codes of every version, damaged and occluded in ways that leave the
/// finders alone, never produce a Micro QR result.
#[test]
fn qr_codes_are_never_micro_qr() {
    for version in [1i16, 2, 3, 5, 7, 12] {
        let data: Vec<u8> = (0..version as usize * 5)
            .map(|i| b'A' + (i % 26) as u8)
            .collect();
        let code = QrCode::with_version(&data, Version::Normal(version), QEc::M).unwrap();
        let dim = code.width();
        let colors = code.to_colors();
        let base = Modules {
            dim,
            dark: colors.iter().map(|c| *c == Color::Dark).collect(),
        };
        let mut variants = vec![base.dark.clone()];
        // Wipe the data area: finders and timing stay.
        let mut wiped = base.dark.clone();
        for y in 9..dim {
            for x in 9..dim {
                wiped[y * dim + x] = false;
            }
        }
        variants.push(wiped);
        // Occlude the top-right and bottom-left finders.
        let mut occ = base.dark.clone();
        for y in 0..9 {
            for x in dim - 9..dim {
                occ[y * dim + x] = false;
            }
        }
        for y in dim - 9..dim {
            for x in 0..9 {
                occ[y * dim + x] = false;
            }
        }
        variants.push(occ);
        // Random damage over the data area.
        let mut noisy = base.dark.clone();
        let mut seed = 99u32;
        for v in noisy.iter_mut() {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if seed >> 28 < 3 {
                *v = !*v;
            }
        }
        variants.push(noisy);
        for dark in variants {
            let m = Modules { dim, dark };
            for deg in [0.0, 30.0, 90.0, 200.0] {
                for px in [3.0, 5.0] {
                    let (img, w, h) = draw(&m, px, deg, 0, false, false);
                    for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
                        let out = scan(&img, w, h, effort);
                        assert_eq!(micro_count(&out), 0, "v{version} {deg} {px} {effort:?}");
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
fn noise_and_patterns_are_never_micro_qr() {
    let (w, h) = (300usize, 220usize);
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
    // Stripes and blobs.
    images.push(
        (0..w * h)
            .map(|i| if (i % w) / 3 % 2 == 0 { 0 } else { 255 })
            .collect(),
    );
    images.push(
        (0..w * h)
            .map(|i| {
                if (i % w / 7 + i / w / 5) % 3 == 0 {
                    0
                } else {
                    255
                }
            })
            .collect(),
    );
    // Random 7x7 rings with random surroundings.
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
            if lcg(&mut seed) < 128 {
                for d in 0..3usize {
                    ringy[(cy + d) * w + (cx + 24 + t * 3).min(w - 4)] = 0;
                }
            }
        }
    }
    images.push(ringy);
    for img in &images {
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            assert_eq!(micro_count(&scan(img, w, h, effort)), 0, "{effort:?}");
        }
    }
}
