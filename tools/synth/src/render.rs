use std::f64::consts::PI;

use image::{GrayImage, Luma};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::encode::Encoded;

/// Source pixels per module when rasterising a symbol.
pub const SRC_SCALE: u32 = 4;

pub type Mat = [[f64; 3]; 3];
pub type Pt = (f64, f64);

pub fn mul(a: &Mat, b: &Mat) -> Mat {
    let mut r = [[0.0; 3]; 3];
    for (i, row) in r.iter_mut().enumerate() {
        for (j, v) in row.iter_mut().enumerate() {
            *v = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    r
}

pub fn apply(m: &Mat, p: Pt) -> Pt {
    let w = m[2][0] * p.0 + m[2][1] * p.1 + m[2][2];
    (
        (m[0][0] * p.0 + m[0][1] * p.1 + m[0][2]) / w,
        (m[1][0] * p.0 + m[1][1] * p.1 + m[1][2]) / w,
    )
}

pub fn inverse(m: &Mat) -> Mat {
    let c =
        |r0: usize, c0: usize, r1: usize, c1: usize| m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0];
    let adj = [
        [c(1, 1, 2, 2), -c(0, 1, 2, 2), c(0, 1, 1, 2)],
        [-c(1, 0, 2, 2), c(0, 0, 2, 2), -c(0, 0, 1, 2)],
        [c(1, 0, 2, 1), -c(0, 0, 2, 1), c(0, 0, 1, 1)],
    ];
    let det = m[0][0] * adj[0][0] + m[0][1] * adj[1][0] + m[0][2] * adj[2][0];
    let mut r = adj;
    for row in &mut r {
        for v in row {
            *v /= det;
        }
    }
    r
}

fn translate(x: f64, y: f64) -> Mat {
    [[1.0, 0.0, x], [0.0, 1.0, y], [0.0, 0.0, 1.0]]
}

fn scale(s: f64) -> Mat {
    [[s, 0.0, 0.0], [0.0, s, 0.0], [0.0, 0.0, 1.0]]
}

fn rotation(a: f64) -> Mat {
    let (s, c) = a.sin_cos();
    [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]
}

/// A rasterised symbol with its symbol rectangle (quiet zone excluded).
#[derive(Debug)]
pub struct CodeImage {
    pub img: GrayImage,
    pub rect: [Pt; 4],
}

impl CodeImage {
    fn full_corners(&self) -> [Pt; 4] {
        let (w, h) = (f64::from(self.img.width()), f64::from(self.img.height()));
        [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)]
    }
}

pub fn rasterize(enc: &Encoded, dark: u8, light: u8) -> CodeImage {
    let k = f64::from(SRC_SCALE);
    let qx = (enc.qz_x * k).round();
    let qy = (enc.qz_y * k).round();
    let sym_w = enc.cols as f64 * k;
    let row_h = (enc.cell_h * k).round();
    let sym_h = enc.rows as f64 * row_h;
    let (w, h) = ((sym_w + 2.0 * qx) as u32, (sym_h + 2.0 * qy) as u32);
    let mut img = GrayImage::from_pixel(w, h, Luma([light]));
    for (x, y, p) in img.enumerate_pixels_mut() {
        let (fx, fy) = (f64::from(x) - qx, f64::from(y) - qy);
        if fx < 0.0 || fy < 0.0 || fx >= sym_w || fy >= sym_h {
            continue;
        }
        let col = (fx / k) as usize;
        let row = (fy / row_h) as usize;
        if enc.modules[row * enc.cols + col] {
            *p = Luma([dark]);
        }
    }
    CodeImage {
        img,
        rect: [
            (qx, qy),
            (qx + sym_w, qy),
            (qx + sym_w, qy + sym_h),
            (qx, qy + sym_h),
        ],
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    /// Pixels per module in the destination (before fitting).
    pub module_px: f64,
    pub angle: f64,
    /// Perspective strength in 0..~0.6 and its direction.
    pub tilt: f64,
    pub tilt_dir: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Linear part of the placement (centred on the code) and its bounding box size.
#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub m: Mat,
    pub bbox_w: f64,
    pub bbox_h: f64,
}

fn bbox(pts: &[Pt]) -> (f64, f64, f64, f64) {
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in pts {
        b = (b.0.min(p.0), b.1.min(p.1), b.2.max(p.0), b.3.max(p.1));
    }
    b
}

pub fn shape(code: &CodeImage, g: &Geometry) -> Shape {
    let (w, h) = (f64::from(code.img.width()), f64::from(code.img.height()));
    let s = g.module_px / f64::from(SRC_SCALE);
    let extent = w.max(h) * s;
    let (a, b) = (
        g.tilt * g.tilt_dir.cos() / extent,
        g.tilt * g.tilt_dir.sin() / extent,
    );
    let persp: Mat = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [a, b, 1.0]];
    let m = mul(
        &persp,
        &mul(
            &rotation(g.angle),
            &mul(&scale(s), &translate(-w / 2.0, -h / 2.0)),
        ),
    );
    let pts: Vec<Pt> = code.full_corners().iter().map(|&p| apply(&m, p)).collect();
    let (x0, y0, x1, y1) = bbox(&pts);
    Shape {
        m,
        bbox_w: x1 - x0,
        bbox_h: y1 - y0,
    }
}

