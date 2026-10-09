use super::*;
use crate::decode::{Shape, probe};
use crate::spec::Layout;
use crate::testutil::*;

fn plane_of(luma: &[u8], side: usize) -> BitMatrix {
    binarize(luma, side, side)
}

/// Largest distance, in modules, between `map` and the truth over the
/// modules of a `size` x `size` symbol.
fn max_error(map: impl Fn(Point) -> Option<Point>, size: usize, quad: [Point; 4], px: f32) -> f32 {
    let truth = truth_map(size, quad);
    let c = (size / 2) as i32;
    let mut worst = 0.0f32;
    for dy in -c..=c {
        for dx in -c..=c {
            let o = Point::new(dx as f32, dy as f32);
            let got = map(o).expect("maps");
            worst = worst.max(got.distance(truth(o)) / px);
        }
    }
    worst
}

/// Fits the core of the symbol of `n` modules drawn through `quad`, starting
/// from its true center.
fn fit_at(
    plane: Plane<'_>,
    quad: [Point; 4],
    n: usize,
    module: f32,
    rings: u8,
    skip: f32,
) -> CoreFit {
    let center = truth_map(n, quad)(Point::new(0.0, 0.0));
    let mut crossings = Vec::new();
    cast_rays(plane, center, module, rings, &mut crossings);
    fit_core(center, &crossings, module, rings, skip).expect("a fit")
}

#[test]
fn rays_count_the_rings_of_the_bullseye() {
    let side_of = |layers| {
        let (_, r) = rxing_fill(2, 30, layers).unwrap();
        let (luma, side, _) = scene(&r.grid, 6.0, 0.0, 0.0, |_| {});
        (plane_of(&luma, side), side)
    };
    let center = |side: usize| Point::new(side as f32 / 2.0, side as f32 / 2.0);
    // Full-range: boundaries 2 to 6 on (almost) every ray; compact: 2 to 4
    // (the rays then run into the mode message and data, whose crossings the
    // fit has to discard).
    let (m, side) = side_of(4);
    let mut crossings = Vec::new();
    cast_rays(Plane::new(&m, false), center(side), 6.0, 6, &mut crossings);
    let count = |j| crossings.iter().filter(|c| c.j == j).count();
    assert!(count(6) >= 28, "{}", count(6));
    assert!(count(2) >= 30 && count(2) <= 32);
    let (m, side) = side_of(-2);
    cast_rays(Plane::new(&m, false), center(side), 6.0, 6, &mut crossings);
    let count = |j| crossings.iter().filter(|c| c.j == j).count();
    assert!(count(4) >= 28, "{}", count(4));
}

#[test]
fn a_ray_fit_needs_a_bullseye() {
    // Concentric circles are not square rings.
    let side = 200usize;
    let mut m = BitMatrix::new(side, side);
    for y in 0..side {
        for x in 0..side {
            let r = ((x as f32 - 100.0).hypot(y as f32 - 100.0) / 6.0).floor() as i32;
            if r % 2 == 0 && r < 8 {
                m.set(x, y, true);
            }
        }
    }
    let plane = Plane::new(&m, false);
    let center = Point::new(100.5, 100.5);
    let mut crossings = Vec::new();
    cast_rays(plane, center, 6.0, 4, &mut crossings);
    assert!(fit_core(center, &crossings, 6.0, 4, 2.0).is_none());
}

#[test]
fn core_fit_follows_rotation_and_perspective() {
    let (_, r) = rxing_fill(6, 30, -3).unwrap();
    for (deg, tilt) in [
        (0.0, 0.0),
        (23.0, 0.0),
        (90.0, 0.0),
        (200.0, 0.0),
        (10.0, 0.25),
    ] {
        let (luma, side, quad) = scene(&r.grid, 6.0, deg, tilt, |_| {});
        let m = plane_of(&luma, side);
        let fit = fit_at(Plane::new(&m, false), quad, r.grid.width(), 6.0, 4, 2.0);
        let truth = truth_map(r.grid.width(), quad);
        // The fit may be a quarter turn off: the rings are symmetric. Its
        // corners of the mode message ring are the true ones in some order.
        for (u, v) in [
            (-5.0, -5.0),
            (5.0, -5.0),
            (5.0, 5.0),
            (-5.0, 5.0),
            (0.0, 0.0),
        ] {
            let o = Point::new(u, v);
            let got = fit.h.map(o).unwrap();
            let best = (0..4)
                .map(|k| truth(unorient(o, k, false)).distance(got))
                .fold(f32::MAX, f32::min);
            assert!(best < 0.35 * 6.0, "{deg} {tilt} {o:?}: {got:?}");
        }
    }
}

