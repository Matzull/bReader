//! With the `rayon` feature, scans must return exactly what the
//! single-threaded paths return, symbol for symbol (D-069): symbology, data,
//! corners bit for bit and order, at every effort, on frames above the
//! parallel thresholds of the 1D reader (2^19 pixels), QR and imgproc (2^20).
#![cfg(feature = "rayon")]

use barcoders::sym::code39::Code39;
use barcoders::sym::code128::Code128;
use barcoders::sym::ean13::EAN13;
use breader::{Effort, ImageBuffer, Reader, ScanOptions, Symbol, Symbology};
use breader_core::parallel;
use datamatrix::{DataMatrix, SymbolList};
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
    blit_scaled(img, s, cx, cy, deg, 1.0, 1);
}

/// As [`blit`] with one sprite pixel covering `scale` frame pixels; every frame
/// pixel averages `ss` x `ss` samples (a print that is thinner than the frame
/// grid needs them to look like a photograph rather than a decimation).
fn blit_scaled(
    img: &mut ImageBuffer,
    s: &Sprite,
    cx: f32,
    cy: f32,
    deg: f32,
    scale: f32,
    ss: usize,
) {
    let (sin, cos) = deg.to_radians().sin_cos();
    let r = (s.w.max(s.h) as f32) * 0.75 * scale.max(1.0);
    let (x0, x1) = ((cx - r).max(0.0) as usize, (cx + r) as usize);
    let (y0, y1) = ((cy - r).max(0.0) as usize, (cy + r) as usize);
    for y in y0..y1.min(img.height()) {
        for x in x0..x1.min(img.width()) {
            let (mut dark, mut inside) = (0, 0);
            for sy in 0..ss {
                for sx in 0..ss {
                    let fx = x as f32 + (sx as f32 + 0.5) / ss as f32 - 0.5 - cx;
                    let fy = y as f32 + (sy as f32 + 0.5) / ss as f32 - 0.5 - cy;
                    let u = (fx * cos + fy * sin) / scale + s.w as f32 / 2.0;
                    let v = (-fx * sin + fy * cos) / scale + s.h as f32 / 2.0;
                    if u >= 0.0 && v >= 0.0 && (u as usize) < s.w && (v as usize) < s.h {
                        inside += 1;
                        dark += usize::from(s.dark[v as usize * s.w + u as usize]);
                    }
                }
            }
            if inside > 0 {
                let n = (ss * ss) as u32;
                // Samples outside the sprite are paper.
                let light = n - dark as u32;
                img.row_mut(y)[x] = ((25 * dark as u32 + 238 * light) / n) as u8;
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

/// A 1D symbol: `modules` (1 = bar) with a quiet zone of 10 modules on each
/// side, `px` sprite pixels per module and `height` rows. `erode` makes every
/// bar a quarter module thinner on each side (needs `px` = 4).
fn linear_sprite(modules: &[u8], px: usize, height: usize, erode: bool) -> Sprite {
    let mut row = vec![false; 10 * px];
    for &m in modules {
        row.extend([m == 1].repeat(px));
    }
    row.extend(vec![false; 10 * px]);
    if erode {
        let fine = row.clone();
        for i in 0..fine.len() {
            row[i] = fine[i] && i > 0 && fine[i - 1] && i + 1 < fine.len() && fine[i + 1];
        }
    }
    let w = row.len();
    Sprite {
        w,
        h: height,
        dark: row.repeat(height),
    }
}

fn dm_sprite(text: &str, px: usize) -> Sprite {
    let dm = DataMatrix::encode(text.as_bytes(), SymbolList::default().enforce_square()).unwrap();
    let bm = dm.bitmap();
    let (cols, rows, bits) = (bm.width(), bm.height(), bm.bits());
    let (w, h) = ((cols + 4) * px, (rows + 4) * px);
    let mut dark = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let (mx, my) = (x / px, y / px);
            if (2..2 + cols).contains(&mx) && (2..2 + rows).contains(&my) {
                dark[y * w + x] = bits[(my - 2) * cols + mx - 2];
            }
        }
    }
    Sprite { w, h, dark }
}

fn thin_code_39(text: &str) -> Sprite {
    linear_sprite(&Code39::new(text).unwrap().encode(), 4, 237, true)
}

fn ean13(digits: &str) -> Sprite {
    linear_sprite(&EAN13::new(digits).unwrap().encode(), 3, 110, false)
}

fn code128(text: &str) -> Sprite {
    linear_sprite(&Code128::new(text).unwrap().encode(), 2, 90, false)
}

/// A page of 1.9 million pixels with the hard case of the 1D reader (thin
/// Code 39, about 1.35 samples per module, whose reads go through the template
/// fallback of D-058, D-061 and D-062), EAN-13, Code 128, QR and Data Matrix,
/// some turned.
fn mixed_page(seed: u32, tilt: f32) -> ImageBuffer {
    let (w, h) = (1600, 1200);
    let mut img = ImageBuffer::new(w, h);
    let mut state = seed;
    for (i, p) in img.as_mut_slice().iter_mut().enumerate() {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        *p = (222 + (i % w) * 14 / w + (state >> 30) as usize) as u8;
    }
    // Thin Code 39 at three phases and slight turns; one pixel of the sprite
    // covers 1.35 / 4 frame pixels.
    for (i, text) in ["9033296", "ABC-123", "BREADER 39"].iter().enumerate() {
        let (cx, cy) = (330.0 + 0.17 * i as f32, 130.0 + 150.0 * i as f32);
        let deg = tilt * i as f32 * 0.2;
        blit_scaled(&mut img, &thin_code_39(text), cx, cy, deg, 1.35 / 4.0, 3);
    }
    blit(&mut img, &ean13("590123412345"), 1150.0, 120.0, tilt * 0.5);
    blit(
        &mut img,
        &code128("\u{0106}123456789012"),
        1150.0,
        280.0,
        -tilt * 0.3,
    );
    blit(
        &mut img,
        &code128("\u{0181}Code-128 text"),
        1150.0,
        420.0,
        90.0,
    );
    let qr = qr_sprite("https://example.org/qr-in-a-mixed-page", EcLevel::M, 4);
    blit(&mut img, &qr, 300.0, 800.0, tilt);
    blit(
        &mut img,
        &qr_sprite("QR TWO 0123456789", EcLevel::Q, 3),
        700.0,
        850.0,
        tilt * 3.0,
    );
    blit(
        &mut img,
        &dm_sprite("Data Matrix 4711", 5),
        1100.0,
        800.0,
        tilt * 2.0,
    );
    blit(
        &mut img,
        &dm_sprite("Second DM 0815 / Z", 4),
        1400.0,
        1000.0,
        90.0 + tilt,
    );
    img
}

/// Scans at every effort in parallel and forced sequential, and requires the
/// same symbols in the same order: `PartialEq` and the `Debug` text, which
/// shows every digit of the corners. Returns the Balanced result.
fn assert_same(img: &ImageBuffer, what: &str) -> Vec<Symbol> {
    let mut balanced = Vec::new();
    for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
        let par = scan(img, effort);
        let seq = parallel::sequential(|| scan(img, effort));
        assert_eq!(par, seq, "{what} {effort:?}");
        assert_eq!(format!("{par:?}"), format!("{seq:?}"), "{what} {effort:?}");
        if effort == Effort::Balanced {
            balanced = par;
        }
    }
    balanced
}

#[test]
fn mixed_pages_with_thin_code_39_read_the_same_in_parallel() {
    let (mut code39, mut other) = (0, 0);
    for (seed, tilt) in [(1, 0.0), (2, 4.0), (3, -7.0), (4, 11.0)] {
        let img = mixed_page(seed, tilt);
        assert!(img.width() * img.height() >= 1 << 20);
        for s in assert_same(&img, &format!("mixed seed {seed} tilt {tilt}")) {
            if s.symbology == Symbology::Code39 {
                code39 += 1;
            } else {
                other += 1;
            }
        }
    }
    assert!(
        code39 >= 4,
        "only {code39} Code 39 symbols read at Balanced"
    );
    assert!(other >= 20, "only {other} other symbols read at Balanced");
}

/// A frame above the 1D threshold (2^19 pixels) and below that of QR and
/// imgproc (2^20): only the 1D reader goes parallel.
#[test]
fn frames_between_the_thresholds_read_the_same() {
    let (w, h) = (1000, 560);
    assert!((1 << 19..1 << 20).contains(&(w * h)));
    let mut img = ImageBuffer::new(w, h);
    img.as_mut_slice().fill(232);
    blit_scaled(
        &mut img,
        &thin_code_39("9033296"),
        300.0,
        150.0,
        0.0,
        1.35 / 4.0,
        3,
    );
    blit(&mut img, &ean13("590123412345"), 700.0, 150.0, 0.0);
    blit(
        &mut img,
        &qr_sprite("between", EcLevel::M, 4),
        300.0,
        400.0,
        5.0,
    );
    blit(&mut img, &dm_sprite("DM between", 5), 700.0, 400.0, 0.0);
    assert_same(&img, "between thresholds");
}

fn collect_images(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            collect_images(&p, out);
        } else if matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("png" | "jpg" | "jpeg")
        ) {
            out.push(p);
        }
    }
}

/// The synthetic dataset (`cargo run --release -p synth -- data/synth --seed 1`),
/// when it has been generated.
#[test]
fn synthetic_dataset_reads_the_same_in_parallel() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/synth");
    let mut files = Vec::new();
    collect_images(&root, &mut files);
    if files.is_empty() {
        eprintln!("skipped: {} has no images", root.display());
        return;
    }
    let mut big = 0;
    for path in &files {
        let decoded = image::ImageReader::open(path)
            .and_then(|r| r.with_guessed_format())
            .map_err(|e| e.to_string())
            .and_then(|r| r.decode().map_err(|e| e.to_string()));
        let gray = decoded
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .into_luma8();
        let (w, h) = (gray.width() as usize, gray.height() as usize);
        let mut img = ImageBuffer::new(w, h);
        img.as_mut_slice().copy_from_slice(gray.as_raw());
        big += usize::from(w * h >= 1 << 19);
        assert_same(&img, &path.display().to_string());
    }
    eprintln!(
        "{} images, {big} of them with 2^19 pixels or more",
        files.len()
    );
}
