use barcoders::sym::code39::Code39;
use barcoders::sym::code128::Code128;
use barcoders::sym::ean8::EAN8;
use barcoders::sym::ean13::EAN13;
use barcoders::sym::tf::TF;
use datamatrix::{DataMatrix, SymbolList, SymbolSize};
use qrcode::QrCode;
use qrcode::types::{Color, EcLevel, Version};
use rand::Rng;
use rand_chacha::ChaCha8Rng;
use rxing::aztec::encoder::aztec_encoder;
use rxing::pdf417::encoder::PDF417;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sym {
    Qr,
    Ean13,
    Ean8,
    UpcA,
    Code128,
    Code39,
    Itf,
    DataMatrix,
    Pdf417,
    Aztec,
    MicroQr,
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
            Sym::DataMatrix => "DATAMATRIX",
            Sym::Pdf417 => "PDF417",
            Sym::Aztec => "AZTEC",
            Sym::MicroQr => "MICRO_QR_CODE",
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
        Sym::Qr | Sym::DataMatrix | Sym::Pdf417 | Sym::Aztec | Sym::MicroQr => {
            unreachable!("not linear")
        }
    };
    let height = linear_height(rng, bars.len());
    Encoded::linear(text, sym.label(), &bars, height)
}

/// Data Matrix shape family of the `datamatrix` class and of the extra images.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmShape {
    Square,
    Rect,
    Dmre,
}

const DM_RECT: [SymbolSize; 6] = [
    SymbolSize::Rect8x18,
    SymbolSize::Rect8x32,
    SymbolSize::Rect12x26,
    SymbolSize::Rect12x36,
    SymbolSize::Rect16x36,
    SymbolSize::Rect16x48,
];

const DM_DMRE: [SymbolSize; 18] = [
    SymbolSize::Rect8x48,
    SymbolSize::Rect8x64,
    SymbolSize::Rect8x80,
    SymbolSize::Rect8x96,
    SymbolSize::Rect8x120,
    SymbolSize::Rect8x144,
    SymbolSize::Rect12x64,
    SymbolSize::Rect12x88,
    SymbolSize::Rect16x64,
    SymbolSize::Rect20x36,
    SymbolSize::Rect20x44,
    SymbolSize::Rect20x64,
    SymbolSize::Rect22x48,
    SymbolSize::Rect24x48,
    SymbolSize::Rect24x64,
    SymbolSize::Rect26x40,
    SymbolSize::Rect26x48,
    SymbolSize::Rect26x64,
];

fn dm_payload(rng: &mut ChaCha8Rng, len: usize) -> String {
    match rng.random_range(0..3u8) {
        0 => to_text(&digits(rng, len)),
        1 => pick_string(rng, &pool("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ -./"), len),
        _ => pick_string(rng, &pool(BYTE_ASCII), len),
    }
}

pub fn encode_dm(rng: &mut ChaCha8Rng, shape: DmShape) -> Encoded {
    let (list, max_len) = match shape {
        DmShape::Square => (SymbolList::default().enforce_square(), 40usize),
        DmShape::Rect => (SymbolList::with_whitelist(DM_RECT), 24),
        DmShape::Dmre => (SymbolList::with_whitelist(DM_DMRE), 40),
    };
    let mut len = rng.random_range(4..=max_len);
    loop {
        let text = dm_payload(rng, len);
        if !text.trim().is_empty()
            && text.trim() == text
            && let Ok(dm) = DataMatrix::encode(text.as_bytes(), list.clone())
        {
            let bm = dm.bitmap();
            return Encoded {
                text,
                format: Sym::DataMatrix.label(),
                modules: bm.bits().to_vec(),
                cols: bm.width(),
                rows: bm.height(),
                cell_h: 1.0,
                qz_x: 2.0,
                qz_y: 2.0,
            };
        }
        len = (len * 4 / 5).max(1);
    }
}

fn pdf417_payload(rng: &mut ChaCha8Rng, len: usize) -> String {
    match rng.random_range(0..4u8) {
        0 => to_text(&digits(rng, len)),
        1 => pick_string(rng, &pool("0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ "), len),
        2 => pick_string(rng, &pool(BYTE_ASCII), len),
        _ => {
            let head = pick_string(rng, &pool("ABCDEFGHIJKLMNOPQRSTUVWXYZ"), 4);
            format!("{head}{}", to_text(&digits(rng, len)))
        }
    }
}

