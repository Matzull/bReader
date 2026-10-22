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

/// Modules (true = bar) of a run-length string such as "211412": digits are
/// widths, starting with a bar.
pub fn runs_to_modules(widths: &[usize]) -> Vec<bool> {
    let mut m = Vec::new();
    for (i, &w) in widths.iter().enumerate() {
        m.extend(std::iter::repeat_n(i % 2 == 0, w));
    }
    m
}

fn digit_widths(s: &str) -> Vec<usize> {
    s.bytes().map(|b| usize::from(b - b'0')).collect()
}

/// Code 128 symbol for `values` (start code first) with the mod-103 check
/// appended, stop and termination bar included.
pub fn code128_modules(values: &[u8]) -> Vec<bool> {
    let mut all = values.to_vec();
    all.push(code128_check(values));
    code128_modules_raw(&all)
}

pub fn code128_check(values: &[u8]) -> u8 {
    let sum: u32 = values
        .iter()
        .enumerate()
        .map(|(i, &v)| u32::from(v) * (i.max(1) as u32))
        .sum();
    (sum % 103) as u8
}

/// Like [`code128_modules`] but `values` already contains the check character.
pub fn code128_modules_raw(values: &[u8]) -> Vec<bool> {
    let mut widths = Vec::new();
    for &v in values {
        widths.extend(digit_widths(crate::code128::PATTERNS[usize::from(v)]));
    }
    widths.extend(digit_widths(crate::code128::PATTERNS[106]));
    widths.push(2);
    runs_to_modules(&widths)
}

/// Code 128 character value of an ASCII character in set B.
pub fn b(c: char) -> u8 {
    c as u8 - 32
}

/// Narrow/wide strings of Code 39, bar first, transcribed from the symbology
/// specification (independent of the table the decoder builds).
pub const CODE39_TABLE: [(char, &str); 44] = [
    ('0', "nnnwwnwnn"),
    ('1', "wnnwnnnnw"),
    ('2', "nnwwnnnnw"),
    ('3', "wnwwnnnnn"),
    ('4', "nnnwwnnnw"),
    ('5', "wnnwwnnnn"),
    ('6', "nnwwwnnnn"),
    ('7', "nnnwnnwnw"),
    ('8', "wnnwnnwnn"),
    ('9', "nnwwnnwnn"),
    ('A', "wnnnnwnnw"),
    ('B', "nnwnnwnnw"),
    ('C', "wnwnnwnnn"),
    ('D', "nnnnwwnnw"),
    ('E', "wnnnwwnnn"),
    ('F', "nnwnwwnnn"),
    ('G', "nnnnnwwnw"),
    ('H', "wnnnnwwnn"),
    ('I', "nnwnnwwnn"),
    ('J', "nnnnwwwnn"),
    ('K', "wnnnnnnww"),
    ('L', "nnwnnnnww"),
    ('M', "wnwnnnnwn"),
    ('N', "nnnnwnnww"),
    ('O', "wnnnwnnwn"),
    ('P', "nnwnwnnwn"),
    ('Q', "nnnnnnwww"),
    ('R', "wnnnnnwwn"),
    ('S', "nnwnnnwwn"),
    ('T', "nnnnwnwwn"),
    ('U', "wwnnnnnnw"),
    ('V', "nwwnnnnnw"),
    ('W', "wwwnnnnnn"),
    ('X', "nwnnwnnnw"),
    ('Y', "wwnnwnnnn"),
    ('Z', "nwwnwnnnn"),
    ('-', "nwnnnnwnw"),
    ('.', "wwnnnnwnn"),
    (' ', "nwwnnnwnn"),
    ('*', "nwnnwnwnn"),
    ('$', "nwnwnwnnn"),
    ('/', "nwnwnnnwn"),
    ('+', "nwnnnwnwn"),
    ('%', "nnnwnwnwn"),
];

/// Code 39 symbol of `body` between start and stop characters. Wide elements
/// are `wide` modules, narrow ones one module, characters are `gap` apart.
pub fn code39_modules(body: &str, wide: usize, gap: usize) -> Vec<bool> {
    let mut widths = Vec::new();
    let text = format!("*{body}*");
    for (i, ch) in text.chars().enumerate() {
        if i > 0 {
            widths.push(gap);
        }
        let (_, p) = CODE39_TABLE.iter().find(|(c, _)| *c == ch).unwrap();
        widths.extend(p.bytes().map(|b| if b == b'w' { wide } else { 1 }));
    }
    runs_to_modules(&widths)
}

