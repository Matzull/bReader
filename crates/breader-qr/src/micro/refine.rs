//! Refinement of the grid corners by sampling sharpness.

use breader_core::{Perspective, Plane, Point};

use crate::sample::Mapper;

/// Sub-sample offsets (in modules) around a module center.
const OFFSETS: [(f32, f32); 5] = [
    (0.0, 0.0),
    (-0.22, -0.22),
    (0.22, -0.22),
    (-0.22, 0.22),
    (0.22, 0.22),
];

/// Expected value of a module that belongs to the finder, separator or
/// timing patterns.
fn expected(x: usize, y: usize) -> Option<bool> {
    if x < 8 && y < 8 {
        Some(x < 7 && y < 7 && (x as i32 - 3).abs().max((y as i32 - 3).abs()) != 2)
    } else if y == 0 || x == 0 {
        Some((x + y).is_multiple_of(2))
    } else {
        None
    }
}

/// How well the mapping places module centers: unanimous sub-samples score
/// high, and the known patterns must come out right.
fn score(bin: Plane<'_>, h: &Perspective, d: usize) -> Option<i32> {
    let mut total = 0;
    for y in 0..d {
        for x in 0..d {
            let mut votes = 0;
            let mut center = false;
            for (i, (ox, oy)) in OFFSETS.iter().enumerate() {
                let p = h.map(Point::new(x as f32 + 0.5 + ox, y as f32 + 0.5 + oy))?;
                let v = p.x >= 0.0
                    && p.y >= 0.0
                    && (p.x as usize) < bin.width()
                    && (p.y as usize) < bin.height()
                    && bin.get(p.x as usize, p.y as usize);
                votes += i32::from(v);
                if i == 0 {
                    center = v;
                }
            }
            total += votes.max(5 - votes);
            if expected(x, y) == Some(center) {
                total += 3;
            }
        }
    }
    Some(total)
}

fn mapper(corners: &[Point; 4], d: usize) -> Option<Perspective> {
    let df = d as f32;
    Perspective::quad_to_quad(
        [
            Point::new(0.0, 0.0),
            Point::new(df, 0.0),
            Point::new(df, df),
            Point::new(0.0, df),
        ],
        *corners,
    )
}

/// Moves the four grid corners to maximize [`score`]; `None` when the
/// mapping has no valid start.
pub(super) fn refine_corners(bin: Plane<'_>, start: &Mapper, d: usize) -> Option<Mapper> {
    let df = d as f32;
    let mut c = [
        start.map(Point::new(0.0, 0.0))?,
        start.map(Point::new(df, 0.0))?,
        start.map(Point::new(df, df))?,
        start.map(Point::new(0.0, df))?,
    ];
    let px = (c[0].distance(c[1]) + c[0].distance(c[3])) / (2.0 * df);
    let mut best = score(bin, &mapper(&c, d)?, d)?;
    for step in [0.3f32, 0.15, 0.07] {
        for _ in 0..3 {
            let mut improved = false;
            for i in 0..4 {
                for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    let mut t = c;
                    t[i] = Point::new(t[i].x + dx * step * px, t[i].y + dy * step * px);
                    let Some(h) = mapper(&t, d) else {
                        continue;
                    };
                    if let Some(s) = score(bin, &h, d)
                        && s > best
                    {
                        best = s;
                        c = t;
                        improved = true;
                    }
                }
            }
            if !improved {
                break;
            }
        }
    }
    mapper(&c, d).map(Mapper::Corners)
}
