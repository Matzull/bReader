use breader_core::{BitMatrix, Plane, Point};

use super::*;
use crate::check;
use crate::detect::Detector;
use crate::testutil::*;

fn lay(plane: Plane<'_>, quad: &[Point; 4], sc: &mut SampleScratch) -> Option<Layout> {
    let m = precheck(plane, quad, sc)?;
    analyze(plane, quad, m, sc)
}

fn closest(d: &Detector, truth: &[Point; 4]) -> Option<crate::detect::Candidate> {
    d.candidates
        .iter()
        .min_by(|a, b| {
            let ea = quad_err(&a.quad, truth);
            let eb = quad_err(&b.quad, truth);
            ea.total_cmp(&eb)
        })
        .copied()
}

fn quad_err(a: &[Point; 4], b: &[Point; 4]) -> f32 {
    (0..4)
        .map(|s| {
            (0..4)
                .map(|i| a[i].distance(b[(i + s) % 4]))
                .fold(0.0, f32::max)
        })
        .fold(f32::MAX, f32::min)
}

struct Outcome {
    layout: Layout,
    hyp: Hypothesis,
    grid: BitMatrix,
}

fn run(bin: &BitMatrix, inverted: bool, truth: &[Point; 4], rows: usize, cols: usize) -> Outcome {
    let mut d = Detector::default();
    d.find(bin, inverted, 0, 4, 128, 32);
    let cand = closest(&d, truth).expect("candidate");
    let plane = Plane::new(bin, inverted);
    let mut sc = SampleScratch::default();
    let layout =
        lay(plane, &cand.quad, &mut sc).unwrap_or_else(|| panic!("no layout for {rows}x{cols}"));
    let mut hyps = [None; 3];
    snap_layout(&layout, &sc, &mut hyps, false);
    let hyp = hyps
        .iter()
        .flatten()
        .find(|h| {
            let (r, c) = h.grid_dims();
            (r, c) == (rows, cols)
        })
        .copied()
        .unwrap_or_else(|| {
            panic!(
                "size {rows}x{cols} not among hypotheses (counted {}x{})",
                layout.rows_est, layout.cols_est
            )
        });
    let h = refine(plane, &layout.corners, &hyp, &mut sc).expect("mapper");
    let (r, c) = hyp.grid_dims();
    let mut grid = BitMatrix::default();
    assert!(sample_grid(plane, &h, r, c, &mut grid));
    Outcome { layout, hyp, grid }
}

fn agreement(g: &BitMatrix, m: &Modules) -> f32 {
    assert_eq!((g.width(), g.height()), (m.cols, m.rows));
    let mut same = 0;
    for r in 0..m.rows {
        for c in 0..m.cols {
            same += usize::from(g.get(c, r) == m.get(c, r));
        }
    }
    same as f32 / (m.rows * m.cols) as f32
}

