use std::time::{Duration, Instant};

use breader_core::{
    Binarizer, BitMatrix, Effort, FrameContext, ImageView, Perspective, Pipeline, Point,
    ScanOptions, Scratch, Symbology,
};

use super::*;
use crate::detect::FinderSearch;

mod render;
use render::{Modules, Render, make_modules, similarity_quad, threshold};

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

fn payload(version: i16) -> Vec<u8> {
    let n = (version as usize * 4).min(40);
    (0..n).map(|i| b'A' + (i % 26) as u8).collect()
}

/// Detects, samples at the best triplet and returns the grid.
fn sample_best(bin: &BitMatrix, expect_dim: usize) -> Option<(BitMatrix, Triplet)> {
    let mut s = FinderSearch::default();
    s.find_finders(bin, 1);
    s.select_triplets(bin, 30, 1);
    let t = *s.triplets.iter().find(|t| {
        let (dims, n) = candidate_dims(t.dim);
        t.dim_hint == expect_dim || dims[..n].contains(&expect_dim)
    })?;
    let mut runs = Vec::new();
    let a = (expect_dim >= 25)
        .then(|| {
            let pred = predict_alignment(&t, expect_dim);
            let module = (t.tl.module + t.tr.module + t.bl.module) / 3.0;
            find_alignment(
                bin,
                &mut runs,
                pred,
                module_axes(&t, expect_dim),
                module * (4.0 + expect_dim as f32 / 16.0),
            )
        })
        .flatten();
    let h = transform(&t, expect_dim, a, Point::default())?;
    let mut grid = BitMatrix::default();
    let m = module_size(&h, expect_dim)?;
    sample_grid(bin, &h, expect_dim, m, &mut grid).then_some((grid, t))
}

fn agreement(grid: &BitMatrix, m: &Modules, transposed: bool) -> f32 {
    let mut ok = 0;
    for y in 0..m.dim {
        for x in 0..m.dim {
            let want = if transposed { m.get(y, x) } else { m.get(x, y) };
            ok += usize::from(grid.get(x, y) == want);
        }
    }
    ok as f32 / (m.dim * m.dim) as f32
}

fn render_case(
    version: i16,
    px: f32,
    deg: f32,
    blur: usize,
    mirrored: bool,
) -> (Modules, Render, Vec<u8>) {
    let m = make_modules(version, &payload(version));
    let side = ((m.dim as f32 + 8.0) * px * 1.5) as usize + 20;
    let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
    let r = Render {
        w: side,
        h: side,
        quad: similarity_quad(m.dim, center, px, deg),
        mirrored,
        blur,
        invert: false,
    };
    let img = r.draw(&m);
    (m, r, img)
}

#[test]
fn finders_and_triplet_clean() {
    for (version, px) in [(1, 4.0), (2, 3.0), (7, 4.0), (10, 3.0), (25, 3.0)] {
        for deg in [0.0, 17.0, 45.0, 90.0, 133.0, 200.0, 291.0] {
            let (m, r, img) = render_case(version, px, deg, 0, false);
            let bin = threshold(&img, r.w, r.h);
            let mut s = FinderSearch::default();
            s.find_finders(&bin, 1);
            s.select_triplets(&bin, 30, 1);
            let t = s
                .triplets
                .first()
                .unwrap_or_else(|| panic!("v{version} {deg}: no triplet"));
            let at = |x: f32, y: f32| {
                let q = r.quad;
                let h = Perspective::quad_to_quad(
                    [
                        Point::new(0.0, 0.0),
                        Point::new(m.dim as f32, 0.0),
                        Point::new(m.dim as f32, m.dim as f32),
                        Point::new(0.0, m.dim as f32),
                    ],
                    q,
                )
                .unwrap();
                h.map(Point::new(x, y)).unwrap()
            };
            let d = m.dim as f32;
            let tol = px * 0.7;
            assert!(t.tl.pos.distance(at(3.5, 3.5)) < tol, "v{version} {deg} tl");
            assert!(
                t.tr.pos.distance(at(d - 3.5, 3.5)) < tol,
                "v{version} {deg} tr"
            );
            assert!(
                t.bl.pos.distance(at(3.5, d - 3.5)) < tol,
                "v{version} {deg} bl"
            );
            let (dims, n) = candidate_dims(t.dim);
            assert!(
                dims[..n].contains(&m.dim),
                "v{version} {deg} dim est {}",
                t.dim
            );
        }
    }
}

