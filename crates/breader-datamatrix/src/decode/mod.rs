//! Module grid to payload: pattern removal, codeword extraction,
//! de-interleaving, Reed-Solomon and encodation modes.

mod encodation;
#[cfg(test)]
mod tests;

use crate::place;
use crate::spec::{self, SymbolSize};
use breader_core::{BitMatrix, DataSegment, Reject};
use breader_ecc::Gf256;

/// Data Matrix field: x^8 + x^5 + x^3 + x^2 + 1, generator roots from alpha^1.
pub(crate) static DM_FIELD: Gf256 = Gf256::new(0x12d, 1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedDm {
    pub rows: u16,
    pub cols: u16,
    pub errors_corrected: u32,
    /// Payload bytes after macro 05/06 expansion; FNC1 in first or second
    /// position is not included, later FNC1 become ASCII 29.
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub eci: Vec<u32>,
    pub structured_append: Option<(u8, u8, u16)>,
    pub mirrored: bool,
    /// ISO/IEC 15424 identifier: ]d1..]d6.
    pub aim_id: [u8; 3],
}

/// Decodes a full sampled symbol grid (dark = true) whose size is `rows` x
/// `cols` including finder and clock patterns. The L-shaped finder must be on
/// the left and bottom edges. Retries the mirrored symbol (the grid reflected
/// across the diagonal through the bottom-left corner, which swaps its
/// dimensions) when the normal orientation fails.
pub(crate) fn decode_grid(grid: &BitMatrix) -> Result<DecodedDm, Reject> {
    let first = decode_oriented(grid, false);
    if first.is_ok() {
        return first;
    }
    let (w, h) = (grid.width(), grid.height());
    if h == 0 || w == 0 || h > 255 || w > 255 || spec::size(w as u16, h as u16).is_none() {
        return first;
    }
    // new(r, c) = old(h - 1 - c, w - 1 - r), a grid of `w` rows and `h` columns.
    let mut t = BitMatrix::new(h, w);
    for r in 0..w {
        for c in 0..h {
            if grid.get(w - 1 - r, h - 1 - c) {
                t.set(c, r, true);
            }
        }
    }
    match decode_oriented(&t, true) {
        Ok(d) => Ok(d),
        Err(_) => first,
    }
}

/// Blocks with at most this many check codewords are in the conservative
/// range of `max_corrections`.
const SMALL_BLOCK_EC: usize = 12;

/// Corrections refused to keep false positives out: small blocks have too
/// little redundancy to trust their last correctable error.
fn max_corrections(ec_per_block: usize) -> usize {
    let reserved = match ec_per_block {
        0..=7 => 2,
        8..=12 => 1,
        _ => 0,
    };
    ec_per_block.saturating_sub(reserved) / 2
}

fn decode_oriented(grid: &BitMatrix, mirrored: bool) -> Result<DecodedDm, Reject> {
    let (rows, cols) = (grid.height(), grid.width());
    if rows > 255 || cols > 255 {
        return Err(Reject::Sampling);
    }
    let size = spec::size(rows as u16, cols as u16).ok_or(Reject::Sampling)?;
    let (corrected, data_cw) = read_codewords(grid, size)?;
    // A block with enough check codewords that decoded without any correction
    // is a valid codeword set by itself (a random word passes with probability
    // 2^-(8 * ec)), so the pad sequence adds nothing to the misdecode guard
    // there. Corrected or small blocks keep it.
    let ec_per_block = usize::from(size.ec_codewords) / usize::from(size.blocks);
    let strict_pad = corrected > 0 || ec_per_block <= SMALL_BLOCK_EC;
    let parsed = encodation::parse(&data_cw, strict_pad)?;
    Ok(DecodedDm {
        rows: size.rows,
        cols: size.cols,
        errors_corrected: corrected,
        data: parsed.data,
        segments: parsed.segments,
        eci: parsed.eci,
        structured_append: parsed.structured_append,
        mirrored,
        aim_id: parsed.aim_id,
    })
}

/// Raw codewords of the grid (data then error correction, in symbol order).
fn extract(grid: &BitMatrix, size: &SymbolSize) -> Result<Vec<u8>, Reject> {
    let placement = place::placement(size).ok_or(Reject::Sampling)?;
    if placement.words.len() != size.total_codewords() {
        return Err(Reject::Sampling);
    }
    let mut out = Vec::with_capacity(placement.words.len());
    for word in &placement.words {
        let mut v = 0u8;
        for &(r, c) in word {
            let (y, x) = size.symbol_position(usize::from(r), usize::from(c));
            v = v << 1 | u8::from(grid.get(x, y));
        }
        out.push(v);
    }
    Ok(out)
}

/// Block index and position inside the block for interleaved index `k`.
fn block_of(k: usize, blocks: usize) -> (usize, usize) {
    (k % blocks, k / blocks)
}

/// Extracts, de-interleaves and corrects; returns (corrected count,
/// corrected data codewords in stream order).
fn read_codewords(grid: &BitMatrix, size: &SymbolSize) -> Result<(u32, Vec<u8>), Reject> {
    let raw = extract(grid, size)?;
    let (data_len, blocks) = (usize::from(size.data_codewords), usize::from(size.blocks));
    let ec = usize::from(size.ec_codewords) / blocks;
    let mut buf: Vec<Vec<u8>> = (0..blocks)
        .map(|b| vec![0u8; data_len / blocks + usize::from(b < data_len % blocks) + ec])
        .collect();
    for (k, &cw) in raw[..data_len].iter().enumerate() {
        let (b, i) = block_of(k, blocks);
        buf[b][i] = cw;
    }
    for (k, &cw) in raw[data_len..].iter().enumerate() {
        let (b, i) = block_of(k, blocks);
        let len = buf[b].len();
        buf[b][len - ec + i] = cw;
    }
    let mut corrected = 0u32;
    for block in &mut buf {
        let n = DM_FIELD
            .decode(block, ec, &[])
            .map_err(|_| Reject::ErrorCorrection)?;
        if n > max_corrections(ec) {
            return Err(Reject::ErrorCorrection);
        }
        corrected += n as u32;
    }
    let data = (0..data_len)
        .map(|k| {
            let (b, i) = block_of(k, blocks);
            buf[b][i]
        })
        .collect();
    Ok((corrected, data))
}