#[derive(Debug)]
pub struct Placement {
    pub h: Mat,
    pub quad: [Pt; 4],
    pub area: Rect,
}

/// Shrinks the shape to fit `cell` (with margin) and places it at a random position.
pub fn place(code: &CodeImage, sh: &Shape, cell: Rect, rng: &mut ChaCha8Rng) -> Placement {
    let f = (0.92 * cell.w / sh.bbox_w)
        .min(0.92 * cell.h / sh.bbox_h)
        .min(1.0);
    let m = mul(&scale(f), &sh.m);
    let (bw, bh) = (sh.bbox_w * f, sh.bbox_h * f);
    let pts: Vec<Pt> = code.full_corners().iter().map(|&p| apply(&m, p)).collect();
    let (x0, y0, _, _) = bbox(&pts);
    let slack_x = (cell.w - bw).max(0.0);
    let slack_y = (cell.h - bh).max(0.0);
    let left = cell.x + slack_x * rng.random_range(0.15..0.85);
    let top = cell.y + slack_y * rng.random_range(0.15..0.85);
    let h = mul(&translate(left - x0, top - y0), &m);
    let mut quad = [(0.0, 0.0); 4];
    for (q, &r) in quad.iter_mut().zip(&code.rect) {
        *q = apply(&h, r);
    }
    Placement {
        h,
        quad,
        area: Rect {
            x: left,
            y: top,
            w: bw,
            h: bh,
        },
    }
}