#[test]
fn sampling_matches_modules() {
    let cases = [
        (1, 5.0),
        (2, 4.0),
        (5, 3.0),
        (7, 4.0),
        (10, 3.0),
        (20, 3.0),
        (25, 3.0),
        (40, 3.0),
    ];
    for (version, px) in cases {
        for deg in [0.0, 23.0, 45.0, 100.0, 180.0, 250.0] {
            let (m, r, img) = render_case(version, px, deg, 0, false);
            let bin = threshold(&img, r.w, r.h);
            let (grid, _) =
                sample_best(&bin, m.dim).unwrap_or_else(|| panic!("v{version} {deg} no sample"));
            let a = agreement(&grid, &m, false);
            assert!(a >= 0.995, "v{version} px{px} deg{deg}: agreement {a}");
        }
    }
}

#[test]
fn sampling_blur_and_small() {
    for (version, px, blur) in [(3, 5.0, 1), (7, 4.0, 1), (4, 2.0, 0), (10, 2.0, 0)] {
        let (m, r, img) = render_case(version, px, 31.0, blur, false);
        let bin = threshold(&img, r.w, r.h);
        let (grid, _) = sample_best(&bin, m.dim).unwrap_or_else(|| panic!("v{version} no sample"));
        let a = agreement(&grid, &m, false);
        assert!(a >= 0.97, "v{version} px{px} blur{blur}: agreement {a}");
    }
}

#[test]
fn mirrored_gives_transposed_grid() {
    for version in [2, 7] {
        let (m, r, img) = render_case(version, 4.0, 10.0, 0, true);
        let bin = threshold(&img, r.w, r.h);
        let (grid, _) = sample_best(&bin, m.dim).unwrap();
        let direct = agreement(&grid, &m, false);
        let transposed = agreement(&grid, &m, true);
        assert!(
            transposed >= 0.995 || direct >= 0.995,
            "{direct} {transposed}"
        );
    }
}

#[test]
fn perspective_sampling() {
    for version in [2, 6, 10, 25] {
        let m = make_modules(version, &payload(version));
        let px = 4.0;
        let s = m.dim as f32 * px;
        let (w, h) = ((s * 1.6) as usize, (s * 1.6) as usize);
        let o = s * 0.3;
        let quad = [
            Point::new(o + s * 0.05, o),
            Point::new(o + s * 1.0, o + s * 0.1),
            Point::new(o + s * 0.9, o + s * 1.0),
            Point::new(o, o + s * 0.85),
        ];
        let r = Render {
            w,
            h,
            quad,
            mirrored: false,
            blur: 0,
            invert: false,
        };
        let img = r.draw(&m);
        let bin = threshold(&img, w, h);
        let (grid, _) = sample_best(&bin, m.dim).unwrap_or_else(|| panic!("v{version}"));
        let a = agreement(&grid, &m, false);
        assert!(a >= 0.99, "v{version}: {a}");
    }
}

fn lcg(state: &mut u64) -> u8 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (*state >> 56) as u8
}

