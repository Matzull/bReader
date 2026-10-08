//! Test-only QR renderer: module grid to luma with similarity or perspective
//! transforms, blur and inversion.

use breader_core::{Perspective, Point};
use qrcode::{Color, EcLevel, QrCode, Version};

pub struct Modules {
    pub dim: usize,
    pub dark: Vec<bool>,
}

impl Modules {
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.dark[y * self.dim + x]
    }
}

pub fn make_modules(version: i16, data: &[u8]) -> Modules {
    let code = QrCode::with_version(data, Version::Normal(version), EcLevel::M).unwrap();
    let dim = code.width();
    let dark = code.to_colors().iter().map(|c| *c == Color::Dark).collect();
    Modules { dim, dark }
}

/// Image corners of module corners (0,0), (d,0), (d,d), (0,d).
pub fn similarity_quad(dim: usize, center: Point, px_per_module: f32, deg: f32) -> [Point; 4] {
    let h = dim as f32 / 2.0 * px_per_module;
    let (s, c) = deg.to_radians().sin_cos();
    [(-h, -h), (h, -h), (h, h), (-h, h)]
        .map(|(x, y)| Point::new(center.x + x * c - y * s, center.y + x * s + y * c))
}

pub struct Render {
    pub w: usize,
    pub h: usize,
    pub quad: [Point; 4],
    pub mirrored: bool,
    pub blur: usize,
    pub invert: bool,
}

impl Render {
    pub fn draw(&self, m: &Modules) -> Vec<u8> {
        self.draw_with(m, |u, v| (u, v))
    }

    /// Draws with the module coordinates displaced by `warp`, a stand-in for
    /// an uneven module pitch.
    pub fn draw_with(&self, m: &Modules, warp: impl Fn(f32, f32) -> (f32, f32)) -> Vec<u8> {
        let d = m.dim as f32;
        let sq = [
            Point::new(0.0, 0.0),
            Point::new(d, 0.0),
            Point::new(d, d),
            Point::new(0.0, d),
        ];
        let inv = Perspective::quad_to_quad(sq, self.quad)
            .unwrap()
            .inverse()
            .unwrap();
        let mut img = vec![255u8; self.w * self.h];
        for y in 0..self.h {
            for x in 0..self.w {
                let mut acc = 0u32;
                for (ox, oy) in [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)] {
                    let p = Point::new(x as f32 + ox, y as f32 + oy);
                    let dark = inv.map(p).is_some_and(|q| {
                        let (u, v) = warp(q.x, q.y);
                        let (u, v) = if self.mirrored { (v, u) } else { (u, v) };
                        u >= 0.0
                            && v >= 0.0
                            && u < d
                            && v < d
                            && m.get(u.floor() as usize, v.floor() as usize)
                    });
                    acc += if dark { 0 } else { 255 };
                }
                img[y * self.w + x] = (acc / 4) as u8;
            }
        }
        if self.blur > 0 {
            img = box_blur(&img, self.w, self.h, self.blur);
        }
        if self.invert {
            for p in &mut img {
                *p = 255 - *p;
            }
        }
        img
    }
}

fn box_blur(src: &[u8], w: usize, h: usize, r: usize) -> Vec<u8> {
    let mut tmp = vec![0u8; w * h];
    let mut out = vec![0u8; w * h];
    let n = (2 * r + 1) as u32;
    for y in 0..h {
        for x in 0..w {
            let mut s = 0;
            for k in 0..=2 * r {
                let xx = (x + k).saturating_sub(r).min(w - 1);
                s += u32::from(src[y * w + xx]);
            }
            tmp[y * w + x] = (s / n) as u8;
        }
    }
    for y in 0..h {
        for x in 0..w {
            let mut s = 0;
            for k in 0..=2 * r {
                let yy = (y + k).saturating_sub(r).min(h - 1);
                s += u32::from(tmp[yy * w + x]);
            }
            out[y * w + x] = (s / n) as u8;
        }
    }
    out
}

pub fn threshold(luma: &[u8], w: usize, h: usize) -> breader_core::BitMatrix {
    let mut m = breader_core::BitMatrix::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if luma[y * w + x] < 128 {
                m.set(x, y, true);
            }
        }
    }
    m
}
