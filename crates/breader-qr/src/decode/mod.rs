//! Module grid to payload: format/version information, unmasking,
//! codeword extraction, de-interleaving, Reed-Solomon and bitstream parsing.

mod bits;
#[cfg(test)]
mod tests;

use crate::spec::{self, FORMAT_MASK, FORMAT_WORDS, VERSION_WORDS};
use breader_core::{BitMatrix, DataSegment, EcLevel, Reject};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedQr {
    pub version: u8,
    pub ec_level: EcLevel,
    pub mask: u8,
    pub errors_corrected: u32,
    /// Concatenated segment payloads: ASCII for numeric/alphanumeric,
    /// raw bytes for byte mode, Shift-JIS bytes for kanji. FNC1 in first
    /// position is not included; `%` in alphanumeric GS1 data is mapped
    /// to ASCII 29 (and `%%` to `%`).
    pub data: Vec<u8>,
    /// Charset of each byte range of `data`, adjacent equal ranges merged.
    pub segments: Vec<DataSegment>,
    pub eci: Vec<u32>,
    pub mirrored: bool,
    /// ISO/IEC 15424 identifier: ]Q1..]Q6.
    pub aim_id: [u8; 3],
}

/// Decodes a sampled module grid (dark = true), `grid.width() == grid.height()`
/// == `spec::dimension(version)`. Retries the transposed grid when format
/// information fails in the normal orientation.
pub(crate) fn decode_grid(grid: &BitMatrix) -> Result<DecodedQr, Reject> {
    let d = grid.width();
    if grid.height() != d || spec::version_for_dimension(d).is_none() {
        return Err(Reject::Sampling);
    }
    let first = decode_oriented(grid, false);
    if first.is_ok() {
        return first;
    }
    let mut t = BitMatrix::new(d, d);
    for y in 0..d {
        for x in 0..d {
            if grid.get(x, y) {
                t.set(y, x, true);
            }
        }
    }
    match decode_oriented(&t, true) {
        Ok(r) => Ok(r),
        Err(_) => first,
    }
}

fn read_bits(grid: &BitMatrix, pos: &[(usize, usize)]) -> u32 {
    let mut w = 0;
    for (i, &(x, y)) in pos.iter().enumerate() {
        if x < grid.width() && y < grid.height() && grid.get(x, y) {
            w |= 1 << i;
        }
    }
    w
}

/// Nearest word of `table` within Hamming distance 3: (distance, index).
fn nearest(word: u32, table: &[u32]) -> Option<(u32, usize)> {
    let mut best: Option<(u32, usize)> = None;
    for (i, &t) in table.iter().enumerate() {
        let dist = (word ^ t).count_ones();
        if dist <= 3 && best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, i));
        }
    }
    best
}

/// Candidate (level, mask) pairs from the two format copies, best first.
fn format_candidates(grid: &BitMatrix) -> Vec<(EcLevel, u8)> {
    let pos = spec::format_positions(grid.width());
    let mut found: Vec<(u32, usize)> = Vec::with_capacity(2);
    for p in &pos {
        let w = read_bits(grid, p) ^ FORMAT_MASK;
        if let Some(n) = nearest(w, &FORMAT_WORDS) {
            found.push(n);
        }
    }
    found.sort_by_key(|&(dist, _)| dist);
    let mut out: Vec<(EcLevel, u8)> = Vec::with_capacity(2);
    for (_, idx) in found {
        let c = (spec::level_from_bits((idx >> 3) as u32), (idx & 7) as u8);
        if !out.contains(&c) {
            out.push(c);
        }
    }
    out
}

fn decode_oriented(grid: &BitMatrix, mirrored: bool) -> Result<DecodedQr, Reject> {
    let version = spec::version_for_dimension(grid.width()).ok_or(Reject::Sampling)?;
    let candidates = format_candidates(grid);
    let mut err = Reject::Format;
    for (level, mask) in candidates {
        match decode_with(grid, version, level, mask, mirrored) {
            Ok(r) => return Ok(r),
            Err(e) => err = e,
        }
    }
    Err(err)
}

