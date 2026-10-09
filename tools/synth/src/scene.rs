use std::f64::consts::PI;

use image::codecs::jpeg::JpegEncoder;
use image::{ExtendedColorType, GrayImage, ImageFormat, Luma};
use imageproc::filter::gaussian_blur_f32;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::encode::{self, DmShape, Sym};
use crate::negative;
use crate::render::{self, Geometry, Pt, Rect, add_noise, uniform};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Clean,
    Rotated,
    Perspective,
    Blur,
    LowRes,
    Illumination,
    Inverted,
    Noise,
    Multi,
    Negative,
    Datamatrix,
}

#[derive(Debug, Clone)]
pub struct Annotation {
    pub text: String,
    pub format: &'static str,
    pub points: [Pt; 4],
}

type Range = (f64, f64);

struct Config {
    qr_module: Range,
    lin_module: Range,
    angle_deg: Range,
    tilt: Range,
}

impl Class {
    pub const ALL: [Class; 11] = [
        Class::Clean,
        Class::Rotated,
        Class::Perspective,
        Class::Blur,
        Class::LowRes,
        Class::Illumination,
        Class::Inverted,
        Class::Noise,
        Class::Multi,
        Class::Negative,
        Class::Datamatrix,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Class::Clean => "clean",
            Class::Rotated => "rotated",
            Class::Perspective => "perspective",
            Class::Blur => "blur",
            Class::LowRes => "low-res",
            Class::Illumination => "illumination",
            Class::Inverted => "inverted",
            Class::Noise => "noise",
            Class::Multi => "multi",
            Class::Negative => "negative",
            Class::Datamatrix => "datamatrix",
        }
    }

    fn index(self) -> u64 {
        Self::ALL.iter().position(|&c| c == self).unwrap_or(0) as u64
    }

    fn config(self) -> Config {
        let (qr_module, lin_module, angle_deg, tilt) = match self {
            Class::Clean => ((6.0, 10.0), (3.0, 5.0), (0.0, 0.0), (0.0, 0.0)),
            Class::Rotated => ((4.0, 8.0), (2.5, 4.0), (0.0, 360.0), (0.0, 0.0)),
            Class::Perspective => ((4.0, 8.0), (2.5, 4.0), (-25.0, 25.0), (0.2, 0.5)),
            Class::Blur => ((5.0, 8.0), (3.0, 4.5), (-10.0, 10.0), (0.0, 0.0)),
            Class::LowRes => ((2.0, 3.0), (1.5, 2.5), (-5.0, 5.0), (0.0, 0.0)),
            Class::Illumination => ((4.0, 8.0), (2.5, 4.0), (-15.0, 15.0), (0.0, 0.0)),
            Class::Inverted => ((5.0, 8.0), (3.0, 4.0), (-10.0, 10.0), (0.0, 0.0)),
            Class::Noise => ((5.0, 8.0), (3.0, 4.0), (-10.0, 10.0), (0.0, 0.0)),
            Class::Multi => ((3.0, 5.0), (2.0, 3.5), (-20.0, 20.0), (0.0, 0.25)),
            Class::Negative => ((3.0, 8.0), (2.0, 4.0), (0.0, 360.0), (0.0, 0.3)),
            Class::Datamatrix => ((3.0, 7.0), (2.0, 4.0), (-30.0, 30.0), (0.0, 0.25)),
        };
        Config {
            qr_module,
            lin_module,
            angle_deg,
            tilt,
        }
    }
}

fn splitmix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn image_rng(seed: u64, class: Class, index: usize) -> ChaCha8Rng {
    ChaCha8Rng::seed_from_u64(splitmix(
        splitmix(seed) ^ (class.index() << 40) ^ index as u64,
    ))
}

fn geometry(cfg: &Config, sym: Sym, rng: &mut ChaCha8Rng) -> Geometry {
    let module_px = match sym {
        Sym::Qr | Sym::DataMatrix | Sym::Aztec => uniform(rng, cfg.qr_module),
        _ => uniform(rng, cfg.lin_module),
    };
    Geometry {
        module_px,
        angle: uniform(rng, cfg.angle_deg).to_radians(),
        tilt: uniform(rng, cfg.tilt),
        tilt_dir: rng.random_range(0.0..2.0 * PI),
    }
}

