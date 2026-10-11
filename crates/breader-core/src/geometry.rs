/// 2D point in pixels; x grows to the right, y downwards.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f32,
    /// Vertical coordinate.
    pub y: f32,
}

impl Point {
    /// Creates a point.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Euclidean distance to `other`.
    pub fn distance(self, other: Self) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }
}

/// Planar projective transform (homography), row-major 3x3 in f64.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Perspective {
    m: [[f64; 3]; 3],
}

impl Perspective {
    /// Maps the unit square (0,0), (1,0), (1,1), (0,1) onto `q` in that order.
    pub fn square_to_quad(q: [Point; 4]) -> Option<Self> {
        let [p0, p1, p2, p3] = q.map(|p| (f64::from(p.x), f64::from(p.y)));
        let sx = p0.0 - p1.0 + p2.0 - p3.0;
        let sy = p0.1 - p1.1 + p2.1 - p3.1;
        let (dx1, dy1) = (p1.0 - p2.0, p1.1 - p2.1);
        let (dx2, dy2) = (p3.0 - p2.0, p3.1 - p2.1);
        let det = dx1 * dy2 - dx2 * dy1;
        if det.abs() < 1e-12 {
            return None;
        }
        let g = (sx * dy2 - dx2 * sy) / det;
        let h = (dx1 * sy - sx * dy1) / det;
        let m = [
            [p1.0 - p0.0 + g * p1.0, p3.0 - p0.0 + h * p3.0, p0.0],
            [p1.1 - p0.1 + g * p1.1, p3.1 - p0.1 + h * p3.1, p0.1],
            [g, h, 1.0],
        ];
        Some(Self { m })
    }

    /// Maps quad `from` onto quad `to`, corner by corner.
    pub fn quad_to_quad(from: [Point; 4], to: [Point; 4]) -> Option<Self> {
        let a = Self::square_to_quad(from)?.inverse()?;
        let b = Self::square_to_quad(to)?;
        Some(b.then(&a))
    }

    /// `self ∘ first`: applies `first`, then `self`.
    fn then(&self, first: &Self) -> Self {
        let (a, b) = (&self.m, &first.m);
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
            }
        }
        Self { m }
    }

    /// Inverse transform, or `None` when singular.
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.m;
        let c = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        let adj = [
            [c(1, 2, 1, 2), -c(0, 2, 1, 2), c(0, 1, 1, 2)],
            [-c(1, 2, 0, 2), c(0, 2, 0, 2), -c(0, 1, 0, 2)],
            [c(1, 2, 0, 1), -c(0, 2, 0, 1), c(0, 1, 0, 1)],
        ];
        let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        Some(Self {
            m: adj.map(|row| row.map(|v| v / det)),
        })
    }

    /// Applies the transform. Returns `None` for points mapped to infinity.
    pub fn map(&self, p: Point) -> Option<Point> {
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        let m = &self.m;
        let w = m[2][0] * x + m[2][1] * y + m[2][2];
        if w.abs() < 1e-12 {
            return None;
        }
        let u = (m[0][0] * x + m[0][1] * y + m[0][2]) / w;
        let v = (m[1][0] * x + m[1][1] * y + m[1][2]) / w;
        (u.is_finite() && v.is_finite()).then(|| Point::new(u as f32, v as f32))
    }
}

/// Projective map fitted by least squares to point pairs.
#[derive(Debug, Clone, Copy)]
pub struct Homography {
    m: [[f64; 3]; 3],
}

