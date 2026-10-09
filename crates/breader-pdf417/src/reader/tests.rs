use super::*;
use crate::testutil::{EncodeOptions, Rng, modules};
use breader_core::{
    Binarizer, FrameContext, ImageView, Pass, Perspective, Preprocessor, ScanOptions, Scratch,
    Symbology,
};
use breader_linear::LinearReader;

/// A rendered scene: the symbol, placed, degraded and rasterized.
#[derive(Clone)]
struct Scene {
    modules: Vec<Vec<u8>>,
    /// Pixels per module.
    px: f32,
    /// Row height in modules.
    row_h: f32,
    angle_deg: f32,
    /// Offsets of the destination corners, in pixels, for perspective.
    skew: [(f32, f32); 4],
    blur: f32,
    noise: f32,
    invert: bool,
    seed: u64,
    /// Rectangles (fractions of the symbol, x0 y0 x1 y1) painted background.
    damage: Vec<[f32; 4]>,
}

struct Raster {
    data: Vec<u8>,
    w: usize,
    h: usize,
}

impl Scene {
    fn new(msg: &str, opts: EncodeOptions, px: f32) -> Self {
        Self {
            modules: modules(msg, opts).expect("fits"),
            px,
            row_h: 3.5,
            angle_deg: 0.0,
            skew: [(0.0, 0.0); 4],
            blur: 0.0,
            noise: 0.0,
            invert: false,
            seed: 1,
            damage: Vec::new(),
        }
    }

    fn raster(&self) -> Raster {
        let quiet = 3.0;
        let rows = self.modules.len();
        let cols = self.modules[0].len();
        let (sw, sh) = (
            cols as f32 + 2.0 * quiet,
            rows as f32 * self.row_h + 2.0 * quiet,
        );
        let (sin, cos) = self.angle_deg.to_radians().sin_cos();
        let corner = |x: f32, y: f32, k: usize| {
            let (dx, dy) = ((x - 0.5) * sw * self.px, (y - 0.5) * sh * self.px);
            Point::new(
                cos * dx - sin * dy + self.skew[k].0,
                sin * dx + cos * dy + self.skew[k].1,
            )
        };
        let dest = [
            corner(0.0, 0.0, 0),
            corner(1.0, 0.0, 1),
            corner(1.0, 1.0, 2),
            corner(0.0, 1.0, 3),
        ];
        let (minx, maxx) = dest
            .iter()
            .fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.x), a.1.max(p.x)));
        let (miny, maxy) = dest
            .iter()
            .fold((f32::MAX, f32::MIN), |a, p| (a.0.min(p.y), a.1.max(p.y)));
        let pad = 24.0;
        let (w, h) = (
            (maxx - minx + 2.0 * pad).ceil() as usize,
            (maxy - miny + 2.0 * pad).ceil() as usize,
        );
        let placed = dest.map(|p| Point::new(p.x - minx + pad, p.y - miny + pad));
        let from = [
            Point::new(0.0, 0.0),
            Point::new(sw, 0.0),
            Point::new(sw, sh),
            Point::new(0.0, sh),
        ];
        let inv = Perspective::quad_to_quad(placed, from).expect("invertible");
        let (bg, fg) = if self.invert {
            (30.0, 225.0)
        } else {
            (225.0, 30.0)
        };
        let bit = |u: f32, v: f32| -> f32 {
            let (mu, mv) = (u - quiet, (v - quiet) / self.row_h);
            if mu < 0.0 || mv < 0.0 || mu >= cols as f32 || mv >= rows as f32 {
                return bg;
            }
            let (fu, fv) = (mu / cols as f32, mv / rows as f32);
            if self
                .damage
                .iter()
                .any(|d| fu >= d[0] && fu < d[2] && fv >= d[1] && fv < d[3])
            {
                return bg;
            }
            if self.modules[mv as usize][mu as usize] != 0 {
                fg
            } else {
                bg
            }
        };
        let mut img = vec![0f32; w * h];
        for y in 0..h {
            for x in 0..w {
                let mut acc = 0.0;
                for sy in 0..3 {
                    for sx in 0..3 {
                        let p = Point::new(
                            x as f32 + (sx as f32 + 0.5) / 3.0,
                            y as f32 + (sy as f32 + 0.5) / 3.0,
                        );
                        acc += inv.map(p).map_or(bg, |q| bit(q.x, q.y));
                    }
                }
                img[y * w + x] = acc / 9.0;
            }
        }
        if self.blur > 0.0 {
            img = gauss(&img, w, h, self.blur);
        }
        let mut rng = Rng(self.seed.max(1));
        let data = img
            .iter()
            .map(|&v| {
                let n = if self.noise > 0.0 {
                    let s: f32 = (0..6).map(|_| (rng.below(1000) as f32) / 1000.0).sum();
                    (s - 3.0) * 2.0 * self.noise / 1.2
                } else {
                    0.0
                };
                (v + n).round().clamp(0.0, 255.0) as u8
            })
            .collect();
        Raster { data, w, h }
    }
}