#[test]
fn garbage_images_do_not_panic() {
    let p = pipeline();
    let start = Instant::now();
    let (w, h) = (400usize, 300usize);
    let mut seed = 7u64;
    let mut images: Vec<Vec<u8>> = Vec::new();
    images.push((0..w * h).map(|_| lcg(&mut seed)).collect());
    images.push(
        (0..w * h)
            .map(|i| if (i % w) / 3 % 2 == 0 { 0 } else { 255 })
            .collect(),
    );
    images.push(
        (0..w * h)
            .map(|i| {
                if (i % w) / 2 % 2 == (i / w) / 2 % 2 {
                    0
                } else {
                    255
                }
            })
            .collect(),
    );
    images.push(
        (0..w * h)
            .map(|i| {
                if (i % w) / 7 % 2 == (i / w) / 7 % 2 {
                    0
                } else {
                    255
                }
            })
            .collect(),
    );
    images.push(vec![0; w * h]);
    images.push(vec![255; w * h]);
    for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
        for img in &images {
            let view = ImageView::from_packed(img, w, h).unwrap();
            let opts = ScanOptions::with_effort(effort);
            let out = p.scan(view, &opts, &mut Scratch::new());
            assert!(out.iter().all(|s| s.symbology == Symbology::Qr));
        }
    }
    assert!(
        start.elapsed() < Duration::from_secs(60),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn tiny_images() {
    let p = pipeline();
    for (w, h) in [(1, 1), (5, 5), (21, 21), (22, 3), (3, 300)] {
        let img = vec![0u8; w * h];
        let view = ImageView::from_packed(&img, w, h).unwrap();
        let _ = p.scan(view, &ScanOptions::default(), &mut Scratch::new());
    }
}

fn scan_one(img: &[u8], w: usize, h: usize, effort: Effort) -> Vec<Symbol> {
    let view = ImageView::from_packed(img, w, h).unwrap();
    pipeline().scan(view, &ScanOptions::with_effort(effort), &mut Scratch::new())
}

#[test]
fn end_to_end_rotations_and_versions() {
    for version in [1, 2, 4, 7, 10, 15, 25, 40] {
        for deg in [0.0, 13.0, 45.0, 90.0, 181.0, 300.0] {
            let (m, r, img) = render_case(version, 4.0, deg, 0, false);
            let out = scan_one(&img, r.w, r.h, Effort::Balanced);
            assert_eq!(out.len(), 1, "v{version} {deg}");
            assert_eq!(out[0].data, payload(version), "v{version} {deg}");
            assert_eq!(
                out[0].text.as_deref().map(str::as_bytes),
                Some(&payload(version)[..])
            );
            let SymbolMeta::Qr(meta) = &out[0].meta else {
                panic!()
            };
            assert_eq!(usize::from(meta.version), (m.dim - 17) / 4);
            assert!(!meta.mirrored);
            let c = out[0].quad.center();
            assert!(c.distance(Point::new(r.w as f32 / 2.0, r.h as f32 / 2.0)) < 4.0);
            let d = (out[0].orientation_deg - deg).rem_euclid(360.0);
            assert!(
                !(3.0..=357.0).contains(&d),
                "orientation {} vs {deg}",
                out[0].orientation_deg
            );
        }
    }
}

#[test]
fn end_to_end_mirrored_inverted_blurred() {
    let (_, r, img) = render_case(5, 4.0, 20.0, 0, true);
    let out = scan_one(&img, r.w, r.h, Effort::Balanced);
    assert_eq!(out.len(), 1);
    let SymbolMeta::Qr(meta) = &out[0].meta else {
        panic!()
    };
    assert!(meta.mirrored);

    let (_, r, img) = render_case(5, 4.0, 20.0, 1, false);
    let inv: Vec<u8> = img.iter().map(|p| 255 - p).collect();
    assert_eq!(scan_one(&inv, r.w, r.h, Effort::Balanced).len(), 1);
    assert!(scan_one(&inv, r.w, r.h, Effort::Fast).is_empty());
    assert_eq!(scan_one(&img, r.w, r.h, Effort::Fast).len(), 1);
}

#[test]
fn end_to_end_two_symbols_and_skip() {
    let a = make_modules(3, b"FIRST");
    let b = make_modules(3, b"SECOND");
    let (w, h) = (360usize, 200usize);
    let mut img = vec![255u8; w * h];
    for (m, cx) in [(&a, 90.0), (&b, 260.0)] {
        let r = Render {
            w,
            h,
            quad: similarity_quad(m.dim, Point::new(cx, 100.0), 4.0, 8.0),
            mirrored: false,
            blur: 0,
            invert: false,
        };
        for (d, s) in img.iter_mut().zip(r.draw(m)) {
            *d = (*d).min(s);
        }
    }
    let out = scan_one(&img, w, h, Effort::Balanced);
    assert_eq!(out.len(), 2);
    let mut texts: Vec<_> = out.iter().map(|s| s.data.clone()).collect();
    texts.sort();
    assert_eq!(texts, [b"FIRST".to_vec(), b"SECOND".to_vec()]);
    // A second scan of the same frame skips symbols already in `out`.
    let view = ImageView::from_packed(&img, w, h).unwrap();
    let opts = ScanOptions::default();
    let mut scratch = Scratch::new();
    let binarizers: Vec<Box<dyn Binarizer>> = vec![Box::new(Threshold)];
    let mut ctx = FrameContext::new(view, &opts, &[], &binarizers, &mut scratch);
    let mut found = Vec::new();
    QrReader.scan(&mut ctx, &mut found);
    assert_eq!(found.len(), 2);
    QrReader.scan(&mut ctx, &mut found);
    assert_eq!(found.len(), 2);
}