pub fn canvas_size(rng: &mut ChaCha8Rng, min_w: u32, max_w: u32) -> (u32, u32) {
    let w = rng.random_range(min_w..=max_w);
    let aspect = rng.random_range(0.6..1.0);
    (w, (f64::from(w) * aspect) as u32)
}

fn rect_of(w: u32, h: u32) -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: f64::from(w),
        h: f64::from(h),
    }
}

fn single(
    class: Class,
    sym: Sym,
    shape: Option<DmShape>,
    rng: &mut ChaCha8Rng,
) -> (GrayImage, Vec<Annotation>) {
    let enc = match shape {
        Some(shape) => encode::encode_dm(rng, shape),
        None => encode::encode(rng, sym),
    };
    single_with(class, sym, enc, rng)
}

fn single_with(
    class: Class,
    sym: Sym,
    enc: encode::Encoded,
    rng: &mut ChaCha8Rng,
) -> (GrayImage, Vec<Annotation>) {
    let cfg = class.config();
    let light: u8 = rng.random_range(220..=255);
    let dark: u8 = rng.random_range(0..=45);
    let code = render::rasterize(&enc, dark, light);
    let geo = geometry(&cfg, sym, rng);
    let sh = render::shape(&code, &geo);

    let (mut w, mut h) = canvas_size(rng, 640, 1600);
    if sh.bbox_w > 0.9 * f64::from(w) {
        w = ((sh.bbox_w / 0.85).ceil() as u32).min(1600);
    }
    if sh.bbox_h > 0.9 * f64::from(h) {
        h = (sh.bbox_h / 0.85).ceil() as u32;
    }
    let mut canvas = GrayImage::from_pixel(w, h, Luma([light]));
    let p = render::place(&code, &sh, rect_of(w, h), rng);
    render::draw(&mut canvas, &code, &p);
    let ann = Annotation {
        text: enc.text,
        format: enc.format,
        points: p.quad,
    };
    (canvas, vec![ann])
}

/// Which symbols a `multi` image carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mix {
    Original,
    DataMatrix,
    Pdf417,
    Aztec,
}

fn multi(rng: &mut ChaCha8Rng, mix: Mix) -> (GrayImage, Vec<Annotation>) {
    let cfg = Class::Multi.config();
    let n = rng.random_range(2..=5usize);
    let (cols, rows) = match n {
        2 => (1, 2),
        3 | 4 => (2, 2),
        _ => (3, 2),
    };
    let w = rng.random_range(1400..=1600u32);
    let h = rng.random_range(900..=1200u32);
    let light: u8 = rng.random_range(220..=255);
    let mut canvas = GrayImage::from_pixel(w, h, Luma([light]));
    let mut cells: Vec<usize> = (0..cols * rows).collect();
    cells.shuffle(rng);
    let (cw, ch) = (f64::from(w) / cols as f64, f64::from(h) / rows as f64);
    let mut anns = Vec::new();
    for &cell in cells.iter().take(n) {
        let sym = Sym::ALL[rng.random_range(0..Sym::ALL.len())];
        let sym = match mix {
            Mix::Original => sym,
            Mix::DataMatrix => Sym::DataMatrix,
            Mix::Pdf417 => Sym::Pdf417,
            Mix::Aztec => Sym::Aztec,
        };
        let enc = encode::encode(rng, sym);
        let dark: u8 = rng.random_range(0..=45);
        let code = render::rasterize(&enc, dark, light);
        let geo = geometry(&cfg, sym, rng);
        let sh = render::shape(&code, &geo);
        let rect = Rect {
            x: (cell % cols) as f64 * cw,
            y: (cell / cols) as f64 * ch,
            w: cw,
            h: ch,
        };
        let p = render::place(&code, &sh, rect, rng);
        render::draw(&mut canvas, &code, &p);
        anns.push(Annotation {
            text: enc.text,
            format: enc.format,
            points: p.quad,
        });
    }
    (canvas, anns)
}

fn jpeg_roundtrip(img: &GrayImage, quality: u8) -> GrayImage {
    let mut buf = Vec::new();
    JpegEncoder::new_with_quality(&mut buf, quality)
        .encode(
            img.as_raw(),
            img.width(),
            img.height(),
            ExtendedColorType::L8,
        )
        .expect("jpeg encode of an in-memory image");
    image::load_from_memory_with_format(&buf, ImageFormat::Jpeg)
        .expect("jpeg decode of freshly encoded data")
        .into_luma8()
}