impl Homography {
    /// Fits `src -> dst` (module to image) with at least four pairs.
    pub fn fit(pairs: &[(Point, Point)]) -> Option<Self> {
        let n = pairs.len();
        if n < 4 {
            return None;
        }
        // Mean and mean distance to it of one side of the pairs, in two
        // passes so that no buffer is needed.
        let norm = |side: fn(&(Point, Point)) -> Point| -> (f64, f64, f64) {
            let at = |p: &(Point, Point)| {
                let q = side(p);
                (f64::from(q.x), f64::from(q.y))
            };
            let (cx, cy) = (
                pairs.iter().map(|p| at(p).0).sum::<f64>() / n as f64,
                pairs.iter().map(|p| at(p).1).sum::<f64>() / n as f64,
            );
            let md = pairs
                .iter()
                .map(|p| (at(p).0 - cx).hypot(at(p).1 - cy))
                .sum::<f64>()
                / n as f64;
            (cx, cy, if md > 1e-9 { md } else { 1.0 })
        };
        let (mcx, mcy, mmd) = norm(|p| p.0);
        let (icx, icy, imd) = norm(|p| p.1);
        let mut ata = [[0.0f64; 9]; 8];
        for (s, d) in pairs {
            let u = (f64::from(s.x) - mcx) / mmd;
            let v = (f64::from(s.y) - mcy) / mmd;
            let x = (f64::from(d.x) - icx) / imd;
            let y = (f64::from(d.y) - icy) / imd;
            let rows = [
                ([u, v, 1.0, 0.0, 0.0, 0.0, -x * u, -x * v], x),
                ([0.0, 0.0, 0.0, u, v, 1.0, -y * u, -y * v], y),
            ];
            for (r, rhs) in rows {
                for i in 0..8 {
                    for j in 0..8 {
                        ata[i][j] += r[i] * r[j];
                    }
                    ata[i][8] += r[i] * rhs;
                }
            }
        }
        for col in 0..8 {
            let piv = (col..8).max_by(|&a, &b| ata[a][col].abs().total_cmp(&ata[b][col].abs()))?;
            if ata[piv][col].abs() < 1e-12 {
                return None;
            }
            ata.swap(col, piv);
            for r in 0..8 {
                if r != col {
                    let f = ata[r][col] / ata[col][col];
                    let pivot_row = ata[col];
                    for (dst, src) in ata[r][col..].iter_mut().zip(&pivot_row[col..]) {
                        *dst -= f * src;
                    }
                }
            }
        }
        let h: Vec<f64> = (0..8).map(|i| ata[i][8] / ata[i][i]).collect();
        let hn = [[h[0], h[1], h[2]], [h[3], h[4], h[5]], [h[6], h[7], 1.0]];
        // H = Timg^-1 * Hn * Tmod
        let tmod = [
            [1.0 / mmd, 0.0, -mcx / mmd],
            [0.0, 1.0 / mmd, -mcy / mmd],
            [0.0, 0.0, 1.0],
        ];
        let timg_inv = [[imd, 0.0, icx], [0.0, imd, icy], [0.0, 0.0, 1.0]];
        let mul = |a: &[[f64; 3]; 3], b: &[[f64; 3]; 3]| {
            let mut o = [[0.0; 3]; 3];
            for i in 0..3 {
                for j in 0..3 {
                    o[i][j] = (0..3).map(|k| a[i][k] * b[k][j]).sum();
                }
            }
            o
        };
        let m = mul(&timg_inv, &mul(&hn, &tmod));
        m.iter()
            .flatten()
            .all(|v| v.is_finite())
            .then_some(Self { m })
    }

    /// Inverse transform, or `None` when singular.
    pub fn inverse(&self) -> Option<Self> {
        let m = &self.m;
        let c = |r0: usize, r1: usize, c0: usize, c1: usize| {
            m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
        };
        let adj = [
            [c(1, 2, 1, 2), -c(0, 2, 1, 2), c(0, 1, 1, 2)],
            [-c(1, 2, 0, 2), c(0, 2, 0, 2), -c(0, 1, 0, 2)],
            [c(1, 2, 0, 1), -c(0, 2, 0, 1), c(0, 1, 0, 1)],
        ];
        let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
        if det.abs() < 1e-12 || !det.is_finite() {
            return None;
        }
        Some(Self {
            m: adj.map(|row| row.map(|v| v / det)),
        })
    }

    /// Applies the transform; `None` for points mapped to infinity.
    pub fn map(&self, p: Point) -> Option<Point> {
        let (x, y) = (f64::from(p.x), f64::from(p.y));
        let m = &self.m;
        let w = m[2][0] * x + m[2][1] * y + m[2][2];
        if w.abs() < 1e-12 {
            return None;
        }
        let u = (m[0][0] * x + m[0][1] * y + m[0][2]) / w;
        let v = (m[1][0] * x + m[1][1] * y + m[1][2]) / w;
        (u.is_finite() && v.is_finite()).then(|| Point::new(u as f32, v as f32))
    }
}

/// Corners in symbol order: top-left, top-right, bottom-right, bottom-left
/// as read in the symbol's own orientation.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Quad {
    /// The four corners, in the order described on [`Quad`].
    pub corners: [Point; 4],
}

impl Quad {
    /// Creates a quad from its corners.
    pub const fn new(corners: [Point; 4]) -> Self {
        Self { corners }
    }

    /// Mean of the four corners.
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

    /// Whether `p` lies inside the (convex) quad; points on an edge count as inside.
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

    fn close(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn perspective_maps_corners() {
        let from = square().corners;
        let to = [
            Point::new(100.0, 50.0),
            Point::new(300.0, 80.0),
            Point::new(280.0, 260.0),
            Point::new(90.0, 240.0),
        ];
        let t = Perspective::quad_to_quad(from, to).unwrap();
        for (f, e) in from.iter().zip(to) {
            assert!(close(t.map(*f).unwrap(), e));
        }
        let inv = t.inverse().unwrap();
        let p = Point::new(3.0, 7.0);
        assert!(close(inv.map(t.map(p).unwrap()).unwrap(), p));
    }

    #[test]
    fn perspective_degenerate() {
        let p = Point::new(1.0, 1.0);
        assert!(Perspective::square_to_quad([p; 4]).is_none());
    }

    #[test]
    fn contains() {
        let q = square();
        assert!(q.contains(Point::new(5.0, 5.0)));
        assert!(!q.contains(Point::new(11.0, 5.0)));
        assert!(!Quad::default().contains(Point::default()));
    }
}