/// Codabar patterns as in the specification, `w` for wide elements.
pub const CODABAR_TABLE: [(char, &str); 20] = [
    ('0', "nnnnnww"),
    ('1', "nnnnwwn"),
    ('2', "nnnwnnw"),
    ('3', "wwnnnnn"),
    ('4', "nnwnnwn"),
    ('5', "wnnnnwn"),
    ('6', "nwnnnnw"),
    ('7', "nwnnwnn"),
    ('8', "nwwnnnn"),
    ('9', "wnnwnnn"),
    ('-', "nnnwwnn"),
    ('$', "nnwwnnn"),
    (':', "wnnnwnw"),
    ('/', "wnwnnnw"),
    ('.', "wnwnwnn"),
    ('+', "nnwnwnw"),
    ('A', "nnwwnwn"),
    ('B', "nnnwnww"),
    ('C', "nwnwnnw"),
    ('D', "nnnwwwn"),
];

/// Codabar symbol of `body` between the `start` and `stop` characters. Wide
/// elements are `wide` modules, narrow ones one module, characters are `gap`
/// modules apart.
pub fn codabar_modules(start: char, body: &str, stop: char, wide: usize, gap: usize) -> Vec<bool> {
    let mut widths = Vec::new();
    let text = format!("{start}{body}{stop}");
    for (i, ch) in text.chars().enumerate() {
        if i > 0 {
            widths.push(gap);
        }
        let (_, p) = CODABAR_TABLE.iter().find(|(c, _)| *c == ch).unwrap();
        widths.extend(p.bytes().map(|b| if b == b'w' { wide } else { 1 }));
    }
    runs_to_modules(&widths)
}

/// Code 93 characters as module strings (1 = bar), by value 0 to 46 (the 43
/// basic characters in check order, then the shift characters ($), (%), (/)
/// and (+)) and the start/stop character `*` last, as printed in the
/// specification (independent of the width table the decoder builds).
pub const CODE93_BITS: [&str; 48] = [
    "100010100",
    "101001000",
    "101000100",
    "101000010",
    "100101000",
    "100100100",
    "100100010",
    "101010000",
    "100010010",
    "100001010",
    "110101000",
    "110100100",
    "110100010",
    "110010100",
    "110010010",
    "110001010",
    "101101000",
    "101100100",
    "101100010",
    "100110100",
    "100011010",
    "101011000",
    "101001100",
    "101000110",
    "100101100",
    "100010110",
    "110110100",
    "110110010",
    "110101100",
    "110100110",
    "110010110",
    "110011010",
    "101101100",
    "101100110",
    "100110110",
    "100111010",
    "100101110",
    "111010100",
    "111010010",
    "111001010",
    "101101110",
    "101110110",
    "110101110",
    "100100110",
    "111011010",
    "111010110",
    "100110010",
    "101011110",
];

/// The check characters C and K of the data `values` (modulo 47, weights
/// from the left written out for the length at hand).
pub fn code93_checks(values: &[u8]) -> (u8, u8) {
    let weighted = |v: &[u8], period: usize| -> u8 {
        let n = v.len();
        let sum: usize = v
            .iter()
            .enumerate()
            .map(|(i, &x)| usize::from(x) * ((n - 1 - i) % period + 1))
            .sum();
        (sum % 47) as u8
    };
    let c = weighted(values, 20);
    let mut with_c = values.to_vec();
    with_c.push(c);
    (c, weighted(&with_c, 15))
}

/// Code 93 symbol of exactly `values` between start and stop, with the
/// termination bar and no check characters added.
pub fn code93_modules_raw(values: &[u8]) -> Vec<bool> {
    let mut m = Vec::new();
    for &v in std::iter::once(&47u8).chain(values).chain(&[47u8]) {
        m.extend(CODE93_BITS[usize::from(v)].bytes().map(|b| b == b'1'));
    }
    m.push(true);
    m
}

