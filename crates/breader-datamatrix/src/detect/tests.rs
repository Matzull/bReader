use breader_core::{BitMatrix, Point};

use super::*;
use crate::testutil::*;

/// Largest corner distance after the best cyclic alignment.
pub(crate) fn quad_error(a: &[Point; 4], b: &[Point; 4]) -> f32 {
    (0..4)
        .map(|s| {
            (0..4)
                .map(|i| a[i].distance(b[(i + s) % 4]))
                .fold(0.0, f32::max)
        })
        .fold(f32::MAX, f32::min)
}

fn best_candidate(bin: &BitMatrix, inverted: bool, truth: &[Point; 4]) -> Option<(f32, usize)> {
    let mut d = Detector::default();
    d.find(bin, inverted, 3, 128, 32);
    d.candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (quad_error(&c.quad, truth), i))
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

#[test]
fn inverted_symbols_are_found_in_the_inverted_plane() {
    let m = encode_size(b"inverted", datamatrix::SymbolSize::Square20);
    let (mut bin, truth) = scene(&m, 4.0, 30.0, false, 0, false);
    bin.invert();
    assert!(best_candidate(&bin, true, &truth).is_some_and(|(e, _)| e < 5.0));
}

#[test]
fn perspective_and_blur() {
    let m = encode_size(b"perspective", datamatrix::SymbolSize::Square24);
    let quad = [
        Point::new(40.0, 30.0),
        Point::new(190.0, 50.0),
        Point::new(175.0, 175.0),
        Point::new(30.0, 160.0),
    ];
    let r = Render {
        w: 220,
        h: 200,
        quad,
        mirrored: false,
        blur: 1,
        invert: false,
    };
    let bin = threshold(&r.draw(&m), r.w, r.h);
    let (err, _) = best_candidate(&bin, false, &quad).expect("candidate");
    assert!(err < 6.0, "error {err}");
}

#[test]
fn no_panic_and_bounded_on_junk() {
    let mut seed = 0x1234_5678_9abc_def0u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let (w, h) = (320usize, 240usize);
    let mut images: Vec<BitMatrix> = Vec::new();
    let mut noise = BitMatrix::new(w, h);
    for y in 0..h {
        for x in 0..w {
            noise.set(x, y, next() & 1 == 1);
        }
    }
    images.push(noise);
    for period in [1usize, 2, 3, 5] {
        let mut stripes = BitMatrix::new(w, h);
        let mut checker = BitMatrix::new(w, h);
        for y in 0..h {
            for x in 0..w {
                stripes.set(x, y, (x / period) % 2 == 0);
                checker.set(x, y, (x / period + y / period) % 2 == 0);
            }
        }
        images.push(stripes);
        images.push(checker);
    }
    let mut full = BitMatrix::new(w, h);
    full.invert();
    images.push(full);
    images.push(BitMatrix::new(w, h));
    let mut d = Detector::default();
    for img in &images {
        for inverted in [false, true] {
            let t = std::time::Instant::now();
            d.find(img, inverted, 4, 128, 32);
            assert!(t.elapsed().as_millis() < 500);
            assert!(d.candidates.len() <= 32);
        }
    }
}

#[test]
fn block_counts_match_naive() {
    let (w, h) = (131usize, 37usize);
    let mut seed = 99u64;
    let mut raw = BitMatrix::new(w, h);
    for y in 0..h {
        for x in 0..w {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            raw.set(x, y, (seed >> 33).is_multiple_of(3));
        }
    }
    for inverted in [false, true] {
        let mut cur = Counts::default();
        Counts::from_blocks4(&raw, inverted, &mut cur);
        let mut next = Counts::default();
        let mut f = 4;
        for level in 0..4 {
            if level > 0 {
                cur.halve(&mut next);
                std::mem::swap(&mut cur, &mut next);
                f *= 2;
            }
            assert_eq!(cur.width, w.div_ceil(f));
            assert_eq!(cur.height, h.div_ceil(f));
            for cy in 0..cur.height {
                for cx in 0..cur.width {
                    let mut n = 0u16;
                    for y in cy * f..((cy + 1) * f).min(h) {
                        for x in cx * f..((cx + 1) * f).min(w) {
                            n += u16::from(raw.get(x, y) != inverted);
                        }
                    }
                    assert_eq!(cur.row(cy)[cx], n, "f {f} cell {cx},{cy} inv {inverted}");
                }
            }
            for min in [1u16, 5, 40] {
                let mut bits = BitMatrix::default();
                cur.threshold(min, &mut bits);
                for cy in 0..cur.height {
                    for cx in 0..cur.width {
                        assert_eq!(bits.get(cx, cy), cur.row(cy)[cx] >= min);
                    }
                }
            }
        }
    }
}

#[test]
fn row_hull_matches_sorted_hull() {
    let mut seed = 7u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for _ in 0..200 {
        let n = 2 + (next() % 40) as usize;
        let y0 = (next() % 50) as i32;
        let rows: Vec<(i32, i32)> = (0..n)
            .map(|_| {
                if next() % 6 == 0 {
                    (i32::MAX, -1)
                } else {
                    let a = (next() % 60) as i32;
                    (a, a + (next() % 30) as i32)
                }
            })
            .collect();
        if rows.iter().filter(|e| e.1 >= 0).count() < 2 {
            continue;
        }
        let mut full = Vec::new();
        for (k, &(a, b)) in rows.iter().enumerate() {
            if b < 0 {
                continue;
            }
            let y = y0 + k as i32;
            full.extend([(a, y), (a, y + 1), (b + 1, y), (b + 1, y + 1)]);
        }
        let mut expect = Vec::new();
        convex_hull(&mut full, &mut expect);
        let mut pts = Vec::new();
        row_extreme_points(&rows, y0, &mut pts);
        let mut got = Vec::new();
        convex_hull_by_rows(&pts, &mut got);
        assert_eq!(got, expect);
    }
}

#[test]
fn morphology_matches_naive() {
    let (w, h) = (150usize, 40usize);
    let mut seed = 5u64;
    let mut src = BitMatrix::new(w, h);
    for y in 0..h {
        for x in 0..w {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            src.set(x, y, !(seed >> 33).is_multiple_of(5));
        }
    }
    for r in [1usize, 2, 3] {
        for and in [true, false] {
            let (mut out, mut tmp, mut hr) =
                (BitMatrix::default(), Vec::new(), BitMatrix::default());
            morph(&src, r, and, &mut out, &mut tmp, &mut hr);
            for y in 0..h {
                for x in 0..w {
                    let mut all = true;
                    let mut any = false;
                    for dy in -(r as i32)..=r as i32 {
                        for dx in -(r as i32)..=r as i32 {
                            let (xx, yy) = (x as i32 + dx, y as i32 + dy);
                            let v = xx >= 0
                                && yy >= 0
                                && (xx as usize) < w
                                && (yy as usize) < h
                                && src.get(xx as usize, yy as usize);
                            all &= v;
                            any |= v;
                        }
                    }
                    assert_eq!(
                        out.get(x, y),
                        if and { all } else { any },
                        "r {r} and {and} at {x},{y}"
                    );
                }
            }
        }
    }
}