/// PDF417 or compact PDF417 with a random column count and error
/// correction level (the encoder is rxing's, as in the unit tests).
pub fn encode_pdf417(rng: &mut ChaCha8Rng) -> Encoded {
    let compact = rng.random_bool(0.25);
    let mut len = rng.random_range(8..=100);
    loop {
        let text = pdf417_payload(rng, len);
        let cols = rng.random_range(2..=8u32);
        let level = rng.random_range(1..=5u32);
        let mut enc = PDF417::with_compact_hint(compact);
        enc.setDimensions(cols, cols, 90, 3);
        if !text.trim().is_empty()
            && text.trim() == text
            && enc.generateBarcodeLogic(&text, level).is_ok()
            && let Some(matrix) = enc.getBarcodeMatrix().as_ref()
        {
            // Rows are stored bottom first.
            let rows: Vec<Vec<u8>> = matrix.getMatrix().into_iter().rev().collect();
            let width = rows[0].len();
            let modules = rows
                .iter()
                .flat_map(|r| r.iter().map(|&m| m != 0))
                .collect();
            let cell_h = rng.random_range(3.0..4.5);
            return Encoded {
                text,
                format: Sym::Pdf417.label(),
                modules,
                cols: width,
                rows: rows.len(),
                cell_h,
                qz_x: 3.0,
                qz_y: 2.0,
            };
        }
        len = (len * 4 / 5).max(1);
    }
}

/// Layer choices of the Aztec images: negative for a compact symbol with that
/// many layers, positive for a full-range one.
pub const AZTEC_LAYERS: [i32; 14] = [-1, 2, -2, 3, -3, 5, -4, 4, 8, 6, 11, 7, 15, 10];

fn aztec_payload(rng: &mut ChaCha8Rng, len: usize) -> String {
    let text = match rng.random_range(0..4u8) {
        0 => to_text(&digits(rng, len)),
        1 => pick_string(rng, &pool("ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789"), len),
        2 => pick_string(rng, &pool(BYTE_ASCII), len),
        _ => {
            let head = pick_string(rng, &pool("abcdefghijklmnopqrstuvwxyz"), 5);
            format!("{head}{}", to_text(&digits(rng, len)))
        }
    };
    text.trim().to_string()
}

/// Aztec Code with the layer count of `AZTEC_LAYERS[pick]`, filled to a
/// random share of its capacity; `mirrored` transposes the modules (the encoder
/// is rxing's, as in the unit tests).
pub fn encode_aztec(rng: &mut ChaCha8Rng, pick: usize, mirrored: bool) -> Encoded {
    let layers = AZTEC_LAYERS[pick % AZTEC_LAYERS.len()];
    let l = layers.unsigned_abs() as usize;
    let bits = (if layers < 0 { 88 } else { 112 } + 16 * l) * l;
    let mut len = (bits * 5 / 60).max(2) * rng.random_range(40..=100usize) / 100;
    let ec = rng.random_range(23..=40u32);
    loop {
        let text = aztec_payload(rng, len.max(1));
        if !text.is_empty()
            && let Ok(code) = aztec_encoder::encode_bytes(text.as_bytes(), ec, layers)
            && code.getLayers() == l as u32
            && code.isCompact() == (layers < 0)
        {
            let n = code.getSize() as usize;
            let m = code.getMatrix();
            let mut modules = vec![false; n * n];
            for y in 0..n {
                for x in 0..n {
                    let (r, c) = if mirrored { (x, y) } else { (y, x) };
                    modules[r * n + c] = m.get(x as u32, y as u32);
                }
            }
            let mut e = Encoded::matrix(text, Sym::Aztec.label(), modules, n);
            e.qz_x = 2.0;
            e.qz_y = 2.0;
            return e;
        }
        len = (len * 4 / 5).max(1);
    }
}

/// Micro QR versions and levels of the images. M3-L is left out: the
/// encoder's M3-L symbols are not valid (a real one is read by the decoder
/// and by zxing-cpp, the encoder's are not).
pub const MICRO_QR_VARIANTS: [(i16, EcLevel); 7] = [
    (1, EcLevel::L),
    (2, EcLevel::L),
    (2, EcLevel::M),
    (3, EcLevel::M),
    (4, EcLevel::L),
    (4, EcLevel::M),
    (4, EcLevel::Q),
];

/// The encoder's padding underflows for M3 data that leaves 4 to 6 bits of
/// the last full codeword free, so such payloads are skipped (in a release
/// build the subtraction would wrap and the padding loop would not end).
fn micro_code(data: &[u8], version: i16, level: EcLevel) -> Option<QrCode> {
    let v = Version::Micro(version);
    let mut bits = qrcode::bits::Bits::new(v);
    bits.push_optimal_data(data).ok()?;
    if version == 3 && (58..=60).contains(&bits.len()) {
        return None;
    }
    QrCode::with_version(data, v, level).ok()
}