fn sample(img: &GrayImage, x: f64, y: f64) -> f64 {
    let (w, h) = (img.width() as i64, img.height() as i64);
    let (fx, fy) = (x - 0.5, y - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |ix: i64, iy: i64| {
        f64::from(
            img.get_pixel(ix.clamp(0, w - 1) as u32, iy.clamp(0, h - 1) as u32)
                .0[0],
        )
    };
    let (ix, iy) = (x0 as i64, y0 as i64);
    let top = at(ix, iy) * (1.0 - tx) + at(ix + 1, iy) * tx;
    let bot = at(ix, iy + 1) * (1.0 - tx) + at(ix + 1, iy + 1) * tx;
    top * (1.0 - ty) + bot * ty
}

/// Warps `code` onto `canvas` with 3x3 supersampling; outside the code the canvas shows through.
pub fn draw(canvas: &mut GrayImage, code: &CodeImage, p: &Placement) {
    let inv = inverse(&p.h);
    let (w, h) = (f64::from(code.img.width()), f64::from(code.img.height()));
    let x0 = (p.area.x.floor() as i64 - 2).max(0);
    let y0 = (p.area.y.floor() as i64 - 2).max(0);
    let x1 = ((p.area.x + p.area.w).ceil() as i64 + 2).min(i64::from(canvas.width()));
    let y1 = ((p.area.y + p.area.h).ceil() as i64 + 2).min(i64::from(canvas.height()));
    const N: usize = 3;
    for y in y0..y1 {
        for x in x0..x1 {
            let bg = f64::from(canvas.get_pixel(x as u32, y as u32).0[0]);
            let mut acc = 0.0;
            for j in 0..N {
                for i in 0..N {
                    let dp = (
                        x as f64 + (i as f64 + 0.5) / N as f64,
                        y as f64 + (j as f64 + 0.5) / N as f64,
                    );
                    let (sx, sy) = apply(&inv, dp);
                    acc += if sx >= 0.0 && sy >= 0.0 && sx < w && sy < h {
                        sample(&code.img, sx, sy)
                    } else {
                        bg
                    };
                }
            }
            canvas.put_pixel(
                x as u32,
                y as u32,
                Luma([(acc / (N * N) as f64).round() as u8]),
            );
        }
    }
}

pub fn uniform(rng: &mut ChaCha8Rng, range: (f64, f64)) -> f64 {
    if range.1 <= range.0 {
        range.0
    } else {
        rng.random_range(range.0..range.1)
    }
}

pub fn gaussian(rng: &mut ChaCha8Rng) -> f64 {
    let u1: f64 = rng.random_range(f64::EPSILON..1.0);
    let u2: f64 = rng.random_range(0.0..1.0);
    (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
}

pub fn add_noise(img: &mut GrayImage, sigma: f64, rng: &mut ChaCha8Rng) {
    for p in img.pixels_mut() {
        let v = f64::from(p.0[0]) + sigma * gaussian(rng);
        p.0[0] = v.round().clamp(0.0, 255.0) as u8;
    }
}

pub fn invert(img: &mut GrayImage) {
    for p in img.pixels_mut() {
        p.0[0] = 255 - p.0[0];
    }
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Strong linear and radial shading plus a soft shadow band, applied multiplicatively.
pub fn illuminate(img: &mut GrayImage, rng: &mut ChaCha8Rng) {
    let (w, h) = (f64::from(img.width()), f64::from(img.height()));
    let diag = w.hypot(h);
    let phi = rng.random_range(0.0..2.0 * PI);
    let (dx, dy) = (phi.cos(), phi.sin());
    let lin = rng.random_range(0.35..0.65);
    let radial = rng.random_range(0.0..0.45);
    let (cx, cy) = (
        rng.random_range(0.2..0.8) * w,
        rng.random_range(0.2..0.8) * h,
    );
    let band_phi = rng.random_range(0.0..PI);
    let (bx, by) = (band_phi.cos(), band_phi.sin());
    let band_centre = rng.random_range(-0.25..0.25) * diag;
    let band_half = rng.random_range(0.05..0.12) * diag;
    let band_gain = rng.random_range(0.35..0.6);
    let soft = 0.02 * diag;
    for (x, y, p) in img.enumerate_pixels_mut() {
        let (fx, fy) = (f64::from(x), f64::from(y));
        let t = ((fx - w / 2.0) * dx + (fy - h / 2.0) * dy) / diag + 0.5;
        let mut gain = 1.0 - lin * t.clamp(0.0, 1.0);
        let r = (fx - cx).hypot(fy - cy) / diag;
        gain *= 1.0 - radial * (2.0 * r).min(1.0).powi(2);
        let d = ((fx - w / 2.0) * bx + (fy - h / 2.0) * by - band_centre).abs();
        let inside = 1.0 - smoothstep(band_half - soft, band_half + soft, d);
        gain *= 1.0 - (1.0 - band_gain) * inside;
        p.0[0] = (f64::from(p.0[0]) * gain).round().clamp(0.0, 255.0) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_roundtrip() {
        let m: Mat = [[1.2, 0.3, 10.0], [-0.2, 0.9, 5.0], [0.0004, 0.0002, 1.0]];
        let inv = inverse(&m);
        let p = (123.0, 45.0);
        let q = apply(&inv, apply(&m, p));
        assert!((q.0 - p.0).abs() < 1e-9 && (q.1 - p.1).abs() < 1e-9);
    }

    #[test]
    fn identity_placement_keeps_corners() {
        use rand::SeedableRng;
        let enc = Encoded::matrix("x".into(), "QR_CODE", vec![true; 9], 3);
        let code = rasterize(&enc, 0, 255);
        let g = Geometry {
            module_px: f64::from(SRC_SCALE),
            angle: 0.0,
            tilt: 0.0,
            tilt_dir: 0.0,
        };
        let sh = shape(&code, &g);
        let mut rng = ChaCha8Rng::seed_from_u64(0);
        let cell = Rect {
            x: 0.0,
            y: 0.0,
            w: 400.0,
            h: 400.0,
        };
        let p = place(&code, &sh, cell, &mut rng);
        let side = p.quad[1].0 - p.quad[0].0;
        assert!((side - 3.0 * f64::from(SRC_SCALE)).abs() < 1e-6);
        let mut canvas = GrayImage::from_pixel(400, 400, Luma([255]));
        draw(&mut canvas, &code, &p);
        let c = (
            (p.quad[0].0 + p.quad[2].0) / 2.0,
            (p.quad[0].1 + p.quad[2].1) / 2.0,
        );
        assert_eq!(canvas.get_pixel(c.0 as u32, c.1 as u32).0[0], 0);
    }
}
