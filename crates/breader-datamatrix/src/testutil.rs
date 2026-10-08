//! Test-only helpers: encode with the `datamatrix` dev-dependency (black box),
//! render modules to luma with similarity or perspective transforms, blur and
//! inversion, and binarize.

use breader_core::{BitMatrix, Perspective, Point};
use datamatrix::{DataMatrix, SymbolList, SymbolSize};

#[derive(Debug, Clone)]
pub struct Modules {
    pub rows: usize,
    pub cols: usize,
    pub dark: Vec<bool>,
    pub payload: Vec<u8>,
}

impl Modules {
    pub fn get(&self, c: usize, r: usize) -> bool {
        self.dark[r * self.cols + c]
    }
}

/// Encodes a pseudo-random payload seeded by `seed` that fills exactly the
/// given symbol size as far as the encoder allows.
pub fn encode_size(seed: &[u8], size: SymbolSize) -> Modules {
    let mut state = seed.iter().fold(0x9E37_79B9_7F4A_7C15u64, |s, &b| {
        (s ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3)
    });
    let base: Vec<u8> = (0..3000)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            b'!' + ((state >> 40) % 90) as u8
        })
        .collect();
    let list = SymbolList::with_whitelist([size]);
    let (mut lo, mut hi) = (1usize, base.len());
    let mut best = None;
    while lo <= hi {
        let mid = (lo + hi) / 2;
        match DataMatrix::encode(&base[..mid], list.clone()) {
            Ok(dm) => {
                best = Some((dm, mid));
                lo = mid + 1;
            }
            Err(_) => hi = mid - 1,
        }
    }
    let (dm, len) = best.expect("some payload fits the symbol");
    let bm = dm.bitmap();
    Modules {
        rows: bm.height(),
        cols: bm.width(),
        dark: bm.bits().to_vec(),
        payload: base[..len].to_vec(),
    }
}

/// Image corners of module corners (0,0), (cols,0), (cols,rows), (0,rows).
pub fn similarity_quad(
    rows: usize,
    cols: usize,
    center: Point,
    px_per_module: f32,
    deg: f32,
) -> [Point; 4] {
    let (hx, hy) = (
        cols as f32 / 2.0 * px_per_module,
        rows as f32 / 2.0 * px_per_module,
    );
    let (s, c) = deg.to_radians().sin_cos();
    [(-hx, -hy), (hx, -hy), (hx, hy), (-hx, hy)]
        .map(|(x, y)| Point::new(center.x + x * c - y * s, center.y + x * s + y * c))
}

#[derive(Debug, Clone)]
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
        let (cw, ch) = (m.cols as f32, m.rows as f32);
        let sq = [
            Point::new(0.0, 0.0),
            Point::new(cw, 0.0),
            Point::new(cw, ch),
            Point::new(0.0, ch),
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
                        let (u, v) = if self.mirrored {
                            (cw - q.x, q.y)
                        } else {
                            (q.x, q.y)
                        };
                        let (u, v) = warp(u, v);
                        u >= 0.0
                            && v >= 0.0
                            && u < cw
                            && v < ch
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

pub fn threshold(luma: &[u8], w: usize, h: usize) -> BitMatrix {
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

/// Renders a symbol centered in a canvas with margin and returns the binary
/// image with the true quad.
pub fn scene(
    m: &Modules,
    px: f32,
    deg: f32,
    mirrored: bool,
    blur: usize,
    invert: bool,
) -> (BitMatrix, [Point; 4]) {
    let diag = ((m.rows * m.rows + m.cols * m.cols) as f32).sqrt() * px;
    let side = (diag + 4.0 * px * 4.0).ceil() as usize + 8;
    let center = Point::new(side as f32 / 2.0, side as f32 / 2.0);
    let quad = similarity_quad(m.rows, m.cols, center, px, deg);
    let r = Render {
        w: side,
        h: side,
        quad,
        mirrored,
        blur,
        invert,
    };
    (threshold(&r.draw(m), side, side), quad)
}

pub fn all_sizes() -> Vec<SymbolSize> {
    use SymbolSize::*;
    vec![
        Square10, Square12, Square14, Square16, Square18, Square20, Square22, Square24, Square26,
        Square32, Square36, Square40, Square44, Square48, Square52, Square64, Square72, Square80,
        Square88, Square96, Square104, Square120, Square132, Square144, Rect8x18, Rect8x32,
        Rect12x26, Rect12x36, Rect16x36, Rect16x48, Rect8x48, Rect8x64, Rect8x80, Rect8x96,
        Rect8x120, Rect8x144, Rect12x64, Rect12x88, Rect16x64, Rect20x36, Rect20x44, Rect20x64,
        Rect22x48, Rect24x48, Rect24x64, Rect26x40, Rect26x48, Rect26x64,
    ]
}
