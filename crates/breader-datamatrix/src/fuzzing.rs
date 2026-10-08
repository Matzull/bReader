//! Entry points for fuzz targets. Not part of the public API.

use breader_core::BitMatrix;

/// Decodes a `rows` x `cols` module grid whose cells are the bits of `bits`
/// (LSB first, row-major; missing bits are light). Returns the payload.
pub fn decode_grid(rows: usize, cols: usize, bits: &[u8]) -> Option<Vec<u8>> {
    if !(1..=144).contains(&rows) || !(1..=144).contains(&cols) {
        return None;
    }
    let mut grid = BitMatrix::new(cols, rows);
    for i in 0..rows * cols {
        if bits.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 == 1) {
            grid.set(i % cols, i / cols, true);
        }
    }
    crate::decode::decode_grid(&grid).ok().map(|d| d.data)
}

/// Builds a symbol of table size number `size_index` (modulo the table
/// length) around `stream` (truncated or padded to the data
/// capacity, with valid error correction) and decodes it. Reaches the
/// encodation layer, which random grids almost never do.
pub fn decode_stream(size_index: usize, stream: &[u8]) -> Option<Vec<u8>> {
    use crate::{place, spec};
    let size = &spec::sizes()[size_index % spec::sizes().len()];
    let data_len = usize::from(size.data_codewords);
    let blocks = usize::from(size.blocks);
    let ec = usize::from(size.ec_codewords) / blocks;
    let mut cw = vec![0u8; size.total_codewords()];
    for (dst, src) in cw[..data_len].iter_mut().zip(stream) {
        *dst = *src;
    }
    if stream.len() < data_len {
        cw[stream.len()] = 129;
        for pos in stream.len() + 2..=data_len {
            let v = 129 + (149 * pos) % 253 + 1;
            cw[pos - 1] = (if v > 254 { v - 254 } else { v }) as u8;
        }
    }
    for b in 0..blocks {
        let block: Vec<u8> = cw[..data_len]
            .iter()
            .copied()
            .skip(b)
            .step_by(blocks)
            .collect();
        let mut check = vec![0u8; ec];
        crate::decode::DM_FIELD
            .encode(&block, ec, &mut check)
            .ok()?;
        for (i, &c) in check.iter().enumerate() {
            cw[data_len + i * blocks + b] = c;
        }
    }
    let mut grid = BitMatrix::new(usize::from(size.cols), usize::from(size.rows));
    let placement = place::placement(size)?;
    for (word, &value) in placement.words.iter().zip(&cw) {
        for (i, &(r, c)) in word.iter().enumerate() {
            let (y, x) = size.symbol_position(usize::from(r), usize::from(c));
            grid.set(x, y, value >> (7 - i) & 1 == 1);
        }
    }
    crate::decode::decode_grid(&grid).ok().map(|d| d.data)
}