fn gauss(src: &[f32], w: usize, h: usize, sigma: f32) -> Vec<f32> {
    let r = (3.0 * sigma).ceil() as i32;
    let k: Vec<f32> = (-r..=r)
        .map(|i| (-(i * i) as f32 / (2.0 * sigma * sigma)).exp())
        .collect();
    let norm: f32 = k.iter().sum();
    let mut tmp = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut a = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let xx = (x as i32 + i as i32 - r).clamp(0, w as i32 - 1) as usize;
                a += kv * src[y * w + xx];
            }
            tmp[y * w + x] = a / norm;
        }
    }
    let mut out = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut a = 0.0;
            for (i, kv) in k.iter().enumerate() {
                let yy = (y as i32 + i as i32 - r).clamp(0, h as i32 - 1) as usize;
                a += kv * tmp[yy * w + x];
            }
            out[y * w + x] = a / norm;
        }
    }
    out
}

struct NoBinarizer;

impl Binarizer for NoBinarizer {
    fn id(&self) -> &'static str {
        "none"
    }

    fn binarize(&self, _src: ImageView<'_>, _out: &mut breader_core::BitMatrix) {}
}

fn pass() -> Pass {
    Pass {
        preprocessor: None,
        binarizer: 0,
        inverted: false,
    }
}

/// Reads the raster with the PDF417 reader alone, scanning on its own.
fn read_alone(r: &Raster, effort: Effort) -> Vec<Symbol> {
    let img = ImageView::from_packed(&r.data, r.w, r.h).unwrap();
    let opts = ScanOptions::with_effort(effort);
    let mut scratch = Scratch::new();
    let pre: Vec<Box<dyn Preprocessor>> = Vec::new();
    let bins: Vec<Box<dyn Binarizer>> = vec![Box::new(NoBinarizer)];
    let mut ctx = FrameContext::new(img, &opts, &pre, &bins, &mut scratch);
    ctx.set_pass(pass());
    let mut out = Vec::new();
    Pdf417Reader.scan(&mut ctx, &mut out);
    out
}

/// Reads the raster after the linear reader, which records the marks.
fn read_after_linear(r: &Raster, effort: Effort) -> Vec<Symbol> {
    let img = ImageView::from_packed(&r.data, r.w, r.h).unwrap();
    let opts = ScanOptions::with_effort(effort);
    let mut scratch = Scratch::new();
    let pre: Vec<Box<dyn Preprocessor>> = Vec::new();
    let bins: Vec<Box<dyn Binarizer>> = vec![Box::new(NoBinarizer)];
    let mut ctx = FrameContext::new(img, &opts, &pre, &bins, &mut scratch);
    ctx.set_pass(pass());
    let mut out = Vec::new();
    LinearReader.scan(&mut ctx, &mut out);
    assert!(ctx.shared::<MarkSet>().scanned);
    Pdf417Reader.scan(&mut ctx, &mut out);
    out.retain(|s| s.symbology == Symbology::Pdf417);
    out
}

