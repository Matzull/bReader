use std::f64::consts::PI;

use image::{GrayImage, Luma};
use imageproc::filter::gaussian_blur_f32;
use rand::Rng;
use rand_chacha::ChaCha8Rng;

use crate::encode::Encoded;
use crate::render::{self, Geometry, Rect, add_noise, uniform};

const KINDS: usize = 8;

fn region(w: u32, h: u32, rng: &mut ChaCha8Rng) -> (u32, u32, u32, u32) {
    let rw = (f64::from(w) * rng.random_range(0.3..1.0)) as u32;
    let rh = (f64::from(h) * rng.random_range(0.3..1.0)) as u32;
    let x = rng.random_range(0..=w - rw);
    let y = rng.random_range(0..=h - rh);
    (x, y, rw, rh)
}

fn stripes(img: &mut GrayImage, rng: &mut ChaCha8Rng) {
    let (w, h) = img.dimensions();
    let diag = f64::from(w).hypot(f64::from(h));
    for _ in 0..rng.random_range(1..=3) {
        let (x0, y0, rw, rh) = region(w, h, rng);
        let angle = if rng.random_bool(0.5) {
            0.0
        } else {
            rng.random_range(0.0..PI)
        };
        let min_bar = rng.random_range(1..=6u32);
        let mut bounds = Vec::new();
        let mut u = -diag;
        while u < 2.0 * diag {
            u += f64::from(min_bar * rng.random_range(1..=4u32));
            bounds.push(u);
        }
        let dark: u8 = rng.random_range(0..=60);
        let (s, c) = angle.sin_cos();
        for y in y0..y0 + rh {
            for x in x0..x0 + rw {
                let u = f64::from(x) * c + f64::from(y) * s;
                let idx = bounds.partition_point(|&b| b < u);
                if idx % 2 == 1 {
                    img.put_pixel(x, y, Luma([dark]));
                }
            }
        }
    }
}

fn fill_rect(img: &mut GrayImage, x: u32, y: u32, w: u32, h: u32, v: u8) {
    for yy in y..(y + h).min(img.height()) {
        for xx in x..(x + w).min(img.width()) {
            img.put_pixel(xx, yy, Luma([v]));
        }
    }
}

fn text_blobs(img: &mut GrayImage, rng: &mut ChaCha8Rng) {
    let (w, h) = img.dimensions();
    let (x0, y0, rw, rh) = region(w, h, rng);
    let line_h = rng.random_range(14..=30u32);
    let ink: u8 = rng.random_range(20..=90);
    let mut y = y0;
    while y + line_h < y0 + rh {
        let mut x = x0;
        let bh = (f64::from(line_h) * rng.random_range(0.4..0.7)) as u32;
        while x < x0 + rw {
            let ww = rng.random_range(10..=80u32);
            fill_rect(img, x, y + (line_h - bh) / 2, ww.min(x0 + rw - x), bh, ink);
            x += ww + rng.random_range(6..=14);
        }
        y += line_h + line_h / 2;
        if rng.random_bool(0.1) {
            y += 2 * line_h;
        }
    }
}

fn noise_pattern(img: &mut GrayImage, rng: &mut ChaCha8Rng) {
    let (w, h) = img.dimensions();
    match rng.random_range(0..3) {
        0 => {
            for p in img.pixels_mut() {
                p.0[0] = rng.random();
            }
        }
        1 => {
            let cell = rng.random_range(2..=6u32);
            for y in (0..h).step_by(cell as usize) {
                for x in (0..w).step_by(cell as usize) {
                    let v = if rng.random_bool(0.5) { 20 } else { 235 };
                    fill_rect(img, x, y, cell, cell, v);
                }
            }
        }
        _ => {
            for p in img.pixels_mut() {
                p.0[0] = 128;
            }
            add_noise(img, 45.0, rng);
            *img = gaussian_blur_f32(img, rng.random_range(0.6..1.6));
        }
    }
}

fn checkerboard(img: &mut GrayImage, rng: &mut ChaCha8Rng) {
    let (w, h) = img.dimensions();
    let (x0, y0, rw, rh) = region(w, h, rng);
    let cell = f64::from(rng.random_range(2..=40u32));
    let angle = if rng.random_bool(0.4) {
        0.0
    } else {
        rng.random_range(0.0..PI)
    };
    let (s, c) = angle.sin_cos();
    let dark: u8 = rng.random_range(0..=80);
    for y in y0..y0 + rh {
        for x in x0..x0 + rw {
            let u = (f64::from(x) * c + f64::from(y) * s) / cell;
            let v = (f64::from(y) * c - f64::from(x) * s) / cell;
            if (u.floor() as i64 + v.floor() as i64) % 2 != 0 {
                img.put_pixel(x, y, Luma([dark]));
            }
        }
    }
}

