//! Synthetic symbol rendering shared by the unit tests.

use breader_core::ImageBuffer;

/// L-code patterns, bit 1 = bar, as printed in the symbology specification.
const L_CODE: [&str; 10] = [
    "0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011",
    "0110111", "0001011",
];

const EAN13_PARITY: [&str; 10] = [
    "LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL",
    "LGGLGL",
];

const UPCE_PARITY_NS0: [&str; 10] = [
    "GGGLLL", "GGLGLL", "GGLLGL", "GGLLLG", "GLGGLL", "GLLGGL", "GLLLGG", "GLGLGL", "GLGLLG",
    "GLLGLG",
];

const EAN5_PARITY: [&str; 10] = [
    "GGLLL", "GLGLL", "GLLGL", "GLLLG", "LGGLL", "LLGGL", "LLLGG", "LGLGL", "LGLLG", "LLGLG",
];

fn bits(s: &str) -> Vec<bool> {
    s.bytes().map(|b| b == b'1').collect()
}

fn l_code(d: u8) -> Vec<bool> {
    bits(L_CODE[d as usize])
}

fn g_code(d: u8) -> Vec<bool> {
    l_code(d).iter().rev().map(|b| !b).collect()
}

fn r_code(d: u8) -> Vec<bool> {
    l_code(d).iter().map(|b| !b).collect()
}

fn digits(s: &str) -> Vec<u8> {
    s.bytes().map(|b| b - b'0').collect()
}

fn coded(d: u8, parity: u8) -> Vec<bool> {
    if parity == b'G' || parity == b'E' {
        g_code(d)
    } else {
        l_code(d)
    }
}

pub fn ean13_modules(text: &str) -> Vec<bool> {
    let d = digits(text);
    assert_eq!(d.len(), 13);
    let parity = EAN13_PARITY[d[0] as usize].as_bytes();
    let mut m = bits("101");
    for i in 0..6 {
        m.extend(coded(d[1 + i], parity[i]));
    }
    m.extend(bits("01010"));
    for &x in &d[7..] {
        m.extend(r_code(x));
    }
    m.extend(bits("101"));
    m
}

pub fn upca_modules(text: &str) -> Vec<bool> {
    assert_eq!(text.len(), 12);
    ean13_modules(&format!("0{text}"))
}

pub fn ean8_modules(text: &str) -> Vec<bool> {
    let d = digits(text);
    assert_eq!(d.len(), 8);
    let mut m = bits("101");
    for &x in &d[..4] {
        m.extend(l_code(x));
    }
    m.extend(bits("01010"));
    for &x in &d[4..] {
        m.extend(r_code(x));
    }
    m.extend(bits("101"));
    m
}

/// `text` is the 8-digit form: number system, six digits, check digit.
pub fn upce_modules(text: &str) -> Vec<bool> {
    let d = digits(text);
    assert_eq!(d.len(), 8);
    let base = UPCE_PARITY_NS0[d[7] as usize].as_bytes();
    let mut m = bits("101");
    for i in 0..6 {
        let g = base[i] == b'G';
        let g = if d[0] == 0 { g } else { !g };
        m.extend(coded(d[1 + i], if g { b'G' } else { b'L' }));
    }
    m.extend(bits("010101"));
    m
}

/// EAN-2 or EAN-5 supplement including its start guard, without the gap.
pub fn addon_modules(text: &str) -> Vec<bool> {
    let d = digits(text);
    let parity = match d.len() {
        2 => {
            let v = u32::from(d[0]) * 10 + u32::from(d[1]);
            ["LL", "LG", "GL", "GG"][(v % 4) as usize]
        }
        5 => {
            let sum = 3 * u32::from(d[0] + d[2] + d[4]) + 9 * u32::from(d[1] + d[3]);
            EAN5_PARITY[(sum % 10) as usize]
        }
        n => panic!("bad add-on length {n}"),
    };
    addon_modules_with_parity(text, parity)
}

/// Supplement with an explicit L/G pattern, possibly an invalid one.
pub fn addon_modules_with_parity(text: &str, parity: &str) -> Vec<bool> {
    let d = digits(text);
    assert_eq!(d.len(), parity.len());
    let mut m = bits("1011");
    for (i, &x) in d.iter().enumerate() {
        if i > 0 {
            m.extend(bits("01"));
        }
        m.extend(coded(x, parity.as_bytes()[i]));
    }
    m
}

pub fn with_addon(mut main: Vec<bool>, gap_modules: usize, addon: Vec<bool>) -> Vec<bool> {
    main.extend(std::iter::repeat_n(false, gap_modules));
    main.extend(addon);
    main
}

pub fn check_digit(body: &str) -> u8 {
    let d = digits(body);
    let sum: u32 = d
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &x)| u32::from(x) * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    ((10 - sum % 10) % 10) as u8
}

#[derive(Debug, Clone, Copy)]
pub struct Rng(pub u64);