fn text(len: usize) -> String {
    (0..len)
        .map(|i| (b'A' + (i * 7 % 26) as u8) as char)
        .collect()
}

fn expect(found: &[Symbol], msg: &str) {
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].data, msg.as_bytes());
    assert_eq!(found[0].symbology, Symbology::Pdf417);
}

#[test]
fn clean_symbols_read_alone_and_after_the_linear_reader() {
    for (len, px) in [(30usize, 2.0f32), (120, 3.0), (400, 2.0)] {
        let msg = text(len);
        let r = Scene::new(&msg, EncodeOptions::default(), px).raster();
        expect(&read_alone(&r, Effort::Balanced), &msg);
        expect(&read_after_linear(&r, Effort::Balanced), &msg);
    }
}

#[test]
fn meta_and_quad_describe_the_symbol() {
    let msg = text(60);
    let opts = EncodeOptions {
        level: 3,
        cols: Some(4),
        ..EncodeOptions::default()
    };
    let r = Scene::new(&msg, opts, 3.0).raster();
    let found = read_alone(&r, Effort::Balanced);
    expect(&found, &msg);
    let SymbolMeta::Pdf417(m) = &found[0].meta else {
        panic!("meta");
    };
    assert_eq!((m.cols, m.ec_level, m.compact), (4, 3, false));
    assert_eq!(m.errors_corrected, 0);
    let q = found[0].quad.corners;
    let width = q[0].distance(q[1]);
    let expected = 3.0 * (17.0 * 7.0 + 18.0);
    assert!(
        (width - expected).abs() < 0.08 * expected,
        "{width} vs {expected}"
    );
    assert!(found[0].orientation_deg < 3.0 || found[0].orientation_deg > 357.0);
    assert_eq!(&found[0].aim_id, b"]L2");
}

#[test]
fn compact_symbols_read() {
    let msg = text(70);
    let opts = EncodeOptions {
        compact: true,
        ..EncodeOptions::default()
    };
    let r = Scene::new(&msg, opts, 3.0).raster();
    let found = read_alone(&r, Effort::Balanced);
    expect(&found, &msg);
    let SymbolMeta::Pdf417(m) = &found[0].meta else {
        panic!("meta");
    };
    assert!(m.compact);
}

#[test]
fn rotations_are_read() {
    let msg = text(80);
    for angle in [
        90.0f32, 180.0, 270.0, 15.0, 30.0, 45.0, 100.0, 135.0, 200.0, 300.0,
    ] {
        let mut s = Scene::new(&msg, EncodeOptions::default(), 3.0);
        s.angle_deg = angle;
        let found = read_alone(&s.raster(), Effort::Exhaustive);
        assert_eq!(found.len(), 1, "angle {angle}");
        assert_eq!(found[0].data, msg.as_bytes(), "angle {angle}");
    }
}

#[test]
fn axis_rotations_read_at_fast_effort() {
    let msg = text(80);
    for angle in [0.0f32, 90.0, 180.0, 270.0] {
        let mut s = Scene::new(&msg, EncodeOptions::default(), 3.0);
        s.angle_deg = angle;
        expect(&read_after_linear(&s.raster(), Effort::Fast), &msg);
    }
}

#[test]
fn perspective_is_read() {
    let msg = text(90);
    let mut s = Scene::new(&msg, EncodeOptions::default(), 3.0);
    s.angle_deg = 8.0;
    s.skew = [(0.0, 0.0), (0.0, 10.0), (0.0, -10.0), (0.0, 0.0)];
    expect(&read_alone(&s.raster(), Effort::Balanced), &msg);
    s.skew = [(12.0, 0.0), (-12.0, 6.0), (-8.0, -6.0), (10.0, 0.0)];
    expect(&read_alone(&s.raster(), Effort::Balanced), &msg);
}