/// Micro QR Code of `MICRO_QR_VARIANTS[pick]`, filled to a random share of
/// its capacity; `mirrored` transposes the modules.
pub fn encode_microqr(rng: &mut ChaCha8Rng, pick: usize, mirrored: bool) -> Encoded {
    let (version, level) = MICRO_QR_VARIANTS[pick % MICRO_QR_VARIANTS.len()];
    let mut len = match version {
        1 => 5,
        2 => 8,
        3 => 12,
        _ => 24,
    } * rng.random_range(50..=100usize)
        / 100;
    loop {
        let kind = if version == 1 {
            0
        } else {
            rng.random_range(0..3u8)
        };
        let text = match kind {
            0 => to_text(&digits(rng, len.max(1))),
            1 => pick_string(
                rng,
                &pool("ABCDEFGHIJKLMNOPQRSTUVWXYZ 0123456789"),
                len.max(1),
            ),
            _ if version >= 3 => pick_string(rng, &pool(BYTE_ASCII), len.max(1)),
            _ => to_text(&digits(rng, len.max(1))),
        };
        let text = text.trim().to_string();
        if !text.is_empty()
            && let Some(code) = micro_code(text.as_bytes(), version, level)
        {
            let n = code.width();
            let colors = code.to_colors();
            let mut modules = vec![false; n * n];
            for y in 0..n {
                for x in 0..n {
                    let (r, c) = if mirrored { (x, y) } else { (y, x) };
                    modules[r * n + c] = colors[y * n + x] == Color::Dark;
                }
            }
            let mut e = Encoded::matrix(text, Sym::MicroQr.label(), modules, n);
            e.qz_x = 2.0;
            e.qz_y = 2.0;
            return e;
        }
        len = (len * 4 / 5).max(1);
    }
}

pub fn encode(rng: &mut ChaCha8Rng, sym: Sym) -> Encoded {
    match sym {
        Sym::MicroQr => {
            let pick = rng.random_range(0..MICRO_QR_VARIANTS.len());
            encode_microqr(rng, pick, false)
        }
        Sym::DataMatrix => encode_dm(rng, DmShape::Square),
        Sym::Pdf417 => encode_pdf417(rng),
        Sym::Aztec => {
            let pick = rng.random_range(0..AZTEC_LAYERS.len());
            encode_aztec(rng, pick, false)
        }
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
    fn aztec_covers_the_layer_choices_and_mirrors() {
        let mut rng = ChaCha8Rng::seed_from_u64(3);
        for (pick, &l) in AZTEC_LAYERS.iter().enumerate() {
            let a = encode_aztec(&mut rng, pick, false);
            let n = a.cols;
            let base = if l < 0 { 11 } else { 14 } + 4 * l.unsigned_abs() as usize;
            assert!(n >= base && n <= base + 1 + 2 * (base / 2 / 15), "{l}: {n}");
            assert_eq!(a.modules.len(), n * n);
        }
        // Same payload and seed, transposed.
        let a = encode_aztec(&mut ChaCha8Rng::seed_from_u64(9), 5, false);
        let b = encode_aztec(&mut ChaCha8Rng::seed_from_u64(9), 5, true);
        assert_eq!(a.text, b.text);
        let n = a.cols;
        assert!((0..n).all(|y| (0..n).all(|x| a.modules[y * n + x] == b.modules[x * n + y])));
    }

    #[test]
    fn microqr_covers_the_variants_and_mirrors() {
        let mut rng = ChaCha8Rng::seed_from_u64(4);
        for (pick, &(v, _)) in MICRO_QR_VARIANTS.iter().enumerate() {
            let m = encode_microqr(&mut rng, pick, false);
            assert_eq!(m.cols, 9 + 2 * v as usize);
            assert!(!m.text.is_empty());
        }
        let a = encode_microqr(&mut ChaCha8Rng::seed_from_u64(9), 5, false);
        let b = encode_microqr(&mut ChaCha8Rng::seed_from_u64(9), 5, true);
        assert_eq!(a.text, b.text);
        let n = a.cols;
        assert!((0..n).all(|y| (0..n).all(|x| a.modules[y * n + x] == b.modules[x * n + y])));
    }

    #[test]
    fn all_symbologies_encode_many_times() {
        let mut rng = ChaCha8Rng::seed_from_u64(7);
        for _ in 0..200 {
            for s in
                Sym::ALL
                    .into_iter()
                    .chain([Sym::DataMatrix, Sym::Pdf417, Sym::Aztec, Sym::MicroQr])
            {
                let e = encode(&mut rng, s);
                assert_eq!(e.modules.len(), e.cols * e.rows);
                assert!(!e.text.is_empty());
            }
        }
    }
}
