#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn distance(self, other: Self) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

/// Corners in symbol order: top-left, top-right, bottom-right, bottom-left
/// as read in the symbol's own orientation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Quad {
    pub corners: [Point; 4],
}

impl Quad {
    pub const fn new(corners: [Point; 4]) -> Self {
        Self { corners }
    }

    pub fn center(&self) -> Point {
        let (sx, sy) = self
            .corners
            .iter()
            .fold((0.0, 0.0), |(x, y), p| (x + p.x, y + p.y));
        Point::new(sx / 4.0, sy / 4.0)
    }

    /// Angle of the top edge in degrees, clockwise from the +x axis, in [0, 360).
    pub fn orientation_deg(&self) -> f32 {
        let [tl, tr, ..] = self.corners;
        (tr.y - tl.y)
            .atan2(tr.x - tl.x)
            .to_degrees()
            .rem_euclid(360.0)
    }

    pub fn contains(&self, p: Point) -> bool {
        let mut sign = 0i8;
        for i in 0..4 {
            let a = self.corners[i];
            let b = self.corners[(i + 1) % 4];
            let cross = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
            let s = if cross > 0.0 {
                1
            } else if cross < 0.0 {
                -1
            } else {
                continue;
            };
            if sign == 0 {
                sign = s;
            } else if sign != s {
                return false;
            }
        }
        sign != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Quad {
        Quad::new([
            Point::new(0.0, 0.0),
            Point::new(10.0, 0.0),
            Point::new(10.0, 10.0),
            Point::new(0.0, 10.0),
        ])
    }

    #[test]
    fn center_and_orientation() {
        let q = square();
        assert_eq!(q.center(), Point::new(5.0, 5.0));
        assert_eq!(q.orientation_deg(), 0.0);
    }

    #[test]
    fn contains() {
        let q = square();
        assert!(q.contains(Point::new(5.0, 5.0)));
        assert!(!q.contains(Point::new(11.0, 5.0)));
        assert!(!Quad::default().contains(Point::default()));
    }
}