impl Rng {
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub fn unit(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    pub fn gauss(&mut self) -> f32 {
        let s: f32 = (0..12).map(|_| self.unit()).sum();
        s - 6.0
    }

    pub fn digits(&mut self, n: usize) -> String {
        (0..n)
            .map(|_| char::from(b'0' + self.below(10) as u8))
            .collect()
    }
}

fn erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let poly = t
        * (0.254_829_6
            + t * (-0.284_496_74 + t * (1.421_413_8 + t * (-1.453_152_1 + t * 1.061_405_4))));
    (1.0 - poly * (-x * x).exp()).copysign(x)
}

fn phi(x: f32) -> f32 {
    0.5 * (1.0 + erf(x / std::f32::consts::SQRT_2))
}

/// Dark intervals blurred by a Gaussian and integrated over each pixel.
pub fn render_intervals(
    dark: &[(f32, f32)],
    len: usize,
    dark_level: f32,
    light_level: f32,
    sigma: f32,
) -> Vec<u8> {
    const SUB: usize = 8;
    (0..len)
        .map(|i| {
            let mut cover = 0.0;
            for k in 0..SUB {
                let t = i as f32 - 0.5 + (k as f32 + 0.5) / SUB as f32;
                for &(a, b) in dark {
                    cover += if sigma < 1e-3 {
                        f32::from(a <= t && t < b)
                    } else if t < a - 4.0 * sigma || t > b + 4.0 * sigma {
                        0.0
                    } else {
                        phi((t - a) / sigma) - phi((t - b) / sigma)
                    };
                }
            }
            let v = light_level - (light_level - dark_level) * cover / SUB as f32;
            v.round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub struct Render {
    pub px: f32,
    pub quiet_left: f32,
    pub quiet_right: f32,
    pub offset: f32,
    pub sigma: f32,
    /// Added to each side of every bar, in pixels (print gain).
    pub gain: f32,
    pub dark: f32,
    pub light: f32,
}

impl Default for Render {
    fn default() -> Self {
        Self {
            px: 3.0,
            quiet_left: 12.0,
            quiet_right: 12.0,
            offset: 0.0,
            sigma: 0.6,
            gain: 0.0,
            dark: 25.0,
            light: 225.0,
        }
    }
}

impl Render {
    pub fn px(px: f32) -> Self {
        Self {
            px,
            ..Self::default()
        }
    }

    pub fn profile(&self, modules: &[bool]) -> Vec<u8> {
        let left = self.quiet_left * self.px + self.offset;
        let mut dark = Vec::new();
        let mut i = 0;
        while i < modules.len() {
            if modules[i] {
                let start = i;
                while i < modules.len() && modules[i] {
                    i += 1;
                }
                dark.push((
                    left + start as f32 * self.px - self.gain,
                    left + i as f32 * self.px + self.gain,
                ));
            } else {
                i += 1;
            }
        }
        let len = (left + modules.len() as f32 * self.px + self.quiet_right * self.px).ceil()
            as usize
            + 1;
        render_intervals(&dark, len, self.dark, self.light, self.sigma)
    }
}

pub fn add_noise(profile: &mut [u8], sigma: f32, rng: &mut Rng) {
    for v in profile {
        *v = (f32::from(*v) + sigma * rng.gauss())
            .round()
            .clamp(0.0, 255.0) as u8;
    }
}

/// Placement of a symbol in a synthetic image: `module_px` per module, bars
/// `height_px` tall, centered at `center` and turned clockwise by `angle_deg`.
#[derive(Debug, Clone, Copy)]
pub struct Placement {
    pub module_px: f32,
    pub height_px: f32,
    pub center: (f32, f32),
    pub angle_deg: f32,
}

/// Draws `modules` (true = bar) with 3x3 supersampling.
pub fn draw_symbol(img: &mut ImageBuffer, modules: &[bool], place: Placement, dark: u8) {
    let (w, h) = (img.width(), img.height());
    let (s, c) = place.angle_deg.to_radians().sin_cos();
    let half_len = modules.len() as f32 * place.module_px / 2.0;
    let half_h = place.height_px / 2.0;
    for y in 0..h {
        for x in 0..w {
            let mut covered = 0;
            for sy in 0..3 {
                for sx in 0..3 {
                    let px = x as f32 + (sx as f32 + 0.5) / 3.0 - 0.5 - place.center.0;
                    let py = y as f32 + (sy as f32 + 0.5) / 3.0 - 0.5 - place.center.1;
                    let u = px * c + py * s + half_len;
                    let v = -px * s + py * c;
                    if u >= 0.0 && v.abs() <= half_h {
                        let idx = (u / place.module_px) as usize;
                        covered += usize::from(idx < modules.len() && modules[idx]);
                    }
                }
            }
            if covered > 0 {
                let a = covered as f32 / 9.0;
                let base = f32::from(img.row_mut(y)[x]);
                let v = base + (f32::from(dark) - base) * a;
                img.row_mut(y)[x] = v.round() as u8;
            }
        }
    }
}

pub fn blank(width: usize, height: usize, light: u8) -> ImageBuffer {
    let mut img = ImageBuffer::new(width, height);
    img.as_mut_slice().fill(light);
    img
}

pub fn invert_image(img: &mut ImageBuffer) {
    for v in img.as_mut_slice() {
        *v = 255 - *v;
    }
}
