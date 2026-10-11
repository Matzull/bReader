//! Micro QR module grid to payload: format information, unmasking,
//! Reed-Solomon and the bitstream.

use super::bits;
use super::spec::{
    FORMAT_MASK, FORMAT_POSITIONS, VARIANTS, Variant, data_positions, mask_bit,
    version_for_dimension,
};
use crate::spec::FORMAT_WORDS;
use breader_core::{BitMatrix, DataSegment, EcLevel, Reject};

/// Largest Hamming distance to a valid format word that is accepted. The
/// symbol has a single copy of the format information, so one more error
/// than the QR copies tolerate would make a random pattern too likely to
/// pass.
pub(crate) const MAX_FORMAT_DISTANCE: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedMicro {
    pub version: u8,
    pub ec_level: EcLevel,
    pub mask: u8,
    pub errors_corrected: u32,
    /// Payload: ASCII for numeric and alphanumeric segments, raw bytes for
    /// byte mode, Shift-JIS for kanji.
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub mirrored: bool,
}

/// Format information read from a grid: (symbol number, mask, distance).
pub(crate) fn read_format(grid: &BitMatrix) -> Option<(usize, u8, u32)> {
    let mut word = 0u32;
    for (i, &(x, y)) in FORMAT_POSITIONS.iter().enumerate() {
        if grid.get(x, y) {
            word |= 1 << i;
        }
    }
    word ^= FORMAT_MASK;
    let mut best: Option<(u32, usize)> = None;
    for (i, &w) in FORMAT_WORDS.iter().enumerate() {
        let dist = (word ^ w).count_ones();
        if dist <= MAX_FORMAT_DISTANCE && best.is_none_or(|(b, _)| dist < b) {
            best = Some((dist, i));
        }
    }
    best.map(|(dist, i)| (i >> 2, (i & 3) as u8, dist))
}

/// Errors Reed-Solomon may correct before the result is distrusted: two
/// check codewords are kept in reserve for detection (one when there are
/// more than eight). M1 corrects nothing.
pub(crate) const fn correctable(ec: usize) -> usize {
    let reserve = if ec <= 8 { 2 } else { 1 };
    (ec - reserve) / 2
}

/// Decodes a square sampled grid (dark = true) of 11, 13, 15 or 17 modules.
/// Retries the transposed grid when the symbol is a mirror image.
pub(crate) fn decode_grid(grid: &BitMatrix) -> Result<DecodedMicro, Reject> {
    let d = grid.width();
    if grid.height() != d || version_for_dimension(d).is_none() {
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

fn decode_oriented(grid: &BitMatrix, mirrored: bool) -> Result<DecodedMicro, Reject> {
    let dim = grid.width();
    let (number, mask, _) = read_format(grid).ok_or(Reject::Format)?;
    let var = VARIANTS[number];
    if version_for_dimension(dim) != Some(var.version) {
        return Err(Reject::Format);
    }
    let (data, errors) = correct(grid, &var, mask)?;
    let parsed = bits::parse(&data, var.data_bits, var.version)?;
    Ok(DecodedMicro {
        version: var.version,
        ec_level: var.level,
        mask,
        errors_corrected: errors as u32,
        data: parsed.data,
        segments: parsed.segments,
        mirrored,
    })
}

/// Unmasks the data modules, runs Reed-Solomon on the single block and
/// returns the corrected data bytes with the number of codewords fixed.
fn correct(grid: &BitMatrix, var: &Variant, mask: u8) -> Result<(Vec<u8>, usize), Reject> {
    let positions = data_positions(grid.width());
    let nbits = var.data_bits + 8 * var.ec;
    if positions.len() < nbits {
        return Err(Reject::Sampling);
    }
    let dcw = var.data_codewords();
    let mut block = vec![0u8; dcw + var.ec];
    for (i, &(x, y)) in positions.iter().take(nbits).enumerate() {
        if grid.get(x, y) ^ mask_bit(mask, y, x) {
            // Data bits fill the data codewords; the check codewords start at
            // the next module even when the last data codeword is a half one.
            let (byte, bit) = if i < var.data_bits {
                (i / 8, i % 8)
            } else {
                let j = i - var.data_bits;
                (dcw + j / 8, j % 8)
            };
            block[byte] |= 0x80 >> bit;
        }
    }
    let n = breader_ecc::QR_FIELD
        .decode(&mut block, var.ec, &[])
        .map_err(|_| Reject::ErrorCorrection)?;
    if n > correctable(var.ec) {
        return Err(Reject::ErrorCorrection);
    }
    // The unused half of a half codeword is not transmitted.
    if !var.data_bits.is_multiple_of(8) && block[dcw - 1] & 0x0F != 0 {
        return Err(Reject::ErrorCorrection);
    }
    block.truncate(dcw);
    Ok((block, n))
}