#[test]
fn strong_perspective_is_read_between_both_boundaries() {
    let msg = text(120);
    let mut s = Scene::new(&msg, EncodeOptions::default(), 3.0);
    s.angle_deg = 12.0;
    s.skew = [(0.0, 0.0), (0.0, 22.0), (0.0, -22.0), (0.0, 0.0)];
    expect(&read_alone(&s.raster(), Effort::Balanced), &msg);
    expect(&read_alone(&s.raster(), Effort::Exhaustive), &msg);
}

#[test]
fn compact_symbols_with_tilted_rows_are_read() {
    let msg = text(70);
    let opts = EncodeOptions {
        compact: true,
        ..EncodeOptions::default()
    };
    let mut s = Scene::new(&msg, opts, 2.5);
    s.angle_deg = -8.0;
    s.skew = [(0.0, 0.0), (0.0, 8.0), (0.0, -8.0), (0.0, 0.0)];
    expect(&read_alone(&s.raster(), Effort::Balanced), &msg);
}

#[test]
fn compact_symbols_with_converging_rows_are_read() {
    let msg = text(70);
    let opts = EncodeOptions {
        compact: true,
        ..EncodeOptions::default()
    };
    let mut s = Scene::new(&msg, opts, 3.0);
    s.skew = [(0.0, 0.0), (0.0, 16.0), (0.0, -16.0), (0.0, 0.0)];
    expect(&read_alone(&s.raster(), Effort::Balanced), &msg);
}

#[test]
fn symbols_cropped_without_a_quiet_zone_are_read() {
    let msg = text(60);
    let r = Scene::new(&msg, EncodeOptions::default(), 3.0).raster();
    // Cut the image at the symbol's own edges (3 modules of quiet zone at
    // 3 px, plus the 24 px padding of the raster).
    let (x0, y0) = (24 + 9, 24 + 9);
    let (w, h) = (r.w - 2 * x0, r.h - 2 * y0);
    let mut data = Vec::new();
    for y in y0..y0 + h {
        data.extend_from_slice(&r.data[y * r.w + x0..y * r.w + x0 + w]);
    }
    expect(&read_alone(&Raster { data, w, h }, Effort::Balanced), &msg);
}

#[test]
fn rows_below_three_modules_high_are_read() {
    let msg = text(40);
    let mut s = Scene::new(&msg, EncodeOptions::default(), 2.0);
    s.row_h = 1.5;
    expect(&read_alone(&s.raster(), Effort::Balanced), &msg);
}

#[test]
fn blur_and_noise_are_survived() {
    let msg = text(100);
    let mut s = Scene::new(&msg, EncodeOptions::default(), 3.0);
    s.blur = 1.0;
    s.noise = 12.0;
    expect(&read_alone(&s.raster(), Effort::Balanced), &msg);
}

#[test]
fn light_on_dark_is_read() {
    let msg = text(60);
    let mut s = Scene::new(&msg, EncodeOptions::default(), 3.0);
    s.invert = true;
    expect(&read_alone(&s.raster(), Effort::Balanced), &msg);
}

#[test]
fn damage_is_corrected_by_reed_solomon() {
    let msg = text(150);
    let opts = EncodeOptions {
        level: 5,
        ..EncodeOptions::default()
    };
    let mut s = Scene::new(&msg, opts, 3.0);
    s.damage = vec![[0.3, 0.3, 0.45, 0.5], [0.7, 0.6, 0.8, 0.8]];
    let found = read_alone(&s.raster(), Effort::Balanced);
    expect(&found, &msg);
    let SymbolMeta::Pdf417(m) = &found[0].meta else {
        panic!("meta");
    };
    assert!(m.errors_corrected + m.erasures > 0);
}

#[test]
fn too_much_damage_reads_nothing() {
    let msg = text(150);
    let opts = EncodeOptions {
        level: 1,
        ..EncodeOptions::default()
    };
    let mut s = Scene::new(&msg, opts, 3.0);
    s.damage = vec![[0.2, 0.1, 0.6, 0.9]];
    assert!(read_alone(&s.raster(), Effort::Balanced).is_empty());
}

