use breader_core::{Effort, Point};

/// A straight sampling line in pass-image coordinates. Integer coordinates
/// address pixel centers; both endpoints lie inside the image.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scanline {
    pub from: Point,
    pub to: Point,
}

impl Scanline {
    pub fn new(from: Point, to: Point) -> Self {
        Self { from, to }
    }

    pub fn length(&self) -> f32 {
        self.from.distance(self.to)
    }

    /// Unit direction, `(1, 0)` for a degenerate line.
    pub fn direction(&self) -> (f32, f32) {
        let len = self.length();
        if len > 1e-6 {
            (
                (self.to.x - self.from.x) / len,
                (self.to.y - self.from.y) / len,
            )
        } else {
            (1.0, 0.0)
        }
    }

    pub fn point_at(&self, t: f32) -> Point {
        let (dx, dy) = self.direction();
        Point::new(self.from.x + dx * t, self.from.y + dy * t)
    }

    #[cfg(test)]
    pub fn is_axis_aligned(&self) -> bool {
        self.from.y == self.to.y || self.from.x == self.to.x
    }
}

/// Shortest line that can contain any supported symbol including quiet zones.
const MIN_LINE_LENGTH: f32 = 60.0;

/// Densities (lines across the image extent) per direction family.
#[derive(Debug, Clone, Copy)]
struct Density {
    axis: usize,
    diagonal: usize,
}

fn density(effort: Effort) -> Density {
    match effort {
        Effort::Fast => Density {
            axis: 32,
            diagonal: 0,
        },
        Effort::Balanced => Density {
            axis: 64,
            diagonal: 32,
        },
        Effort::Exhaustive => Density {
            axis: 128,
            diagonal: 64,
        },
    }
}

fn max_spacing(effort: Effort, angle_deg: f32) -> f32 {
    let base = match effort {
        Effort::Fast => 48.0,
        Effort::Balanced => 24.0,
        Effort::Exhaustive => 12.0,
    };
    if is_axis(angle_deg) { base } else { base * 1.5 }
}

fn is_axis(angle_deg: f32) -> bool {
    angle_deg % 90.0 == 0.0
}

/// Directions in degrees, in the order they are emitted.
fn angles(effort: Effort) -> &'static [f32] {
    match effort {
        Effort::Fast => &[0.0, 90.0],
        Effort::Balanced => &[0.0, 90.0, 45.0, 135.0, 22.5, 67.5, 112.5, 157.5],
        Effort::Exhaustive => &[
            0.0, 90.0, 45.0, 135.0, 15.0, 75.0, 105.0, 165.0, 30.0, 60.0, 120.0, 150.0,
        ],
    }
}

/// Scanline generator for one pass image.
#[derive(Debug, Clone, Copy)]
pub struct ScanPlan {
    width: usize,
    height: usize,
    effort: Effort,
}

impl ScanPlan {
    pub fn new(width: usize, height: usize, effort: Effort) -> Self {
        Self {
            width,
            height,
            effort,
        }
    }

    /// Spacing in pixels between parallel lines of the given direction.
    pub fn spacing(&self, angle_deg: f32) -> f32 {
        let d = density(self.effort);
        let count = if is_axis(angle_deg) {
            d.axis
        } else {
            d.diagonal
        }
        .max(1);
        (self.extent(angle_deg) / count as f32).clamp(2.0, max_spacing(self.effort, angle_deg))
    }

    /// Spacing to the neighbouring lines of the plan for one of its lines.
    pub fn line_spacing(&self, line: &Scanline) -> f32 {
        if line.from.x == line.to.x {
            return self.spacing(90.0);
        }
        if line.from.y == line.to.y {
            return self.spacing(0.0);
        }
        let (dx, dy) = line.direction();
        let angle = dy.atan2(dx).to_degrees().rem_euclid(180.0);
        self.spacing(if is_axis(angle) { angle + 0.01 } else { angle })
    }

    fn extent(&self, angle_deg: f32) -> f32 {
        let (s, c) = angle_deg.to_radians().sin_cos();
        self.width as f32 * s.abs() + self.height as f32 * c.abs()
    }