fn mask_bit(mask: u8, x: usize, y: usize) -> bool {
    let (i, j) = (y, x);
    match mask {
        0 => (i + j) % 2 == 0,
        1 => i % 2 == 0,
        2 => j % 3 == 0,
        3 => (i + j) % 3 == 0,
        4 => (i / 2 + j / 3) % 2 == 0,
        5 => (i * j) % 2 + (i * j) % 3 == 0,
        6 => ((i * j) % 2 + (i * j) % 3) % 2 == 0,
        _ => ((i + j) % 2 + (i * j) % 3) % 2 == 0,
    }
}

fn decode_with(
    grid: &BitMatrix,
    version: u8,
    level: EcLevel,
    mask: u8,
    mirrored: bool,
) -> Result<DecodedQr, Reject> {
    let layout = spec::block_layout(version, level).ok_or(Reject::Format)?;
    let positions = spec::data_positions(version);
    let total = layout.total_codewords();
    if spec::total_codewords(version) != total || positions.len() < total * 8 {
        return Err(Reject::Sampling);
    }
    let mut codewords = vec![0u8; total];
    for (i, &(x, y)) in positions.iter().take(total * 8).enumerate() {
        let bit = grid.get(x, y) ^ mask_bit(mask, x, y);
        if bit {
            codewords[i / 8] |= 0x80 >> (i % 8);
        }
    }

    // De-interleave into blocks of data followed by EC codewords.
    let ec = usize::from(layout.ec_per_block);
    let mut lens: Vec<usize> = Vec::with_capacity(layout.block_count());
    for &(n, k) in &layout.groups {
        for _ in 0..n {
            lens.push(usize::from(k));
        }
    }
    let mut blocks: Vec<Vec<u8>> = lens.iter().map(|&k| vec![0u8; k + ec]).collect();
    let max_data = lens.iter().copied().max().unwrap_or(0);
    let mut it = codewords.iter().copied();
    for i in 0..max_data {
        for (block, &k) in blocks.iter_mut().zip(&lens) {
            if i < k {
                block[i] = it.next().ok_or(Reject::Length)?;
            }
        }
    }
    for i in 0..ec {
        for (b, &k) in lens.iter().enumerate() {
            blocks[b][k + i] = it.next().ok_or(Reject::Length)?;
        }
    }

    let mut corrected = 0u32;
    let mut payload: Vec<u8> = Vec::with_capacity(layout.data_codewords());
    for (block, &k) in blocks.iter_mut().zip(&lens) {
        let n = breader_ecc::QR_FIELD
            .decode(block, ec, &[])
            .map_err(|_| Reject::ErrorCorrection)?;
        let allowed = (ec - usize::from(spec::misdecode_protection(version, level)).min(ec)) / 2;
        if n > allowed {
            return Err(Reject::ErrorCorrection);
        }
        corrected = corrected.saturating_add(n as u32);
        payload.extend_from_slice(&block[..k]);
    }

    let parsed = bits::parse(&payload, version)?;
    Ok(DecodedQr {
        version,
        ec_level: level,
        mask,
        errors_corrected: corrected,
        data: parsed.data,
        segments: parsed.segments,
        eci: parsed.eci,
        mirrored,
        aim_id: parsed.aim_id,
    })
}

/// Version from the two version-information blocks (versions 7+), BCH-corrected.
/// Returns `None` when neither copy decodes.
pub(crate) fn read_version(grid: &BitMatrix) -> Option<u8> {
    let d = grid.width();
    if grid.height() != d || d < 45 {
        return None;
    }
    let mut best: Option<(u32, usize)> = None;
    for p in &spec::version_positions(d) {
        let w = read_bits(grid, p);
        if let Some(n) = nearest(w, &VERSION_WORDS)
            && best.is_none_or(|(b, _)| n.0 < b)
        {
            best = Some(n);
        }
    }
    best.map(|(_, i)| 7 + i as u8)
}
