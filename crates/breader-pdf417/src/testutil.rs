//! Test helpers: an independent encoder (rxing's, used as a dev-dependency to
//! encode only) and a small sealer for hand-built codeword streams.

use crate::spec::{self, Layout};
use breader_ecc::PDF417_FIELD;
use rxing::common::CharacterSet;
use rxing::pdf417::encoder::pdf_417_high_level_encoder::encodeHighLevel;
use rxing::pdf417::encoder::{Compaction, PDF417};

pub(crate) const START: u32 = 0x1_FEA8;
pub(crate) const STOP: u32 = 0x3_FA29;

/// xorshift64 generator.
pub(crate) struct Rng(pub u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A symbol read back from the modules of an encoder's output.
#[derive(Debug, Clone)]
pub(crate) struct Rendered {
    pub layout: Layout,
    pub compact: bool,
    /// Rows times columns data-region codewords: length descriptor, data,
    /// padding and check codewords.
    pub cells: Vec<u16>,
    /// Left row indicator of each row, top to bottom.
    pub left: Vec<u16>,
    /// Right row indicators; empty for compact symbols.
    pub right: Vec<u16>,
}

#[derive(Clone, Copy)]
pub(crate) struct EncodeOptions {
    pub compaction: Compaction,
    pub level: u32,
    pub compact: bool,
    pub cols: Option<u32>,
    pub encoding: Option<CharacterSet>,
    pub auto_eci: bool,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            compaction: Compaction::AUTO,
            level: 2,
            compact: false,
            cols: None,
            encoding: None,
            auto_eci: false,
        }
    }
}

fn read_modules(row: &[u8], at: usize, len: usize) -> u32 {
    row[at..at + len]
        .iter()
        .fold(0, |acc, &m| acc << 1 | u32::from(m != 0))
}

/// Encodes `msg` (characters up to U+00FF are bytes) and reads the rendered
/// symbol back through the pattern tables, checking start and stop patterns
/// and the cluster of every codeword on the way.
pub(crate) fn render(msg: &str, opts: EncodeOptions) -> Rendered {
    try_render(msg, opts).expect("message fits")
}

/// [`render`], or `None` when the encoder cannot fit the message.
pub(crate) fn try_render(msg: &str, opts: EncodeOptions) -> Option<Rendered> {
    let mut enc = PDF417::with_compact_hint(opts.compact);
    enc.setCompaction(opts.compaction);
    enc.setEncoding(opts.encoding);
    if let Some(c) = opts.cols {
        enc.setDimensions(c, c, 90, 3);
    } else {
        enc.setDimensions(30, 1, 90, 3);
    }
    enc.generateBarcodeLogicWithAutoECI(msg, opts.level, opts.auto_eci)
        .ok()?;
    let mut rows = enc.getBarcodeMatrix().as_ref().unwrap().getMatrix();
    // The matrix is stored bottom row first.
    rows.reverse();
    let row_len = rows[0].len();
    let side = if opts.compact { 2 } else { 4 };
    let cols = (row_len - 1) / 17 - side;
    let (mut cells, mut left, mut right) = (Vec::new(), Vec::new(), Vec::new());
    for (r, row) in rows.iter().enumerate() {
        assert_eq!(row.len(), row_len);
        assert_eq!(read_modules(row, 0, 17), START, "start pattern, row {r}");
        let mut at = 17;
        let next = |at: &mut usize| {
            let p = read_modules(row, *at, 17);
            *at += 17;
            let (k, v) = spec::lookup(p).unwrap_or_else(|| panic!("unknown pattern {p:#x}"));
            assert_eq!(k, r % 3, "cluster of row {r}");
            v
        };
        left.push(next(&mut at));
        for _ in 0..cols {
            cells.push(next(&mut at));
        }
        if opts.compact {
            assert_eq!(row[at], 1, "single stop bar");
            assert_eq!(row.len(), at + 1);
        } else {
            right.push(next(&mut at));
            assert_eq!(read_modules(row, at, 18), STOP, "stop pattern, row {r}");
        }
    }
    let nrows = rows.len();
    let layout = Layout::new(nrows, cols, opts.level as u8).unwrap();
    Some(Rendered {
        layout,
        compact: opts.compact,
        cells,
        left,
        right,
    })
}

/// High-level encoding of `msg` (characters up to U+00FF are bytes), the
/// codewords after the length descriptor.
pub(crate) fn high_level(msg: &str, compaction: Compaction) -> Vec<u16> {
    encodeHighLevel(msg, compaction, None, false)
        .unwrap()
        .chars()
        .map(|c| c as u16)
        .collect()
}

/// Wraps data codewords (without the length descriptor) in a symbol of
/// `cols` data columns: descriptor, padding, check codewords. Returns the
/// layout and the row-major codewords.
pub(crate) fn seal(data: &[u16], level: u8, cols: usize) -> (Layout, Vec<u16>) {
    let ec = spec::ec_codewords(level);
    let rows = (1 + data.len() + ec).div_ceil(cols).max(spec::MIN_ROWS);
    let total = rows * cols;
    let sld = total - ec;
    assert!(sld > data.len(), "data does not fit");
    let mut cells = Vec::with_capacity(total);
    cells.push(sld as u16);
    cells.extend_from_slice(data);
    cells.resize(sld, 900);
    let mut check = vec![0u16; ec];
    PDF417_FIELD.encode(&cells, ec, &mut check).unwrap();
    cells.extend(check);
    (Layout::new(rows, cols, level).unwrap(), cells)
}

/// Same as [`seal`] with the fewest columns (at least 3) that keep the symbol
/// within 90 rows.
pub(crate) fn seal_auto(data: &[u16], level: u8) -> (Layout, Vec<u16>) {
    let ec = spec::ec_codewords(level);
    let cols = ((1 + data.len() + ec).div_ceil(spec::MAX_ROWS)).clamp(3, spec::MAX_COLS);
    seal(data, level, cols)
}