#[test]
fn two_symbols_in_one_frame() {
    let (a, b) = (text(40), "second symbol 123456".to_string());
    let ra = Scene::new(&a, EncodeOptions::default(), 3.0).raster();
    let rb = Scene::new(&b, EncodeOptions::default(), 3.0).raster();
    let w = ra.w.max(rb.w);
    let mut data = vec![225u8; w * (ra.h + rb.h)];
    for y in 0..ra.h {
        data[y * w..y * w + ra.w].copy_from_slice(&ra.data[y * ra.w..(y + 1) * ra.w]);
    }
    for y in 0..rb.h {
        let row = (ra.h + y) * w;
        data[row..row + rb.w].copy_from_slice(&rb.data[y * rb.w..(y + 1) * rb.w]);
    }
    let r = Raster {
        data,
        w,
        h: ra.h + rb.h,
    };
    let mut found = read_alone(&r, Effort::Balanced);
    found.sort_by(|x, y| x.data.cmp(&y.data));
    assert_eq!(found.len(), 2);
    let mut want = [a.into_bytes(), b.into_bytes()];
    want.sort();
    assert_eq!(found[0].data, want[0]);
    assert_eq!(found[1].data, want[1]);
}

#[test]
fn reader_initialisation_symbols_are_rejected() {
    // 921 as the first data codeword.
    let msg = "\u{0}";
    let _ = msg;
    let data = [921u16, 900, 5, 6, 7];
    let (layout, cells) = crate::testutil::seal(&data, 2, 3);
    let dec = crate::decode::decode_codewords(&cells, layout).unwrap();
    assert!(dec.reader_init);
    let boundary = Boundary {
        kind: MarkKind::Start,
        inverted: false,
        line: crate::geom::Line {
            origin: Point::new(0.0, 0.0),
            dir: (0.0, 1.0),
        },
        rows: (1.0, 0.0),
        lo: 0.0,
        hi: 10.0,
        module: 2.0,
    };
    let found = Found {
        solved: Solved {
            decoded: dec,
            compact: false,
            fit: None,
        },
        frame: Frame::Straight {
            boundary,
            rows: (1.0, 0.0),
            fan: 0.0,
            width: 100.0,
        },
        span: (0.0, 10.0),
    };
    assert!(build_symbol(&found, (1.0, 1.0)).is_none());
}

#[test]
fn nothing_in_stripes_and_noise() {
    let mut rng = Rng(9);
    for trial in 0..6 {
        let (w, h) = (300usize, 120usize);
        let mut data = vec![230u8; w * h];
        match trial % 3 {
            0 => {
                // Random vertical stripes of 1 to 6 pixels.
                let mut x = 0;
                let mut dark = false;
                while x < w {
                    let run = 1 + rng.below(6);
                    for xx in x..(x + run).min(w) {
                        for y in 0..h {
                            data[y * w + xx] = if dark { 20 } else { 230 };
                        }
                    }
                    x += run;
                    dark = !dark;
                }
            }
            1 => {
                for v in data.iter_mut() {
                    *v = rng.below(256) as u8;
                }
            }
            _ => {
                // Start patterns alone, repeated down the image.
                let widths = [8, 1, 1, 1, 1, 1, 1, 3];
                for y in 0..h {
                    let mut x = 20;
                    for (i, &e) in widths.iter().enumerate() {
                        if i % 2 == 0 {
                            data[y * w + x..y * w + x + 3 * e].fill(20);
                        }
                        x += 3 * e;
                    }
                    data[y * w + x..y * w + x + 6].fill(20);
                }
            }
        }
        let r = Raster { data, w, h };
        assert!(
            read_alone(&r, Effort::Exhaustive).is_empty(),
            "trial {trial}"
        );
    }
}
