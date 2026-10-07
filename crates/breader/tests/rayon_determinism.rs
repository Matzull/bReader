//! With the `rayon` feature, scans must return exactly what the
//! single-threaded paths return, symbol for symbol.
#![cfg(feature = "rayon")]

use breader::{Effort, ImageBuffer, Reader, ScanOptions, Symbol};
use breader_core::parallel;
use qrcode::{Color, EcLevel, QrCode};

const L_CODE: [&str; 10] = [
    "0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011",
    "0110111", "0001011",
];

struct Sprite {
    w: usize,
    h: usize,
    dark: Vec<bool>,
}

fn qr_sprite(text: &str, level: EcLevel, px: usize) -> Sprite {
    let code = QrCode::with_error_correction_level(text.as_bytes(), level).unwrap();
    let dim = code.width();
    let side = (dim + 8) * px;
    let colors = code.to_colors();
    let mut dark = vec![false; side * side];
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / px, y / px);
            if (4..4 + dim).contains(&mx) && (4..4 + dim).contains(&my) {
                dark[y * side + x] = colors[(my - 4) * dim + mx - 4] == Color::Dark;
            }
        }
    }
    Sprite {
        w: side,
        h: side,
        dark,
    }
}

fn ean_sprite(digits: &str, px: usize, height: usize) -> Sprite {
    let d: Vec<usize> = digits.bytes().map(|b| usize::from(b - b'0')).collect();
    // EAN-8 layout: L codes on the left half, inverted L codes (R) on the right.
    let bits = |s: &str| s.bytes().map(|b| b == b'1').collect::<Vec<_>>();
    let mut m = bits("101");
    for &v in &d[..4] {
        m.extend(bits(L_CODE[v]));
    }
    m.extend(bits("01010"));
    for &v in &d[4..] {
        m.extend(bits(L_CODE[v]).iter().map(|b| !b));
    }
    m.extend(bits("101"));
    let (w, h) = ((m.len() + 20) * px, height);
    let mut dark = vec![false; w * h];
    for y in 0..h {
        for (i, &bar) in m.iter().enumerate() {
            if bar {
                dark[y * w + (10 + i) * px..y * w + (11 + i) * px].fill(true);
            }
        }
    }
    Sprite { w, h, dark }
}

/// Draws the sprite centred at `(cx, cy)` turned by `deg` degrees.
fn blit(img: &mut ImageBuffer, s: &Sprite, cx: f32, cy: f32, deg: f32) {
    let (sin, cos) = deg.to_radians().sin_cos();
    let r = (s.w.max(s.h) as f32) * 0.75;
    let (x0, x1) = ((cx - r).max(0.0) as usize, (cx + r) as usize);
    let (y0, y1) = ((cy - r).max(0.0) as usize, (cy + r) as usize);
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            let (dx, dy) = (x as f32 - cx, y as f32 - cy);
            let u = dx * cos + dy * sin + s.w as f32 / 2.0;
            let v = -dx * sin + dy * cos + s.h as f32 / 2.0;
            if u >= 0.0 && v >= 0.0 && (u as usize) < s.w && (v as usize) < s.h {
                img.row_mut(y)[x] = if s.dark[v as usize * s.w + u as usize] {
                    25
                } else {
                    238
                };
            }
        }
    }
}

fn page(seed: u32, tilt: f32) -> ImageBuffer {
    let (w, h) = (1700, 1300);
    let mut img = ImageBuffer::new(w, h);
    let mut state = seed;
    for (i, p) in img.as_mut_slice().iter_mut().enumerate() {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        let shade = 215 + (i % w) * 20 / w;
        *p = (shade + (state >> 29) as usize) as u8;
    }
    let stamp = ["https://example.org/a", "HELLO WORLD 0123456789", "x"];
    let levels = [EcLevel::M, EcLevel::H, EcLevel::L];
    for (i, text) in stamp.iter().enumerate() {
        let s = qr_sprite(text, levels[i], 4 + i);
        let (cx, cy) = (260.0 + 520.0 * i as f32, 280.0);
        blit(&mut img, &s, cx, cy, tilt * (i as f32 - 1.0));
    }
    let stamp = qr_sprite(&"Long payload 0123456789. ".repeat(12), EcLevel::Q, 4);
    blit(&mut img, &stamp, 450.0, 760.0, 17.0 + tilt);
    for (i, digits) in ["12345670", "40170725", "96385074", "00000019"]
        .iter()
        .enumerate()
    {
        let s = ean_sprite(digits, 3, 110);
        let (cx, cy) = (
            1000.0 + 330.0 * (i % 2) as f32,
            700.0 + 240.0 * (i / 2) as f32,
        );
        blit(&mut img, &s, cx, cy, 90.0 * (i % 2) as f32 + tilt * 2.0);
    }
    img
}

fn scan(img: &ImageBuffer, effort: Effort) -> Vec<Symbol> {
    Reader::with_options(ScanOptions::with_effort(effort)).scan(img.view())
}

#[test]
fn parallel_scans_equal_sequential_scans() {
    let mut total = 0;
    for (seed, tilt) in [(1, 0.0), (2, 9.0), (3, -23.0)] {
        let img = page(seed, tilt);
        for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
            let par = scan(&img, effort);
            let seq = parallel::sequential(|| scan(&img, effort));
            assert_eq!(par, seq, "seed {seed} tilt {tilt} {effort:?}");
            total += par.len();
        }
    }
    assert!(total >= 20, "only {total} symbols read");
}