/// Applies the class-specific degradation to a single-symbol image.
fn degrade(class: Class, img: &mut GrayImage, rng: &mut ChaCha8Rng) {
    match class {
        Class::Blur => {
            let sigma = rng.random_range(0.5..2.5) as f32;
            *img = gaussian_blur_f32(img, sigma);
        }
        Class::Illumination => render::illuminate(img, rng),
        Class::Inverted => render::invert(img),
        Class::Noise => {
            let sigma = rng.random_range(6.0..18.0);
            add_noise(img, sigma, rng);
            *img = jpeg_roundtrip(img, rng.random_range(30..=70));
        }
        _ => {}
    }
}

pub fn generate(class: Class, index: usize, seed: u64) -> (GrayImage, Vec<Annotation>) {
    let mut rng = image_rng(seed, class, index);
    let rng = &mut rng;
    match class {
        Class::Negative => (negative::generate(index, rng), Vec::new()),
        Class::Multi => multi(rng, Mix::Original),
        Class::Datamatrix => {
            let shape = [
                DmShape::Square,
                DmShape::Square,
                DmShape::Rect,
                DmShape::Dmre,
            ][index % 4];
            let (mut img, anns) = single(class, Sym::DataMatrix, Some(shape), rng);
            if index % 5 == 4 {
                render::invert(&mut img);
            }
            if index % 3 == 2 {
                let sigma = rng.random_range(0.5..1.5) as f32;
                img = gaussian_blur_f32(&img, sigma);
            }
            (img, anns)
        }
        _ => {
            let sym = Sym::CYCLE[index % Sym::CYCLE.len()];
            let (mut img, anns) = single(class, sym, None, rng);
            degrade(class, &mut img, rng);
            (img, anns)
        }
    }
}

/// Additional Data Matrix images of a class, generated after its regular
/// images so the regular ones stay byte-identical to the pre-Data-Matrix sets.
pub fn generate_extra(class: Class, j: usize, seed: u64) -> Option<(GrayImage, Vec<Annotation>)> {
    let mut rng = image_rng(seed.wrapping_add(0xD47A_5EED), class, j);
    let rng = &mut rng;
    match class {
        Class::Negative | Class::Datamatrix => None,
        Class::Multi => Some(multi(rng, Mix::DataMatrix)),
        _ => {
            let (mut img, anns) = single(class, Sym::DataMatrix, Some(DmShape::Square), rng);
            degrade(class, &mut img, rng);
            Some((img, anns))
        }
    }
}

/// PDF417 images of a class, generated after the Data Matrix ones so that
/// every earlier image stays byte-identical.
pub fn generate_pdf417(class: Class, j: usize, seed: u64) -> Option<(GrayImage, Vec<Annotation>)> {
    let mut rng = image_rng(seed.wrapping_add(0x7DF4_1700), class, j);
    let rng = &mut rng;
    match class {
        Class::Negative | Class::Datamatrix => None,
        Class::Multi => Some(multi(rng, Mix::Pdf417)),
        _ => {
            let (mut img, anns) = single(class, Sym::Pdf417, None, rng);
            degrade(class, &mut img, rng);
            Some((img, anns))
        }
    }
}

/// Aztec images of a class, generated after the PDF417 ones so that every
/// earlier image stays byte-identical: layer counts cycle over compact and
/// full-range symbols, every third image is mirrored.
pub fn generate_aztec(class: Class, j: usize, seed: u64) -> Option<(GrayImage, Vec<Annotation>)> {
    let mut rng = image_rng(seed.wrapping_add(0xA27E_C000), class, j);
    let rng = &mut rng;
    match class {
        Class::Negative | Class::Datamatrix => None,
        Class::Multi => Some(multi(rng, Mix::Aztec)),
        _ => {
            let pick = j + class.index() as usize * 3;
            let enc = encode::encode_aztec(rng, pick, j % 3 == 2);
            let (mut img, anns) = single_with(class, Sym::Aztec, enc, rng);
            degrade(class, &mut img, rng);
            Some((img, anns))
        }
    }
}
