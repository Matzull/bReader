//! Test scenes: a module grid drawn into a luma image through a quad, with
//! rotation, perspective, blur, noise and inversion.

use super::{Rendered, Rng, rxing_encode};
use breader_core::{BitMatrix, Perspective, Point};

/// Encodes up to `len` pseudo-random printable characters (fewer when they
/// do not fit) in a symbol of exactly `layers` layers (see `rxing_encode`).
pub(crate) fn rxing_fill(seed: u64, len: usize, layers: i32) -> Option<(Vec<u8>, Rendered)> {
    let mut rng = Rng(seed | 1);
    let mut data: Vec<u8> = (0..len).map(|_| b'!' + rng.below(90) as u8).collect();
    while !data.is_empty() {
        if let Some(r) = rxing_encode(&data, 23, layers, None)
            && r.layout.layers == layers.unsigned_abs() as usize
        {
            return Some((data, r));
        }
        data.truncate(data.len() * 4 / 5);
    }
    None
}

/// A module grid drawn into a luma image through a quad: the image corners of
/// module corners (0,0), (n,0), (n,n), (0,n).
#[derive(Debug, Clone)]
pub(crate) struct Render {
    pub w: usize,
    pub h: usize,
    pub quad: [Point; 4],
    /// Number of 3 x 3 box blurs.
    pub blur: usize,
    pub invert: bool,
    /// Amplitude of the uniform noise added to every pixel.
    pub noise: u32,
    pub seed: u64,
}

impl Render {
    pub(crate) fn draw(&self, grid: &BitMatrix) -> Vec<u8> {
        let n = grid.width() as f32;
        let sq = [
            Point::new(0.0, 0.0),
            Point::new(n, 0.0),
            Point::new(n, n),
            Point::new(0.0, n),
        ];
        let inv = Perspective::quad_to_quad(sq, self.quad)
            .unwrap()
            .inverse()
            .unwrap();
        let mut img = vec![235u8; self.w * self.h];
        for y in 0..self.h {
            for x in 0..self.w {
                let mut acc = 0u32;
                for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let p = Point::new(x as f32 + ox, y as f32 + oy);
                    let dark = inv.map(p).is_some_and(|q| {
                        q.x >= 0.0
                            && q.y >= 0.0
                            && q.x < n
                            && q.y < n
                            && grid.get(q.x as usize, q.y as usize)
                    });
                    acc += if dark { 25 } else { 235 };
                }
                img[y * self.w + x] = (acc / 4) as u8;
            }
        }
        for _ in 0..self.blur {
            img = box_blur(&img, self.w, self.h);
        }
        if self.noise > 0 {
            let mut rng = Rng(self.seed | 1);
            for p in &mut img {
                let d = rng.below(2 * self.noise as usize + 1) as i32 - self.noise as i32;
                *p = (i32::from(*p) + d).clamp(0, 255) as u8;
            }
        }
        if self.invert {
            for p in &mut img {
                *p = 255 - *p;
            }
        }
        img
    }
}

fn box_blur(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let at = |x: usize, y: usize, dx: isize, dy: isize| {
        let xx = (x as isize + dx).clamp(0, w as isize - 1) as usize;
        let yy = (y as isize + dy).clamp(0, h as isize - 1) as usize;
        u32::from(src[yy * w + xx])
    };
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    s += at(x, y, dx, dy);
                }
            }
            out[y * w + x] = (s / 9) as u8;
        }
    }
    out
}

/// Image corners of a square symbol of `n` modules of `px` pixels, rotated
/// by `deg` around `center`; `tilt` in [0, 0.5) shrinks the top edge towards
/// its middle (a perspective view from below, applied before the rotation).
pub(crate) fn quad_at(n: usize, center: Point, px: f32, deg: f32, tilt: f32) -> [Point; 4] {
    let h = n as f32 / 2.0 * px;
    let top = h * (1.0 - tilt);
    let (s, c) = deg.to_radians().sin_cos();
    [(-top, -h), (top, -h), (h, h), (-h, h)]
        .map(|(x, y)| Point::new(center.x + x * c - y * s, center.y + x * s + y * c))
}

/// Draws `grid` rotated, tilted and so on on a canvas with a margin of a few
/// modules; `f` adjusts the render (blur, noise, inversion). Returns the luma
/// image, its side and the quad.
pub(crate) fn scene(
    grid: &BitMatrix,
    px: f32,
    deg: f32,
    tilt: f32,
    f: impl Fn(&mut Render),
) -> (Vec<u8>, usize, [Point; 4]) {
    let n = grid.width();
    let side = (n as f32 * px * 1.5 + 12.0 * px).ceil() as usize;
    let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
    let quad = quad_at(n, center, px, deg, tilt);
    let mut r = Render {
        w: side,
        h: side,
        quad,
        blur: 0,
        invert: false,
        noise: 0,
        seed: 1,
    };
    f(&mut r);
    (r.draw(grid), side, quad)
}

/// Dark pixels (below 128) of a luma image.
pub(crate) fn binarize(luma: &[u8], w: usize, h: usize) -> BitMatrix {
    let mut m = BitMatrix::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if luma[y * w + x] < 128 {
                m.set(x, y, true);
            }
        }
    }
    m
}

/// The exact map from module offsets (from the center module) to the image
/// for a symbol of `n` modules drawn through `quad`.
pub(crate) fn truth_map(n: usize, quad: [Point; 4]) -> impl Fn(Point) -> Point {
    let m = n as f32;
    let sq = [
        Point::new(0.0, 0.0),
        Point::new(m, 0.0),
        Point::new(m, m),
        Point::new(0.0, m),
    ];
    let p = Perspective::quad_to_quad(sq, quad).unwrap();
    let c = (n / 2) as f32 + 0.5;
    move |o| p.map(Point::new(c + o.x, c + o.y)).unwrap()
}
