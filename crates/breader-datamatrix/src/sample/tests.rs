use breader_core::{BitMatrix, Point};

use super::*;
use crate::check;
use crate::detect::Detector;
use crate::plane::Plane;
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
    d.find(bin, inverted, 4, 128, 32);
    let cand = closest(&d, truth).expect("candidate");
    let plane = Plane::new(bin, inverted);
    let mut sc = SampleScratch::default();
    let layout =
        lay(plane, &cand.quad, &mut sc).unwrap_or_else(|| panic!("no layout for {rows}x{cols}"));
    let mut hyps = [None; 3];
    snap_layout(&layout, &sc, &mut hyps);
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
        d.find(&bin, false, 4, 128, 32);
        let cand = closest(&d, &truth).unwrap();
        let plane = Plane::new(&bin, false);
        let mut sc = SampleScratch::default();
        let layout = lay(plane, &cand.quad, &mut sc).unwrap();
        let mut hyps = [None; 3];
        snap_layout(&layout, &sc, &mut hyps);
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
        for deg in [0.0, 17.0, 45.0, 90.0, 133.0, 200.0, 275.0] {
            for px in [3.0, 4.5, 7.0] {
                if (m.cols as f32 * px) > 700.0 || (m.rows <= 12 && m.cols >= 88 && px < 4.0) {
                    continue;
                }
                let tag = format!("{}x{} deg {deg} px {px}", m.rows, m.cols);
                let (bin, truth) = scene(&m, px, deg, false, 0, false);
                let mut d = Detector::default();
                d.find(&bin, false, 4, 128, 32);
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
                snap_layout(&layout, &sc, &mut hyps);
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
