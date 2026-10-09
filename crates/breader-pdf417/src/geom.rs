//! Small vector helpers and the robust line fit used for symbol boundaries.

use breader_core::Point;

pub(crate) type Vec2 = (f32, f32);

pub(crate) fn sub(a: Point, b: Point) -> Vec2 {
    (a.x - b.x, a.y - b.y)
}

pub(crate) fn at(p: Point, d: Vec2, t: f32) -> Point {
    Point::new(p.x + d.0 * t, p.y + d.1 * t)
}

pub(crate) fn dot(a: Vec2, b: Vec2) -> f32 {
    a.0 * b.0 + a.1 * b.1
}

pub(crate) fn perp(d: Vec2) -> Vec2 {
    (-d.1, d.0)
}

pub(crate) fn unit(v: Vec2) -> Option<Vec2> {
    let n = v.0.hypot(v.1);
    (n > 1e-6 && n.is_finite()).then(|| (v.0 / n, v.1 / n))
}

/// A line through `origin` with unit direction `dir`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Line {
    pub origin: Point,
    pub dir: Vec2,
}

impl Line {
    /// Signed distance of `p` from the line (positive on the `perp(dir)` side).
    pub(crate) fn offset(&self, p: Point) -> f32 {
        dot(sub(p, self.origin), perp(self.dir))
    }

    /// Position of the foot of `p` along the line.
    pub(crate) fn along(&self, p: Point) -> f32 {
        dot(sub(p, self.origin), self.dir)
    }

    pub(crate) fn point(&self, t: f32) -> Point {
        at(self.origin, self.dir, t)
    }
}

/// Total least squares line through `pts`, or `None` for fewer than two
/// distinct points.
pub(crate) fn fit(pts: &[Point]) -> Option<Line> {
    if pts.len() < 2 {
        return None;
    }
    let n = pts.len() as f32;
    let (sx, sy) = pts.iter().fold((0.0, 0.0), |a, p| (a.0 + p.x, a.1 + p.y));
    let c = Point::new(sx / n, sy / n);
    let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
    for p in pts {
        let (dx, dy) = (p.x - c.x, p.y - c.y);
        xx += dx * dx;
        xy += dx * dy;
        yy += dy * dy;
    }
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    let dir = unit((angle.cos(), angle.sin()))?;
    (xx + yy > 1e-9).then_some(Line { origin: c, dir })
}

/// Line through `pts` after dropping outliers: points farther than
/// `max_residual` from the fit are removed one by one, worst first, while
/// at least `min_points` remain. Returns the line and the kept points.
pub(crate) fn robust_fit(
    pts: &[Point],
    max_residual: f32,
    min_points: usize,
) -> Option<(Line, Vec<Point>)> {
    let mut kept: Vec<Point> = pts.to_vec();
    loop {
        let line = fit(&kept)?;
        let (worst, dist) = kept
            .iter()
            .enumerate()
            .map(|(i, &p)| (i, line.offset(p).abs()))
            .fold((0, 0.0), |a, b| if b.1 > a.1 { b } else { a });
        if dist <= max_residual {
            return Some((line, kept));
        }
        if kept.len() <= min_points {
            return None;
        }
        kept.swap_remove(worst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_a_slanted_line() {
        let pts: Vec<Point> = (0..20)
            .map(|i| Point::new(10.0 + 0.2 * i as f32, 3.0 + i as f32))
            .collect();
        let l = fit(&pts).unwrap();
        assert!(l.dir.1.abs() > 0.97);
        for p in &pts {
            assert!(l.offset(*p).abs() < 1e-3);
        }
    }

    #[test]
    fn outliers_are_dropped() {
        let mut pts: Vec<Point> = (0..30).map(|i| Point::new(5.0, i as f32 * 2.0)).collect();
        pts[7] = Point::new(25.0, 14.0);
        pts[20] = Point::new(-9.0, 40.0);
        let (l, kept) = robust_fit(&pts, 0.5, 4).unwrap();
        assert_eq!(kept.len(), 28);
        assert!(l.dir.0.abs() < 1e-3);
    }

    #[test]
    fn too_scattered_points_fail() {
        let pts = [
            Point::new(0.0, 0.0),
            Point::new(10.0, 1.0),
            Point::new(1.0, 10.0),
            Point::new(11.0, 11.0),
        ];
        assert!(robust_fit(&pts, 0.2, 3).is_none());
    }
}
