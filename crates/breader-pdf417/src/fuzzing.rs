//! Entry points for fuzz targets. Not part of the public API.

use crate::decode::{self, ERASED};
use crate::spec::{self, Layout};
use breader_ecc::PDF417_FIELD;

/// Decodes a `rows` x `cols` codeword matrix with error correction level
/// `ec_level`. `cells` is row-major; missing cells are erasures, and so is
/// any value above 928. Returns the payload.
pub fn decode_codewords(rows: usize, cols: usize, ec_level: u8, cells: &[u16]) -> Option<Vec<u8>> {
    let layout = Layout::new(rows, cols, ec_level)?;
    let mut grid = vec![ERASED; rows * cols];
    for (dst, &src) in grid.iter_mut().zip(cells) {
        *dst = src;
    }
    decode::decode_codewords(&grid, layout).ok().map(|d| d.data)
}

/// Builds a symbol with `cols` data columns around `stream` (data
/// codewords, taken modulo 929 and truncated to the capacity of 90 rows) with
/// a valid length descriptor, padding and check codewords, and decodes it.
/// Reaches the compaction layer, which random matrices almost never do.
pub fn decode_stream(cols: usize, ec_level: u8, stream: &[u16]) -> Option<Vec<u8>> {
    let cols = cols.clamp(spec::MIN_COLS, spec::MAX_COLS);
    let ec_level = ec_level.min(spec::MAX_EC_LEVEL);
    let ec = spec::ec_codewords(ec_level);
    let capacity = (spec::MAX_ROWS * cols).min(spec::MAX_CODEWORDS);
    if capacity <= ec + 1 {
        return None;
    }
    let data = &stream[..stream.len().min(capacity - ec - 1)];
    let rows = (1 + data.len() + ec).div_ceil(cols).max(spec::MIN_ROWS);
    let total = rows * cols;
    if total > spec::MAX_CODEWORDS {
        return None;
    }
    let mut cells = Vec::with_capacity(total);
    cells.push((total - ec) as u16);
    cells.extend(data.iter().map(|&c| c % 929));
    cells.resize(total - ec, 900);
    let mut check = vec![0u16; ec];
    PDF417_FIELD.encode(&cells, ec, &mut check).ok()?;
    cells.extend(check);
    let layout = Layout::new(rows, cols, ec_level)?;
    decode::decode_codewords(&cells, layout)
        .ok()
        .map(|d| d.data)
}