#[test]
fn sampled_grid_equals_source() {
    use datamatrix::SymbolSize::*;
    let sizes = [
        Square10, Square14, Square18, Square26, Square32, Square40, Square64, Square96, Square144,
        Rect8x18, Rect12x36, Rect16x48, Rect8x64, Rect26x40,
    ];
    let mut failures = Vec::new();
    for size in sizes {
        let m = encode_size(b"sampling check 0123456789", size);
        for deg in [0.0, 17.0, 90.0, 133.0, 275.0] {
            for px in [4.0, 5.5, 7.0] {
                let (bin, truth) = scene(&m, px, deg, false, 0, false);
                let o = run(&bin, false, &truth, m.rows, m.cols);
                assert!(!o.hyp.swapped);
                let a = agreement(&o.grid, &m);
                if a < 0.99 {
                    failures.push(format!("{}x{} deg {deg} px {px}: {a:.3}", m.rows, m.cols));
                }
                assert!(check::plausible(&o.grid, &o.hyp));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn orientation_corners() {
    let m = encode_size(b"orientation", datamatrix::SymbolSize::Rect16x48);
    for deg in [0.0, 60.0, 180.0, 250.0] {
        let (bin, truth) = scene(&m, 5.0, deg, false, 0, false);
        let o = run(&bin, false, &truth, m.rows, m.cols);
        // truth is TL, TR, BR, BL of the unmirrored symbol.
        for (a, b) in o.layout.corners.iter().zip(&truth) {
            assert!(a.distance(*b) < 4.0, "deg {deg}: {a:?} vs {b:?}");
        }
        assert_eq!((o.layout.rows_est, o.layout.cols_est), (m.rows, m.cols));
    }
}

#[test]
fn inverted_polarity() {
    let m = encode_size(b"inverted", datamatrix::SymbolSize::Square22);
    let (mut bin, truth) = scene(&m, 4.0, 40.0, false, 0, false);
    bin.invert();
    let o = run(&bin, true, &truth, m.rows, m.cols);
    assert!(agreement(&o.grid, &m) >= 0.99);
}

#[test]
fn mirrored_symbol_is_reflected_across_the_bl_diagonal() {
    for size in [
        datamatrix::SymbolSize::Square20,
        datamatrix::SymbolSize::Rect12x26,
    ] {
        let m = encode_size(b"mirror me", size);
        let (bin, truth) = scene(&m, 5.0, 25.0, true, 0, false);
        let mut d = Detector::default();
        d.find(&bin, false, 0, 4, 128, 32);
        let cand = closest(&d, &truth).unwrap();
        let plane = Plane::new(&bin, false);
        let mut sc = SampleScratch::default();
        let layout = lay(plane, &cand.quad, &mut sc).unwrap();
        let mut hyps = [None; 3];
        snap_layout(&layout, &sc, &mut hyps, false);
        let hyp = hyps[0].unwrap();
        let (r, c) = hyp.grid_dims();
        let h = refine(plane, &layout.corners, &hyp, &mut sc).unwrap();
        let mut g = BitMatrix::default();
        assert!(sample_grid(plane, &h, r, c, &mut g));
        assert_eq!(hyp.swapped, m.rows != m.cols);
        // g(row, col) = source(rows - 1 - col, cols - 1 - row)
        let mut same = 0;
        for y in 0..r {
            for x in 0..c {
                same += usize::from(g.get(x, y) == m.get(m.cols - 1 - y, m.rows - 1 - x));
            }
        }
        assert!(
            same as f32 / (r * c) as f32 >= 0.99,
            "{}x{}",
            m.rows,
            m.cols
        );
    }
}

#[test]
fn perspective_grid_and_blur() {
    let m = encode_size(b"perspective and blur", datamatrix::SymbolSize::Square26);
    let quad = [
        Point::new(40.0, 30.0),
        Point::new(250.0, 55.0),
        Point::new(230.0, 215.0),
        Point::new(30.0, 205.0),
    ];
    for blur in [0, 1] {
        let r = Render {
            w: 290,
            h: 250,
            quad,
            mirrored: false,
            blur,
            invert: false,
        };
        let bin = threshold(&r.draw(&m), r.w, r.h);
        let o = run(&bin, false, &quad, m.rows, m.cols);
        let a = agreement(&o.grid, &m);
        assert!(a >= 0.99, "blur {blur}: {a:.3}");
    }
}

#[test]
fn mapper_fit_recovers_homography() {
    let src = [
        (0.0, 0.0),
        (10.0, 0.0),
        (10.0, 10.0),
        (0.0, 10.0),
        (5.0, 5.0),
        (2.0, 7.0),
    ];
    let target = |u: f64, v: f64| {
        let w = 1.0 + 0.01 * u + 0.005 * v;
        (
            (30.0 + 8.0 * u + 1.0 * v) / w,
            (20.0 - 0.5 * u + 9.0 * v) / w,
        )
    };
    let pairs: Vec<_> = src
        .iter()
        .map(|&(u, v)| {
            let (x, y) = target(u, v);
            (u, v, x, y, 1.0)
        })
        .collect();
    let h = Mapper::fit(&pairs).unwrap();
    let (x, y) = h.map(3.0, 4.0).unwrap();
    let (ex, ey) = target(3.0, 4.0);
    assert!((x - ex).abs() < 1e-6 && (y - ey).abs() < 1e-6);
}

/// Every stage (candidate, layout, size, sampled grid) succeeds on clean
/// renderings of every size, angle and module size from 3 px; the longest
/// DMRE rectangles at 3 px (8 or 12 rows of 3 px) are the known exception.
#[test]
fn clean_sweep_succeeds_at_every_stage() {
    let mut fails: Vec<String> = Vec::new();
    for size in all_sizes() {
        let m = encode_size(b"sweep", size);
        for deg in [0.0, 45.0, 133.0, 275.0] {
            for px in [3.0, 7.0] {
                if (m.cols as f32 * px) > 700.0 || (m.rows <= 12 && m.cols >= 88 && px < 4.0) {
                    continue;
                }
                let tag = format!("{}x{} deg {deg} px {px}", m.rows, m.cols);
                let (bin, truth) = scene(&m, px, deg, false, 0, false);
                let mut d = Detector::default();
                d.find(&bin, false, 0, 4, 128, 32);
                let Some(cand) = closest(&d, &truth) else {
                    fails.push(format!("{tag}: no candidate"));
                    continue;
                };
                if quad_err(&cand.quad, &truth) > 1.2 * px + 1.5 {
                    fails.push(format!("{tag}: quad err {}", quad_err(&cand.quad, &truth)));
                    continue;
                }
                let plane = Plane::new(&bin, false);
                let mut sc = SampleScratch::default();
                let Some(layout) = lay(plane, &cand.quad, &mut sc) else {
                    fails.push(format!("{tag}: no layout"));
                    continue;
                };
                let mut hyps = [None; 3];
                snap_layout(&layout, &sc, &mut hyps, false);
                let Some(hyp) = hyps
                    .iter()
                    .flatten()
                    .find(|h| h.grid_dims() == (m.rows, m.cols))
                    .copied()
                else {
                    fails.push(format!(
                        "{tag}: size miss (counted {}x{})",
                        layout.rows_est, layout.cols_est
                    ));
                    continue;
                };
                let h = refine(plane, &layout.corners, &hyp, &mut sc).unwrap();
                let mut grid = BitMatrix::default();
                if !sample_grid(plane, &h, m.rows, m.cols, &mut grid) {
                    fails.push(format!("{tag}: sampling"));
                    continue;
                }
                let a = agreement(&grid, &m);
                if a < 0.99 {
                    fails.push(format!("{tag}: agreement {a:.3}"));
                }
            }
        }
    }
    assert!(fails.is_empty(), "{fails:#?}");
}

/// `n` alternating modules starting dark, `per` samples each.
fn alternating(n: usize, per: usize) -> Vec<bool> {
    (0..n * per).map(|i| (i / per).is_multiple_of(2)).collect()
}

/// A size is hypothesised from one clock track alone when the other track
/// is lost (blur or perspective at its far end): `dm-2.jpg` of the zxing
/// samples is a 22x22 symbol whose right track agrees at 0.84 with 22
/// modules and whose top track agrees with nothing.
#[test]
fn one_strong_clock_track_gives_the_size_when_weak_hypotheses_are_on() {
    let mut sc = SampleScratch {
        right_samples: alternating(22, 6),
        // A top track that matches no module count.
        top_samples: (0..132).map(|i| (i * 7 / 5) % 3 == 0).collect(),
        ..SampleScratch::default()
    };
    let mut strict = [None; 4];
    snap_tracks(&sc, false, &mut strict);
    assert!(strict.iter().all(Option::is_none));
    let mut weak = [None; 4];
    snap_tracks(&sc, true, &mut weak);
    let first = weak[0].expect("a hypothesis from the strong track");
    assert_eq!(first.grid_dims(), (22, 22), "squares come first");
    // Both tracks in agreement keep their hypotheses and their order.
    sc.top_samples = alternating(22, 6);
    let mut both = [None; 4];
    snap_tracks(&sc, true, &mut both);
    assert_eq!(both[0].unwrap().grid_dims(), (22, 22));
    let mut both_strict = [None; 4];
    snap_tracks(&sc, false, &mut both_strict);
    assert_eq!(both_strict[0].unwrap().grid_dims(), (22, 22));
}

/// The size is found by the corner search when the counted clock runs are
/// wrong and the quad is off at the far corners (`4902777015927`, a 16x16
/// symbol whose clock runs were counted as 10 and 14).
#[test]
fn corner_search_ranks_the_true_size_first_on_a_perspective_symbol() {
    let m = encode_size(b"corner search", datamatrix::SymbolSize::Square16);
    let quad = [
        Point::new(40.0, 30.0),
        Point::new(130.0, 44.0),
        Point::new(122.0, 118.0),
        Point::new(34.0, 104.0),
    ];
    let r = Render {
        w: 170,
        h: 150,
        quad,
        mirrored: false,
        blur: 1,
        invert: false,
    };
    let bin = threshold(&r.draw(&m), r.w, r.h);
    let plane = Plane::new(&bin, false);
    let mut sc = SampleScratch::default();
    let mut layout = lay(plane, &quad, &mut sc).expect("layout");
    // Quad corners moved by up to a module and a half: a poor detection.
    layout.corners[1].x -= 7.0;
    layout.corners[2].y += 6.0;
    layout.corners[0].x += 4.0;
    // Counts a quarter off.
    layout.rows_est = 12;
    layout.cols_est = 12;
    let mut ranked = Vec::new();
    broad_hypotheses(plane, &layout, &mut sc, &mut ranked);
    let (share, hyp, corners) = ranked.first().expect("a size within the search");
    assert_eq!(hyp.grid_dims(), (16, 16));
    assert!(*share <= 0.2, "{share}");
    let h = refine(plane, corners, hyp, &mut sc).expect("mapper");
    let mut grid = BitMatrix::default();
    assert!(sample_grid(plane, &h, 16, 16, &mut grid));
    assert!(agreement(&grid, &m) >= 0.97);
}

/// Below two pixels per module the geometry searches do not run: any
/// pattern fits a track read from one or two pixels per module.
#[test]
fn corner_search_refuses_modules_under_two_pixels() {
    let m = encode_size(b"tiny", datamatrix::SymbolSize::Square16);
    let (bin, truth) = scene(&m, 1.6, 0.0, false, 0, false);
    let plane = Plane::new(&bin, false);
    let mut sc = SampleScratch::default();
    let hyp = Hypothesis {
        size: crate::spec::size(16, 16).unwrap(),
        swapped: false,
    };
    assert!(corner_search(plane, &truth, &hyp, &mut sc).is_none());
    let (bin, truth) = scene(&m, 3.0, 0.0, false, 0, false);
    let plane = Plane::new(&bin, false);
    let (found, cost) = corner_search(plane, &truth, &hyp, &mut sc).expect("3 px per module");
    assert_eq!(cost, 0);
    assert!(quad_err(&found, &truth) < 1.5);
}

/// The retries that search the geometry never read modules outside the
/// image: a symbol cut by the border of the frame (`case-208` of the
/// skewed-datamatrix set, unlabelled because of it) is read from guesses.
#[test]
fn strict_sampling_refuses_modules_outside_the_image() {
    let m = encode_size(b"cut by the frame", datamatrix::SymbolSize::Square20);
    let px = 6.0;
    let center = Point::new(60.0, 70.0);
    let quad = similarity_quad(m.rows, m.cols, center, px, 0.0);
    let r = Render {
        w: 130,
        h: 140,
        quad,
        mirrored: false,
        blur: 0,
        invert: false,
    };
    // The symbol spans x = 0..120; the image ends at 111 (1.5 modules inside it).
    let luma = r.draw(&m);
    let (w, h) = (111, r.h);
    let cropped: Vec<u8> = (0..h)
        .flat_map(|y| luma[y * r.w..y * r.w + w].to_vec())
        .collect();
    let bin = threshold(&cropped, w, h);
    let plane = Plane::new(&bin, false);
    let mapper = Mapper::from_corners(&quad, m.rows, m.cols).unwrap();
    let mut grid = BitMatrix::default();
    // 2 of 20 columns are outside: 10 % of the modules, within the plain limit.
    assert!(sample_grid_warped(
        plane, &mapper, None, m.rows, m.cols, &mut grid, false
    ));
    assert!(!sample_grid_warped(
        plane, &mapper, None, m.rows, m.cols, &mut grid, true
    ));
}

/// A compressed axis: every boundary of the top track is found where the
/// clock shows it (`distorted.png`: modules 4 px wide on the left, 2 px on
/// the right).
#[test]
fn track_warp_places_the_boundaries_of_a_compressed_axis() {
    let m = encode_size(b"compressed", datamatrix::SymbolSize::Square16);
    let px = 6.0;
    let side = ((m.cols as f32 + 8.0) * px) as usize;
    let quad = similarity_quad(
        m.rows,
        m.cols,
        Point::new(side as f32 / 2.0, side as f32 / 2.0),
        px,
        0.0,
    );
    let cols = m.cols as f32;
    let r = Render {
        w: side,
        h: side,
        quad,
        mirrored: false,
        blur: 0,
        invert: false,
    };
    // The symbol's module u = cols * (x / cols)^0.8 is drawn at the position x
    // of the quad: the modules are narrow on the left and 1.8 times wider on
    // the right, and the homography is up to 1.2 modules off in the middle.
    let luma = r.draw_with(&m, |u, v| (cols * (u / cols).powf(0.8), v));
    let bin = threshold(&luma, side, side);
    let plane = Plane::new(&bin, false);
    let hyp = Hypothesis {
        size: crate::spec::size(16, 16).unwrap(),
        swapped: false,
    };
    let mapper = Mapper::from_corners(&quad, m.rows, m.cols).unwrap();
    let mut plain = BitMatrix::default();
    assert!(sample_grid(plane, &mapper, 16, 16, &mut plain));
    let plain_agreement = agreement(&plain, &m);
    let warp = track_warp(plane, &mapper, &hyp, &mut SampleScratch::default())
        .expect("both tracks count 16 modules");
    let mut warped = BitMatrix::default();
    assert!(sample_grid_warped(
        plane,
        &mapper,
        Some(&warp),
        16,
        16,
        &mut warped,
        true
    ));
    let warped_agreement = agreement(&warped, &m);
    assert!(plain_agreement < 0.9, "{plain_agreement}");
    assert!(warped_agreement >= 0.97, "{warped_agreement}");
}
