//! Entry points for fuzz targets. Not part of the public API.

use breader_core::BitMatrix;

/// Decodes a `dim` x `dim` module grid whose cells are the bits of `bits`
/// (LSB first, row-major; missing bits are light). Returns the payload.
pub fn decode_grid(dim: usize, bits: &[u8]) -> Option<Vec<u8>> {
    if !(1..=177).contains(&dim) {
        return None;
    }
    let mut grid = BitMatrix::new(dim, dim);
    for i in 0..dim * dim {
        if bits.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 == 1) {
            grid.set(i % dim, i / dim, true);
        }
    }
    crate::decode::decode_grid(&grid).ok().map(|d| d.data)
}

/// Decodes a `dim` x `dim` Micro QR module grid with the same cell layout
/// as [`decode_grid`]. Returns the payload.
pub fn decode_micro_grid(dim: usize, bits: &[u8]) -> Option<Vec<u8>> {
    if !(1..=64).contains(&dim) {
        return None;
    }
    let mut grid = BitMatrix::new(dim, dim);
    for i in 0..dim * dim {
        if bits.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 == 1) {
            grid.set(i % dim, i / dim, true);
        }
    }
    crate::micro::decode::decode_grid(&grid)
        .ok()
        .map(|d| d.data)
}