/// Code 93 symbol of the data `values` with C and K appended.
pub fn code93_modules(values: &[u8]) -> Vec<bool> {
    let (c, k) = code93_checks(values);
    let mut all = values.to_vec();
    all.extend([c, k]);
    code93_modules_raw(&all)
}

/// Values of the Full ASCII text `text`: the 43 basic characters stay
/// themselves (also `$ % + /`), the others become a shift character and a letter, as in the
/// specification's table of the 128 characters.
pub fn code93_ascii_values(text: &[u8]) -> Vec<u8> {
    const BASIC: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";
    let value = |c: u8| BASIC.iter().position(|&b| b == c).unwrap() as u8;
    let (dollar, percent, slash, plus) = (43u8, 44u8, 45u8, 46u8);
    let mut out = Vec::new();
    for &c in text {
        let (shift, letter): (Option<u8>, u8) = match c {
            _ if BASIC.contains(&c) => (None, c),
            0 => (Some(percent), b'U'),
            1..=26 => (Some(dollar), b'A' + c - 1),
            27..=31 => (Some(percent), b'A' + c - 27),
            33..=44 => (Some(slash), b'A' + c - 33),
            47 => (Some(slash), b'O'),
            58 => (Some(slash), b'Z'),
            59..=63 => (Some(percent), b'F' + c - 59),
            64 => (Some(percent), b'V'),
            91..=95 => (Some(percent), b'K' + c - 91),
            96 => (Some(percent), b'W'),
            97..=122 => (Some(plus), b'A' + c - 97),
            123..=126 => (Some(percent), b'P' + c - 123),
            127 => (Some(percent), b'T'),
            _ => (None, c),
        };
        if let Some(s) = shift {
            out.push(s);
        }
        out.push(value(letter));
    }
    out
}

const ITF_DIGITS: [&str; 10] = [
    "nnwwn", "wnnnw", "nwnnw", "wwnnn", "nnwnw", "wnwnn", "nwwnn", "nnnww", "wnnwn", "nwnwn",
];

/// Interleaved 2 of 5 symbol with start and stop; wide elements are `wide`
/// modules.
pub fn itf_modules(digits: &str, wide: usize) -> Vec<bool> {
    assert!(digits.len().is_multiple_of(2));
    let w = |c: u8| if c == b'w' { wide } else { 1 };
    let mut widths = vec![1, 1, 1, 1];
    let d = digits.as_bytes();
    for pair in d.chunks(2) {
        let a = ITF_DIGITS[usize::from(pair[0] - b'0')].as_bytes();
        let s = ITF_DIGITS[usize::from(pair[1] - b'0')].as_bytes();
        for i in 0..5 {
            widths.push(w(a[i]));
            widths.push(w(s[i]));
        }
    }
    widths.extend([wide, 1, 1]);
    runs_to_modules(&widths)
}

/// `modules` with a block of bars `left` and `right` spaces away from it
/// (0: no block on that side). Gives a quiet zone limited by content.
pub fn with_neighbours(modules: &[bool], left: usize, right: usize) -> Vec<bool> {
    const BLOCK: usize = 12;
    let mut out = Vec::new();
    if left > 0 {
        out.extend([true; BLOCK]);
        out.extend(vec![false; left]);
    }
    out.extend_from_slice(modules);
    if right > 0 {
        out.extend(vec![false; right]);
        out.extend([true; BLOCK]);
    }
    out
}

/// Cuts a rendered profile, symmetric in quiet zone, `quiet_px` samples at
/// each end: from the left and from the right, every sample deeper into the
/// symbol. Returns the depth with the cut profile. Models a symbol that the
/// image border or the end of a scanline truncates.
pub fn border_cuts(profile: &[u8], quiet_px: f32, symbol_px: f32) -> Vec<(usize, Vec<u8>)> {
    let quiet = quiet_px as usize;
    let mut out = Vec::new();
    for depth in 1..symbol_px as usize {
        out.push((depth, profile[quiet + depth..].to_vec()));
        out.push((depth, profile[..profile.len() - quiet - depth].to_vec()));
    }
    out
}

/// ITF-14 payload: 13 digits plus the GS1 check digit.
pub fn with_gs1_check(body: &str) -> String {
    format!("{body}{}", check_digit(body))
}