#[test]
fn the_orientation_helpers_agree_and_the_probe_finds_each_orientation() {
    let (_, r) = rxing_fill(9, 20, -2).unwrap();
    let n = r.grid.width();
    let c = (n / 2) as i32;
    let center_crop = |g: &BitMatrix| {
        let mut core = BitMatrix::new(CORE_SIZE, CORE_SIZE);
        for y in 0..CORE_SIZE {
            for x in 0..CORE_SIZE {
                let (gx, gy) = (c as usize - 7 + x, c as usize - 7 + y);
                if g.get(gx, gy) {
                    core.set(x, y, true);
                }
            }
        }
        core
    };
    let regular = center_crop(&r.grid);
    // `orient` and `unorient` describe the same transformation.
    for mirror in [false, true] {
        for rot in 0..4 {
            let o = orient(&regular, rot, mirror);
            for dy in -7..=7i32 {
                for dx in -7..=7i32 {
                    let p = unorient(Point::new(dx as f32, dy as f32), rot, mirror);
                    let (sx, sy) = ((7 + p.x as i32) as usize, (7 + p.y as i32) as usize);
                    assert_eq!(
                        o.get((7 + dx) as usize, (7 + dy) as usize),
                        regular.get(sx, sy)
                    );
                }
            }
        }
    }
    // A symbol seen in any of the eight orientations probes as a regular one
    // in exactly the orientation that undoes it.
    let layout = Layout {
        compact: true,
        layers: 2,
    };
    for mirror in [false, true] {
        for rot in 0..4 {
            let seen = orient(&regular, rot, mirror);
            let mut found = Vec::new();
            for m2 in [false, true] {
                for r2 in 0..4 {
                    if probe(&orient(&seen, r2, m2)).contains(&Shape::Layout(layout)) {
                        found.push((r2, m2));
                    }
                }
            }
            assert_eq!(found.len(), 1, "{rot} {mirror}: {found:?}");
            let back = orient(&seen, found[0].0, found[0].1);
            assert_eq!(back, regular, "{rot} {mirror}");
        }
    }
}

#[test]
fn bullseye_match_tells_compact_from_full() {
    let core_of = |layers| {
        let (_, r) = rxing_fill(4, 20, layers).unwrap();
        let c = r.grid.width() / 2;
        let mut core = BitMatrix::new(CORE_SIZE, CORE_SIZE);
        for y in 0..CORE_SIZE {
            for x in 0..CORE_SIZE {
                if r.grid.get(c - 7 + x, c - 7 + y) {
                    core.set(x, y, true);
                }
            }
        }
        core
    };
    let full = core_of(3);
    assert!(bullseye_match(&full, true) > 0.99);
    let compact = core_of(-3);
    assert!(bullseye_match(&compact, false) > 0.99);
    assert!(bullseye_match(&compact, true) < 0.9);
}

#[test]
fn sampling_a_clean_symbol_reproduces_its_modules() {
    let (_, r) = rxing_fill(3, 40, 5).unwrap();
    let (luma, side, quad) = scene(&r.grid, 5.0, 0.0, 0.0, |_| {});
    let m = plane_of(&luma, side);
    let plane = Plane::new(&m, false);
    let fit = fit_at(plane, quad, r.grid.width(), 5.0, 6, 2.0);
    let size = r.grid.width();
    let mapper = anchor_grid(plane, &fit, size);
    assert!(max_error(|p| mapper.map(p), size, quad, 5.0) < 0.2);
    let mut grid = BitMatrix::new(1, 1);
    assert!(sample_grid(plane, &mapper, size, &mut grid));
    assert_eq!(grid, r.grid);
}

#[test]
fn reference_grid_anchoring_beats_the_core_fit_on_a_large_tilted_symbol() {
    let (_, r) = rxing_fill(12, 600, 18).unwrap();
    let size = r.grid.width();
    assert!(size > 80);
    let (luma, side, quad) = scene(&r.grid, 4.0, 12.0, 0.2, |_| {});
    let m = plane_of(&luma, side);
    let plane = Plane::new(&m, false);
    let fit = fit_at(plane, quad, size, 4.0, 6, 2.0);
    let global = Mapper::Global(fit.h);
    let anchored = anchor_grid(plane, &fit, size);
    assert!(matches!(anchored, Mapper::Warped(_)));
    let e_global = max_error(|p| global.map(p), size, quad, 4.0);
    let e_anchored = max_error(|p| anchored.map(p), size, quad, 4.0);
    assert!(e_anchored < 1.2, "{e_anchored}");
    assert!(e_anchored * 3.0 < e_global, "{e_anchored} vs {e_global}");
    // And the sampled symbol decodes.
    let mut grid = BitMatrix::new(1, 1);
    assert!(sample_grid(plane, &anchored, size, &mut grid));
    let wrong = (0..size)
        .flat_map(|y| (0..size).map(move |x| (x, y)))
        .filter(|&(x, y)| grid.get(x, y) != r.grid.get(x, y))
        .count();
    assert!(wrong * 100 < size * size, "{wrong} wrong modules");
}

#[test]
fn reference_grid_check_catches_a_misplaced_symbol() {
    let (_, r) = rxing_fill(5, 60, 6).unwrap();
    let layout = r.layout;
    assert!(grid_lines_plausible(&r.grid, layout));
    // The same modules one module to the right and down: the lines no longer
    // alternate where they should.
    let n = r.grid.width();
    let mut shifted = BitMatrix::new(n, n);
    for y in 0..n - 1 {
        for x in 0..n - 1 {
            if r.grid.get(x, y) {
                shifted.set(x + 1, y + 1, true);
            }
        }
    }
    assert!(!grid_lines_plausible(&shifted, layout));
    let (_, c) = rxing_fill(5, 8, -2).unwrap();
    assert!(grid_lines_plausible(&c.grid, c.layout));
}

#[test]
fn outside_modules_read_as_light_and_are_counted() {
    let m = BitMatrix::new(20, 20);
    let plane = Plane::new(&m, true);
    // The whole symbol is in the image: nothing outside.
    let h = Homography::fit(
        &[(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
            .map(|(u, v)| (Point::new(u, v), Point::new(10.0 + u * 3.0, 10.0 + v * 3.0))),
    )
    .unwrap();
    let mut out = BitMatrix::new(1, 1);
    assert!(sample_grid(plane, &Mapper::Global(h), 5, &mut out));
    // A 9 x 9 grid reaches past the 20 x 20 image.
    assert!(!sample_grid(plane, &Mapper::Global(h), 15, &mut out));
    assert!(sample_grid_within(
        plane,
        &Mapper::Global(h),
        15,
        &mut out,
        100
    ));
}
