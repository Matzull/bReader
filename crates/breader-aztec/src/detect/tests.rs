use super::*;
use crate::testutil::*;

fn search(luma: &[u8], side: usize, both: bool) -> Vec<Candidate> {
    let m = binarize(luma, side, side);
    let mut s = BullseyeSearch::default();
    s.find(&m, 2, both);
    s.candidates
}

#[test]
fn ratio_accepts_equal_runs() {
    assert!(bullseye_ratio(&[5; 7], ROW_TOL_PERCENT));
    assert!(bullseye_ratio(&[4, 5, 6, 5, 4, 6, 5], ROW_TOL_PERCENT));
    assert!(bullseye_ratio(
        &[20, 22, 19, 21, 20, 18, 22],
        ROW_TOL_PERCENT
    ));
}

#[test]
fn ratio_rejects_uneven_runs() {
    // A QR finder row (1:1:3:1:1) padded with its neighbours, a stripe, a
    // run twice as wide as the others, an empty window.
    assert!(!bullseye_ratio(&[5, 5, 5, 15, 5, 5, 5], ROW_TOL_PERCENT));
    assert!(!bullseye_ratio(&[4, 4, 4, 9, 4, 4, 4], ROW_TOL_PERCENT));
    assert!(!bullseye_ratio(&[6, 6, 6, 6, 6, 6, 12], ROW_TOL_PERCENT));
    assert!(!bullseye_ratio(&[1, 1, 1, 30, 1, 1, 1], ROW_TOL_PERCENT));
    assert!(!bullseye_ratio(&[0; 7], ROW_TOL_PERCENT));
}

#[test]
fn ratio_tolerates_a_pixel_of_jitter_on_narrow_runs() {
    assert!(bullseye_ratio(&[2, 3, 2, 3, 2, 2, 2], ROW_TOL_PERCENT));
    assert!(!bullseye_ratio(&[2, 7, 2, 3, 2, 1, 2], ROW_TOL_PERCENT));
}

#[test]
fn finds_compact_and_full_bullseyes() {
    for layers in [-1, -3, 2, 6] {
        let (_, r) = rxing_fill(5, 12, layers).unwrap();
        let (luma, side, _) = scene(&r.grid, 4.0, 0.0, 0.0, |_| {});
        let found = search(&luma, side, false);
        let c = found.first().expect("a candidate");
        let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
        assert!(c.pos.distance(center) < 1.5, "{layers}: {:?}", c.pos);
        assert!((c.module - 4.0).abs() < 1.0, "{layers}: {}", c.module);
        assert!(!c.inv);
    }
}

#[test]
fn finds_rotated_bullseyes() {
    let (_, r) = rxing_fill(8, 20, -2).unwrap();
    for deg in [13.0, 30.0, 45.0, 77.0, 135.0, 200.0] {
        let (luma, side, _) = scene(&r.grid, 5.0, deg, 0.0, |_| {});
        let found = search(&luma, side, false);
        let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
        assert!(
            found.iter().any(|c| c.pos.distance(center) < 2.0),
            "{deg}: {found:?}"
        );
    }
}

#[test]
fn finds_a_light_on_dark_bullseye_only_when_asked() {
    let (_, r) = rxing_fill(3, 10, -1).unwrap();
    let (luma, side, _) = scene(&r.grid, 4.0, 0.0, 0.0, |r| r.invert = true);
    assert!(search(&luma, side, false).iter().all(|c| {
        c.pos
            .distance(Point::new(side as f32 / 2.0, side as f32 / 2.0))
            > 5.0
    }));
    let found = search(&luma, side, true);
    let c = found.first().expect("a candidate");
    assert!(c.inv);
}

#[test]
fn survives_blur_and_noise() {
    let (_, r) = rxing_fill(4, 20, 3).unwrap();
    let (luma, side, _) = scene(&r.grid, 4.0, 10.0, 0.0, |r| {
        r.blur = 1;
        r.noise = 20;
    });
    let found = search(&luma, side, false);
    let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
    assert!(found.iter().any(|c| c.pos.distance(center) < 2.0));
}

#[test]
fn stripes_and_checkers_give_no_candidate_and_noise_few() {
    let side = 240;
    let stripes: Vec<u8> = (0..side * side)
        .map(|i| if (i % side / 3) % 2 == 0 { 0 } else { 255 })
        .collect();
    let checker: Vec<u8> = (0..side * side)
        .map(|i| {
            if (i % side / 4 + i / side / 4) % 2 == 0 {
                0
            } else {
                255
            }
        })
        .collect();
    let mut rng = Rng(11);
    let noise: Vec<u8> = (0..side * side)
        .map(|_| if rng.below(2) == 1 { 255 } else { 0 })
        .collect();
    for (i, img) in [stripes, checker].iter().enumerate() {
        let f = search(img, side, true);
        assert!(f.is_empty(), "{i}: {f:?}");
    }
    // Random noise has bullseye-like runs here and there; they are few.
    assert!(search(&noise, side, true).len() < 40);
}

#[test]
fn a_stripe_repeated_on_every_row_is_confirmed_once() {
    // A barcode-like pattern of equal bars: many row hits, one confirm each
    // time the sequence is new.
    let (w, h) = (300usize, 200usize);
    let mut m = BitMatrix::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if (x / 3) % 2 == 0 {
                m.set(x, y, true);
            }
        }
    }
    let mut s = BullseyeSearch::default();
    s.find(&m, 1, true);
    assert!(s.candidates.is_empty());
    assert!(s.confirms < 4 * w, "{}", s.confirms);
}