    /// Fills `out` with the scanlines of the plan, each direction ordered
    /// from the image center outwards.
    pub fn lines(&self, out: &mut Vec<Scanline>) {
        out.clear();
        for &angle in angles(self.effort) {
            self.direction_lines(angle, out);
        }
    }

    fn direction_lines(&self, angle_deg: f32, out: &mut Vec<Scanline>) {
        let spacing = self.spacing(angle_deg);
        let half = self.extent(angle_deg) / 2.0;
        let (w1, h1) = (self.width as f32 - 1.0, self.height as f32 - 1.0);
        let (cx, cy) = (w1 / 2.0, h1 / 2.0);
        let mut k = 0u32;
        loop {
            let first = k as f32 * spacing;
            if first > half {
                break;
            }
            let offsets = if k == 0 {
                [Some(0.0), None]
            } else {
                [Some(first), Some(-first)]
            };
            for t in offsets.into_iter().flatten() {
                if let Some(line) = self.clip(angle_deg, cx, cy, t) {
                    out.push(line);
                }
            }
            k += 1;
        }
    }

    fn clip(&self, angle_deg: f32, cx: f32, cy: f32, offset: f32) -> Option<Scanline> {
        let (w1, h1) = (self.width as f32 - 1.0, self.height as f32 - 1.0);
        let line = match angle_deg {
            0.0 => {
                let y = (cy + offset).round();
                (0.0 <= y && y <= h1).then(|| Scanline::new(Point::new(0.0, y), Point::new(w1, y)))
            }
            90.0 => {
                let x = (cx + offset).round();
                (0.0 <= x && x <= w1).then(|| Scanline::new(Point::new(x, 0.0), Point::new(x, h1)))
            }
            _ => {
                let (s, c) = angle_deg.to_radians().sin_cos();
                let (px, py) = (cx - s * offset, cy + c * offset);
                let (lo, hi) = clip_param(px, py, c, s, w1, h1)?;
                Some(Scanline::new(
                    Point::new(px + c * lo, py + s * lo),
                    Point::new(px + c * hi, py + s * hi),
                ))
            }
        }?;
        (line.length() >= MIN_LINE_LENGTH).then_some(line)
    }
}

/// Clips the segment `a`-`b` to the image. Axis-aligned segments snap to
/// pixel coordinates, as the profile reader expects.
pub fn clip_line(a: Point, b: Point, width: usize, height: usize, snap: bool) -> Option<Scanline> {
    let (w1, h1) = (width as f32 - 1.0, height as f32 - 1.0);
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len = dx.hypot(dy);
    if len < 1e-3 {
        return None;
    }
    let (ux, uy) = (dx / len, dy / len);
    let (lo, hi) = clip_param(a.x, a.y, ux, uy, w1, h1)?;
    let (lo, hi) = (lo.max(0.0), hi.min(len));
    if hi - lo < MIN_LINE_LENGTH {
        return None;
    }
    let mut from = Point::new(a.x + ux * lo, a.y + uy * lo);
    let mut to = Point::new(a.x + ux * hi, a.y + uy * hi);
    if snap {
        let clamp = |p: Point| Point::new(p.x.round().clamp(0.0, w1), p.y.round().clamp(0.0, h1));
        (from, to) = (clamp(from), clamp(to));
    }
    Some(Scanline::new(from, to))
}

