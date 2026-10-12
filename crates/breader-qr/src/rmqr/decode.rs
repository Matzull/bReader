//! rMQR module grid to payload: format information, unmasking,
//! de-interleaving, Reed-Solomon and the bitstream.

use super::bits;
use super::spec::{
    self, Cell, DATA, FORMAT_MASK_LEFT, FORMAT_MASK_RIGHT, FORMAT_WORDS, format_left, format_right,
    mask_bit, template, version_for_size,
};
use crate::micro::decode::correctable;
use breader_core::{BitMatrix, DataSegment, EcLevel, Reject};

/// Largest Hamming distance to a valid format word accepted when both copies
/// decode to the same word (the code has minimum distance 8).
pub(crate) const MAX_FORMAT_DISTANCE: u32 = 3;
/// Largest distance accepted when only one copy decodes.
pub(crate) const MAX_SINGLE_DISTANCE: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedRmqr {
    /// Version indicator plus one, 1 to 32.
    pub version: u8,
    pub width: usize,
    pub height: usize,
    pub ec_level: EcLevel,
    pub errors_corrected: u32,
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub eci: Vec<u32>,
    pub aim_id: [u8; 3],
}

fn read_bits(grid: &BitMatrix, cells: &[Cell; 18], swap_tail: bool) -> u32 {
    let mut w = 0u32;
    for (i, &(x, y)) in cells.iter().enumerate() {
        let i = if swap_tail && i >= 15 { 32 - i } else { i };
        if x < grid.width() && y < grid.height() && grid.get(x, y) {
            w |= 1 << i;
        }
    }
    w
}

/// Nearest format word to `raw ^ mask`: (distance, 6-bit value). The three
/// extra bits are tried in both orders, as the order of that tail was not
/// confirmed against the standard text.
pub(crate) fn nearest_format(grid: &BitMatrix, cells: &[Cell; 18], mask: u32) -> Option<(u32, u8)> {
    let mut best: Option<(u32, u8)> = None;
    for swap in [false, true] {
        let word = read_bits(grid, cells, swap) ^ mask;
        for (d, &t) in FORMAT_WORDS.iter().enumerate() {
            let dist = (word ^ t).count_ones();
            if dist <= MAX_FORMAT_DISTANCE && best.is_none_or(|(b, _)| dist < b) {
                best = Some((dist, d as u8));
            }
        }
    }
    best
}

/// Format information from the left copy only (used before the grid is
/// sampled): (distance, 6-bit value).
pub(crate) fn left_format(grid: &BitMatrix) -> Option<(u32, u8)> {
    nearest_format(grid, &format_left(), FORMAT_MASK_LEFT)
}

/// Combines both copies of a sampled grid into `(level high, version indicator)`.
pub(crate) fn read_format(grid: &BitMatrix) -> Option<(bool, usize)> {
    let l = nearest_format(grid, &format_left(), FORMAT_MASK_LEFT);
    let r = nearest_format(
        grid,
        &format_right(grid.width(), grid.height()),
        FORMAT_MASK_RIGHT,
    );
    let v = match (l, r) {
        (Some((da, a)), Some((db, b))) => {
            if a == b {
                a
            } else if da <= 1 && da + 2 <= db {
                // One copy is clean and the other is nearer to another word.
                a
            } else if db <= 1 && db + 2 <= da {
                b
            } else {
                return None;
            }
        }
        (Some((d, a)), None) | (None, Some((d, a))) if d <= MAX_SINGLE_DISTANCE => a,
        _ => return None,
    };
    Some((v >> 5 & 1 == 1, usize::from(v & 31)))
}

/// Decodes a sampled grid (dark = true) of an rMQR size.
pub(crate) fn decode_grid(grid: &BitMatrix) -> Result<DecodedRmqr, Reject> {
    let (w, h) = (grid.width(), grid.height());
    let version = version_for_size(w, h).ok_or(Reject::Sampling)?;
    let (high, named) = read_format(grid).ok_or(Reject::Format)?;
    if named != version {
        return Err(Reject::Format);
    }
    let layout = spec::layout(version, high);
    let t = template(version);
    if t.data.len() < 8 * layout.total {
        return Err(Reject::Sampling);
    }
    debug_assert!(t.cells.iter().filter(|&&c| c == DATA).count() == t.data.len());
    let mut codewords = vec![0u8; layout.total];
    for (i, &(x, y)) in t.data.iter().take(8 * layout.total).enumerate() {
        let (x, y) = (usize::from(x), usize::from(y));
        if grid.get(x, y) ^ mask_bit(x, y) {
            codewords[i / 8] |= 0x80 >> (i % 8);
        }
    }

    let (b, ec) = (layout.blocks, layout.ec);
    let lens: Vec<usize> = (0..b).map(|i| layout.block_data(i)).collect();
    let mut blocks: Vec<Vec<u8>> = lens.iter().map(|&k| vec![0u8; k + ec]).collect();
    let mut it = codewords.iter().copied();
    for i in 0..lens.iter().copied().max().unwrap_or(0) {
        for (block, &k) in blocks.iter_mut().zip(&lens) {
            if i < k {
                block[i] = it.next().ok_or(Reject::Length)?;
            }
        }
    }
    for i in 0..ec {
        for (block, &k) in blocks.iter_mut().zip(&lens) {
            block[k + i] = it.next().ok_or(Reject::Length)?;
        }
    }
    let mut corrected = 0u32;
    let mut payload: Vec<u8> = Vec::with_capacity(layout.data_codewords());
    for (block, &k) in blocks.iter_mut().zip(&lens) {
        let n = breader_ecc::QR_FIELD
            .decode(block, ec, &[])
            .map_err(|_| Reject::ErrorCorrection)?;
        if n > correctable(ec) {
            return Err(Reject::ErrorCorrection);
        }
        corrected += n as u32;
        payload.extend_from_slice(&block[..k]);
    }
    let parsed = bits::parse(&payload, version)?;
    Ok(DecodedRmqr {
        version: version as u8 + 1,
        width: w,
        height: h,
        ec_level: if high { EcLevel::H } else { EcLevel::M },
        errors_corrected: corrected,
        data: parsed.data,
        segments: parsed.segments,
        eci: parsed.eci,
        aim_id: parsed.aim_id,
    })
}
