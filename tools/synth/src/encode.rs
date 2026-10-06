use barcoders::sym::code39::Code39;
use barcoders::sym::code128::Code128;
use barcoders::sym::ean8::EAN8;
use barcoders::sym::ean13::EAN13;
use barcoders::sym::tf::TF;
use qrcode::QrCode;
use qrcode::types::{Color, EcLevel, Version};
use rand::Rng;
use rand_chacha::ChaCha8Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sym {
    Qr,
    Ean13,
    Ean8,
    UpcA,
    Code128,
    Code39,
    Itf,
}

impl Sym {
    pub const ALL: [Sym; 7] = [
        Sym::Qr,
        Sym::Ean13,
        Sym::Ean8,
        Sym::UpcA,
        Sym::Code128,
        Sym::Code39,
        Sym::Itf,
    ];

    /// Deterministic rotation that favours QR (the most varied symbology).
    pub const CYCLE: [Sym; 9] = [
        Sym::Qr,
        Sym::Ean13,
        Sym::Qr,
        Sym::Ean8,
        Sym::UpcA,
        Sym::Code128,
        Sym::Qr,
        Sym::Code39,
        Sym::Itf,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Sym::Qr => "QR_CODE",
            Sym::Ean13 => "EAN_13",
            Sym::Ean8 => "EAN_8",
            Sym::UpcA => "UPC_A",
            Sym::Code128 => "CODE_128",
            Sym::Code39 => "CODE_39",
            Sym::Itf => "ITF",
        }
    }
}

/// A symbol as a dark/light module grid plus its nominal physical layout.
#[derive(Debug, Clone)]
pub struct Encoded {
    pub text: String,
    pub format: &'static str,
    pub modules: Vec<bool>,
    pub cols: usize,
    pub rows: usize,
    /// Height of one grid row in module widths.
    pub cell_h: f64,
    pub qz_x: f64,
    pub qz_y: f64,
}

impl Encoded {
    pub fn linear(text: String, format: &'static str, bars: &[u8], height: f64) -> Self {
        Self {
            text,
            format,
            modules: bars.iter().map(|&b| b != 0).collect(),
            cols: bars.len(),
            rows: 1,
            cell_h: height,
            qz_x: 10.0,
            qz_y: 6.0,
        }
    }

    pub fn matrix(text: String, format: &'static str, modules: Vec<bool>, n: usize) -> Self {
        Self {
            text,
            format,
            modules,
            cols: n,
            rows: n,
            cell_h: 1.0,
            qz_x: 4.0,
            qz_y: 4.0,
        }
    }
}

pub fn gtin_check(digits: &[u8]) -> u8 {
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| u32::from(d) * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    ((10 - sum % 10) % 10) as u8
}

fn digits(rng: &mut ChaCha8Rng, n: usize) -> Vec<u8> {
    (0..n).map(|_| rng.random_range(0..10u8)).collect()
}

fn to_text(d: &[u8]) -> String {
    d.iter().map(|&x| char::from(b'0' + x)).collect()
}

fn with_check(mut d: Vec<u8>) -> Vec<u8> {
    d.push(gtin_check(&d));
    d
}

fn pick_string(rng: &mut ChaCha8Rng, pool: &[char], len: usize) -> String {
    (0..len)
        .map(|_| pool[rng.random_range(0..pool.len())])
        .collect()
}

fn pool(s: &str) -> Vec<char> {
    s.chars().collect()
}

const ALNUM_QR: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";
const BYTE_ASCII: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 .,:;/?-_=&#@!";
const BYTE_UTF8: &str = "abcdefghijklmnopqrstuvwxyz éüöñçàß€äèí日本語中文";

fn qr_payload(rng: &mut ChaCha8Rng, len: usize) -> String {
    match rng.random_range(0..4u8) {
        0 => to_text(&digits(rng, len)),
        1 => pick_string(rng, &pool(ALNUM_QR), len),
        2 => {
            let mut s = pick_string(rng, &pool(BYTE_ASCII), len);
            if s.trim() != s {
                s = format!("Q{}", s.trim());
            }
            s
        }
        _ => {
            let mut s = pick_string(rng, &pool(BYTE_UTF8), len.div_ceil(2).max(2));
            s.insert(0, 'é');
            s.trim().to_string()
        }
    }
}