/// Parameter interval of `p + t * (dx, dy)` inside `[0, w1] x [0, h1]`.
fn clip_param(px: f32, py: f32, dx: f32, dy: f32, w1: f32, h1: f32) -> Option<(f32, f32)> {
    let mut lo = f32::NEG_INFINITY;
    let mut hi = f32::INFINITY;
    for (p, d, max) in [(px, dx, w1), (py, dy, h1)] {
        if d.abs() < 1e-6 {
            if p < 0.0 || p > max {
                return None;
            }
        } else {
            let (a, b) = ((0.0 - p) / d, (max - p) / d);
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
        }
    }
    (lo.is_finite() && hi.is_finite() && lo < hi).then_some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(w: usize, h: usize, effort: Effort) -> Vec<Scanline> {
        let mut v = Vec::new();
        ScanPlan::new(w, h, effort).lines(&mut v);
        v
    }

    #[test]
    fn fast_has_only_axis_lines_with_expected_spacing() {
        let v = lines(640, 480, Effort::Fast);
        assert!(v.iter().all(Scanline::is_axis_aligned));
        let horizontal = v.iter().filter(|l| l.from.y == l.to.y).count();
        let vertical = v.len() - horizontal;
        assert!((31..=33).contains(&horizontal), "{horizontal}");
        assert!((31..=33).contains(&vertical), "{vertical}");
    }

    #[test]
    fn center_line_comes_first_and_order_spreads_outwards() {
        let v = lines(640, 480, Effort::Fast);
        assert_eq!(v[0].from.y, 240.0);
        let d: Vec<f32> = v
            .iter()
            .take_while(|l| l.from.y == l.to.y)
            .map(|l| (l.from.y - 239.5).abs())
            .collect();
        assert!(d.windows(3).all(|w| w[2] + 0.01 >= w[0]));
    }

    #[test]
    fn effort_increases_line_count() {
        let f = lines(800, 600, Effort::Fast).len();
        let b = lines(800, 600, Effort::Balanced).len();
        let e = lines(800, 600, Effort::Exhaustive).len();
        assert!(f < b && b < e, "{f} {b} {e}");
    }

    #[test]
    fn lines_stay_inside_the_image() {
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            for (w, h) in [(640, 480), (100, 1000), (97, 61), (1, 1), (5000, 70)] {
                for l in lines(w, h, effort) {
                    for p in [l.from, l.to] {
                        assert!(p.x >= -1e-3 && p.x <= w as f32 - 1.0 + 1e-3, "{p:?}");
                        assert!(p.y >= -1e-3 && p.y <= h as f32 - 1.0 + 1e-3, "{p:?}");
                    }
                    assert!(l.length() >= MIN_LINE_LENGTH);
                }
            }
        }
    }

    #[test]
    fn minimum_spacing_is_two_pixels() {
        let plan = ScanPlan::new(100, 100, Effort::Exhaustive);
        assert_eq!(plan.spacing(0.0), 2.0);
        assert!(lines(100, 100, Effort::Exhaustive).len() < 600);
    }

    #[test]
    fn clip_line_clips_and_snaps_axis_aligned_segments() {
        let l = clip_line(
            Point::new(-50.2, 10.0),
            Point::new(300.4, 10.0),
            200,
            100,
            true,
        )
        .unwrap();
        assert_eq!(
            (l.from.x, l.from.y, l.to.x, l.to.y),
            (0.0, 10.0, 199.0, 10.0)
        );
        assert!(
            clip_line(
                Point::new(0.0, 150.0),
                Point::new(100.0, 150.0),
                200,
                100,
                true
            )
            .is_none()
        );
        assert!(
            clip_line(
                Point::new(10.0, 10.0),
                Point::new(30.0, 10.0),
                200,
                100,
                true
            )
            .is_none()
        );
        let d = clip_line(
            Point::new(-10.0, -10.0),
            Point::new(150.0, 150.0),
            200,
            100,
            false,
        )
        .unwrap();
        assert!(d.from.x >= 0.0 && d.from.y >= 0.0 && d.to.y <= 99.0 + 1e-3);
    }

    #[test]
    fn diagonal_lines_have_the_requested_angles() {
        for (effort, step) in [(Effort::Balanced, 22.5f32), (Effort::Exhaustive, 15.0)] {
            let v = lines(640, 480, effort);
            let diag: Vec<_> = v.iter().filter(|l| !l.is_axis_aligned()).collect();
            assert!(!diag.is_empty());
            for l in diag {
                let (dx, dy) = l.direction();
                let angle = dy.atan2(dx).to_degrees().rem_euclid(180.0);
                let k = (angle / step).round();
                assert!((angle - k * step).abs() < 0.05, "{angle}");
            }
        }
    }
}