/// Edge detection and decoding as the reader does it: both reading
/// directions, both polarities. Hits of light-on-dark have `inverted` set.
/// Rows of stacked symbols (`part`) are not symbols and are left out.
pub fn decode_all<D: crate::decoder::LinearDecoder>(
    decoder: &D,
    profile: &[u8],
    opts: &crate::decoder::LineOptions,
) -> Vec<crate::decoder::LineHit> {
    let mut hits = decode_all_parts(decoder, profile, opts);
    hits.retain(|h| !h.part && !h.text.is_empty());
    hits
}

/// [`decode_all`] with the rows of stacked symbols.
pub fn decode_all_parts<D: crate::decoder::LinearDecoder>(
    decoder: &D,
    profile: &[u8],
    opts: &crate::decoder::LineOptions,
) -> Vec<crate::decoder::LineHit> {
    use crate::edges::{EdgeDetector, EdgeParams, Edges};
    use crate::runs::Runs;
    let mut edges = Edges::default();
    EdgeDetector::default().detect(profile, &EdgeParams::fine(), &mut edges);
    let mut runs = Runs::default();
    Runs::from_edges(&edges, &mut runs);
    let mut rev = Runs::default();
    Runs::reverse_from(&runs, &mut rev);
    let mut out = Vec::new();
    decoder.decode(&runs, opts, &mut out);
    decoder.decode(&rev, opts, &mut out);
    let dark = out.len();
    runs.flip_roles();
    rev.flip_roles();
    decoder.decode(&runs, opts, &mut out);
    decoder.decode(&rev, opts, &mut out);
    for h in &mut out[dark..] {
        h.inverted = true;
    }
    out
}

pub fn line_options(f: impl FnOnce(&mut breader_core::ScanOptions)) -> crate::decoder::LineOptions {
    let mut o = breader_core::ScanOptions::default();
    f(&mut o);
    crate::decoder::LineOptions::from(&o)
}

fn databar_modules(encoding: anyd::output::Encoding) -> Vec<bool> {
    match encoding {
        anyd::output::Encoding::Linear(p) => p.modules,
        _ => panic!("linear pattern expected"),
    }
}

/// GS1 DataBar Omnidirectional of a 13-digit GTIN body, from the
/// independent encoder: 96 modules, the first one the guard's space.
pub fn databar_omni_modules(body: &str) -> Vec<bool> {
    use anyd::codes::databar::DataBarEncoder;
    use anyd::traits::Encode;
    let enc = DataBarEncoder::new();
    databar_modules(
        enc.encode(&enc.build_omni(body.as_bytes()).unwrap())
            .unwrap(),
    )
}

/// GS1 DataBar Expanded of a GS1 element string (FNC1 as byte 0x1D) in
/// one row.
pub fn databar_expanded_modules(elements: &str) -> Vec<bool> {
    use anyd::codes::databar::DataBarEncoder;
    use anyd::traits::Encode;
    let enc = DataBarEncoder::new();
    databar_modules(
        enc.encode(&enc.build_expanded(elements.as_bytes()).unwrap())
            .unwrap(),
    )
}

/// GS1 DataBar Limited of a 13-digit GTIN body (first digit 0 or 1):
/// 74 modules and five of quiet zone.
pub fn databar_limited_modules(body: &str) -> Vec<bool> {
    use anyd::codes::databar::DataBarEncoder;
    use anyd::traits::Encode;
    let enc = DataBarEncoder::new();
    databar_modules(
        enc.encode(&enc.build_limited(body.as_bytes()).unwrap())
            .unwrap(),
    )
}

/// Code 93 symbol from the independent encoder `anyd`: start, `text` (Full
/// ASCII through the shift characters when `full_ascii`), C, K, stop and the
/// termination bar.
pub fn code93_oracle_modules(text: &[u8], full_ascii: bool) -> Vec<bool> {
    use anyd::codes::code93::Code93Encoder;
    use anyd::output::Encoding;
    use anyd::traits::Encode;
    let enc = Code93Encoder::new();
    let symbol = enc.build(text, full_ascii).expect("encodable");
    match enc.encode(&symbol).expect("encoded") {
        Encoding::Linear(p) => p.modules,
        _ => panic!("linear pattern expected"),
    }
}