fn encode_qr(rng: &mut ChaCha8Rng) -> Encoded {
    let version = rng.random_range(1..=10u8);
    let ec = [EcLevel::L, EcLevel::M, EcLevel::Q, EcLevel::H][rng.random_range(0..4)];
    let mut len = rng.random_range(4..=(usize::from(version) * 10 + 8));
    loop {
        let text = qr_payload(rng, len);
        if !text.is_empty()
            && !text.starts_with(char::is_whitespace)
            && let Ok(code) =
                QrCode::with_version(text.as_bytes(), Version::Normal(i16::from(version)), ec)
        {
            let n = code.width();
            let modules = code
                .to_colors()
                .into_iter()
                .map(|c| c == Color::Dark)
                .collect();
            return Encoded::matrix(text, Sym::Qr.label(), modules, n);
        }
        len = (len * 4 / 5).max(1);
    }
}

fn linear_height(rng: &mut ChaCha8Rng, cols: usize) -> f64 {
    let ratio = rng.random_range(0.2..0.45);
    (cols as f64 * ratio).max(30.0)
}

fn encode_linear(rng: &mut ChaCha8Rng, sym: Sym) -> Encoded {
    let fail = "encoder rejected generated payload";
    let (text, bars) = match sym {
        Sym::Ean13 => {
            let d = with_check(digits(rng, 12));
            let t = to_text(&d);
            (t.clone(), EAN13::new(&t).expect(fail).encode())
        }
        Sym::UpcA => {
            let d = with_check(digits(rng, 11));
            let t = to_text(&d);
            let bars = EAN13::new(format!("0{t}")).expect(fail).encode();
            (t, bars)
        }
        Sym::Ean8 => {
            let d = with_check(digits(rng, 7));
            let t = to_text(&d);
            (t.clone(), EAN8::new(&t).expect(fail).encode())
        }
        Sym::Code128 => {
            if rng.random_bool(0.25) {
                let n = 2 * rng.random_range(4..=8);
                let t = to_text(&digits(rng, n));
                let bars = Code128::new(format!("\u{0106}{t}")).expect(fail).encode();
                (t, bars)
            } else {
                let len = rng.random_range(6..=14);
                let t = pick_string(
                    rng,
                    &pool("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-./:"),
                    len,
                );
                let bars = Code128::new(format!("\u{0181}{t}")).expect(fail).encode();
                (t, bars)
            }
        }
        Sym::Code39 => {
            let len = rng.random_range(4..=12);
            let t = pick_string(rng, &pool("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-."), len);
            let bars = Code39::new(&t).expect(fail).encode();
            (t, bars)
        }
        Sym::Itf => {
            let d = with_check(digits(rng, 13));
            let t = to_text(&d);
            (t.clone(), TF::interleaved(&t).expect(fail).encode())
        }
        Sym::Qr => unreachable!("QR is not linear"),
    };
    let height = linear_height(rng, bars.len());
    Encoded::linear(text, sym.label(), &bars, height)
}

pub fn encode(rng: &mut ChaCha8Rng, sym: Sym) -> Encoded {
    match sym {
        Sym::Qr => encode_qr(rng),
        _ => encode_linear(rng, sym),
    }
}

#[cfg(test)]
mod tests {
    use rand::SeedableRng;

    use super::*;

    #[test]
    fn gtin_check_known_values() {
        assert_eq!(gtin_check(&[4, 0, 0, 6, 3, 8, 1, 3, 3, 3, 9, 3]), 1);
        assert_eq!(gtin_check(&[9, 6, 3, 8, 5, 0, 7]), 4);
    }

    #[test]
    fn module_counts() {
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        assert_eq!(encode(&mut rng, Sym::Ean13).cols, 95);
        assert_eq!(encode(&mut rng, Sym::UpcA).cols, 95);
        assert_eq!(encode(&mut rng, Sym::Ean8).cols, 67);
        let upc = encode(&mut rng, Sym::UpcA);
        assert_eq!(upc.text.len(), 12);
        let itf = encode(&mut rng, Sym::Itf);
        assert_eq!(itf.text.len(), 14);
    }

    #[test]
    fn all_symbologies_encode_many_times() {
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        for _ in 0..200 {
            for s in Sym::ALL {
                let e = encode(&mut rng, s);
                assert_eq!(e.modules.len(), e.cols * e.rows);
                assert!(!e.text.is_empty());
            }
        }
    }
}