fn gradient(img: &mut GrayImage, rng: &mut ChaCha8Rng) {
    let (w, h) = img.dimensions();
    let phi = rng.random_range(0.0..2.0 * PI);
    let (dx, dy) = (phi.cos(), phi.sin());
    let diag = f64::from(w).hypot(f64::from(h));
    let (lo, hi) = (rng.random_range(0.0..80.0), rng.random_range(170.0..255.0));
    let radial = rng.random_bool(0.4);
    let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let (fx, fy) = (f64::from(x) - cx, f64::from(y) - cy);
        let t = if radial {
            (fx.hypot(fy) / (diag / 2.0)).min(1.0)
        } else {
            ((fx * dx + fy * dy) / diag + 0.5).clamp(0.0, 1.0)
        };
        p.0[0] = (lo + (hi - lo) * t) as u8;
    }
    if rng.random_bool(0.5) {
        add_noise(img, 3.0, rng);
    }
}

fn finder(modules: &mut [bool], n: usize, ox: usize, oy: usize) {
    for dy in 0..7 {
        for dx in 0..7 {
            let ring = dx == 0 || dx == 6 || dy == 0 || dy == 6;
            let core = (2..=4).contains(&dx) && (2..=4).contains(&dy);
            modules[(oy + dy) * n + ox + dx] = ring || core;
        }
    }
}

/// Random module grid with QR finder patterns but meaningless data.
fn qr_lookalike(rng: &mut ChaCha8Rng) -> Encoded {
    let n = 21 + 4 * rng.random_range(0..6usize);
    let mut modules: Vec<bool> = (0..n * n).map(|_| rng.random_bool(0.5)).collect();
    finder(&mut modules, n, 0, 0);
    finder(&mut modules, n, n - 7, 0);
    finder(&mut modules, n, 0, n - 7);
    for i in 0..8 {
        for (x, y) in [
            (i, 7),
            (7, i),
            (n - 8, i),
            (n - 8 + i.min(7), 7),
            (i, n - 8),
            (7, n - 8 + i.min(7)),
        ] {
            if x < n && y < n && !(x >= 8 && y >= 8) {
                modules[y * n + x] = false;
            }
        }
    }
    Encoded::matrix(String::new(), "NONE", modules, n)
}

/// Random bar widths framed by guard-like bars, shaped like an EAN/Code 128 symbol.
fn linear_lookalike(rng: &mut ChaCha8Rng) -> Encoded {
    let total = rng.random_range(67..=190usize);
    let mut bars = vec![1u8, 0, 1];
    let mut dark = false;
    while bars.len() + 3 < total {
        let run = rng.random_range(1..=4usize);
        bars.extend(std::iter::repeat_n(u8::from(dark), run));
        dark = !dark;
    }
    bars.extend([0, 1, 0, 1]);
    let height = (bars.len() as f64 * rng.random_range(0.2..0.45)).max(30.0);
    Encoded::linear(String::new(), "NONE", &bars, height)
}

fn lookalike(img: &mut GrayImage, qr: bool, rng: &mut ChaCha8Rng) {
    let enc = if qr {
        qr_lookalike(rng)
    } else {
        linear_lookalike(rng)
    };
    let light = img.get_pixel(0, 0).0[0];
    let code = render::rasterize(&enc, rng.random_range(0..=45), light);
    let geo = Geometry {
        module_px: if qr {
            uniform(rng, (3.0, 8.0))
        } else {
            uniform(rng, (2.0, 4.0))
        },
        angle: rng.random_range(0.0..2.0 * PI),
        tilt: uniform(rng, (0.0, 0.3)),
        tilt_dir: rng.random_range(0.0..2.0 * PI),
    };
    let sh = render::shape(&code, &geo);
    let cell = Rect {
        x: 0.0,
        y: 0.0,
        w: f64::from(img.width()),
        h: f64::from(img.height()),
    };
    let p = render::place(&code, &sh, cell, rng);
    render::draw(img, &code, &p);
}

pub fn generate(index: usize, rng: &mut ChaCha8Rng) -> GrayImage {
    let w = rng.random_range(640..=1600u32);
    let h = (f64::from(w) * rng.random_range(0.6..1.0)) as u32;
    let light: u8 = rng.random_range(200..=255);
    let mut img = GrayImage::from_pixel(w, h, Luma([light]));
    match index % KINDS {
        0 => stripes(&mut img, rng),
        1 => text_blobs(&mut img, rng),
        2 => noise_pattern(&mut img, rng),
        3 => checkerboard(&mut img, rng),
        4 => gradient(&mut img, rng),
        5 => lookalike(&mut img, true, rng),
        6 => lookalike(&mut img, false, rng),
        _ => {
            text_blobs(&mut img, rng);
            stripes(&mut img, rng);
        }
    }
    if rng.random_bool(0.3) {
        img = gaussian_blur_f32(&img, rng.random_range(0.5..1.5));
    }
    img
}
