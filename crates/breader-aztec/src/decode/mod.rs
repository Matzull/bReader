//! Module grid to payload: finder gate, mode message, codeword extraction
//! along the spiral, Reed-Solomon with the misdecode guard, bit unstuffing
//! and high-level decoding.

#[cfg(test)]
mod tests;
pub(crate) mod text;

use crate::spec::{self, Layout};
use breader_core::{BitMatrix, Charset, DataSegment, Reject};
use breader_ecc::{
    AZTEC_DATA_6, AZTEC_DATA_8, AZTEC_DATA_10, AZTEC_DATA_12, AZTEC_PARAM, Correction, RsError,
};
use text::Fnc1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedAztec {
    pub compact: bool,
    /// 0 for a Rune.
    pub layers: u8,
    pub data_codewords: u16,
    pub ec_codewords: u16,
    pub errors_corrected: u32,
    pub erasures: u32,
    pub mirrored: bool,
    pub rune: Option<u8>,
    /// Payload bytes; FNC1 in first or second position is not included,
    /// later FNC1 become ASCII 29. A Rune's payload is its decimal text.
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub eci: Vec<u32>,
    /// ISO/IEC 15424 identifier: ]z0 to ]z5 and ]zC for a Rune.
    pub aim_id: [u8; 3],
}

/// Check codewords the guard keeps in reserve: blocks with few check words
/// have too little redundancy to trust their last correctable error.
fn reserved(ec: usize) -> usize {
    match ec {
        0..=7 => 2,
        8..=12 => 1,
        _ => 0,
    }
}

/// Correction budget: `2 * errors + erasures` may not exceed it.
pub(crate) fn budget(ec: usize) -> usize {
    ec.saturating_sub(reserved(ec))
}

/// The grid reflected across its main diagonal, which turns a mirrored
/// symbol into a regular one (and back).
pub(crate) fn transpose(grid: &BitMatrix) -> BitMatrix {
    let (w, h) = (grid.width(), grid.height());
    let mut t = BitMatrix::new(h, w);
    for y in 0..h {
        for x in 0..w {
            if grid.get(x, y) {
                t.set(y, x, true);
            }
        }
    }
    t
}

/// Decodes a sampled symbol: a square grid of the full symbol (dark =
/// true), already rotated so that the orientation marks are at the top-left
/// (three modules), top-right (two) and bottom-right (one) corners of the
/// mode message ring. `mirrored` only records that the caller transposed it.
pub(crate) fn decode_grid(grid: &BitMatrix, mirrored: bool) -> Result<DecodedAztec, Reject> {
    let size = grid.width();
    if size != grid.height() || size > 151 {
        return Err(Reject::Sampling);
    }
    if size == spec::RUNE_SIZE {
        return decode_rune(grid, mirrored);
    }
    let mut candidates: Vec<(usize, Layout)> = Layout::for_size(size)
        .filter_map(|l| finder_score(grid, l.compact).map(|s| (s, l)))
        .collect();
    if candidates.is_empty() {
        return Err(Reject::NoPattern);
    }
    candidates.sort_by_key(|&(s, _)| std::cmp::Reverse(s));
    let mut last = Reject::Format;
    for (_, layout) in candidates {
        match decode_layout(grid, layout, mirrored) {
            Ok(d) => return Ok(d),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Modules of the bullseye rings and orientation marks that agree with the
/// pattern, if enough of them do to be worth an error correction attempt.
fn finder_score(grid: &BitMatrix, compact: bool) -> Option<usize> {
    let size = grid.width();
    let c = (size / 2) as i32;
    let core: i32 = if compact { 4 } else { 6 };
    let at = |dx: i32, dy: i32| grid.get((c + dx) as usize, (c + dy) as usize);
    let mut ring_ok = 0;
    for dy in -core..=core {
        for dx in -core..=core {
            let d = dx.abs().max(dy.abs()) as usize;
            ring_ok += usize::from(at(dx, dy) == spec::bullseye_dark(d));
        }
    }
    let ring_total = ((2 * core + 1) * (2 * core + 1)) as usize;
    let r = core + 1;
    let marks: [(i32, i32, bool); 12] = [
        (-r, -r, true),
        (-r + 1, -r, true),
        (-r, -r + 1, true),
        (r, -r, true),
        (r, -r + 1, true),
        (r - 1, -r, false),
        (r, r - 1, true),
        (r, r, false),
        (r - 1, r, false),
        (-r, r, false),
        (-r + 1, r, false),
        (-r, r - 1, false),
    ];
    let marks_ok = marks.iter().filter(|&&(dx, dy, v)| at(dx, dy) == v).count();
    (ring_ok * 5 >= ring_total * 4 && marks_ok >= 10).then_some(ring_ok + marks_ok)
}

fn read_mode_bits(grid: &BitMatrix, compact: bool) -> Vec<bool> {
    let c = (grid.width() / 2) as i32;
    spec::mode_positions(compact)
        .into_iter()
        .map(|(dx, dy)| grid.get((c + dx) as usize, (c + dy) as usize))
        .collect()
}

fn words4(bits: &[bool]) -> Vec<u16> {
    bits.chunks(4)
        .map(|w| w.iter().fold(0u16, |acc, &b| acc << 1 | u16::from(b)))
        .collect()
}

/// Reed-Solomon over the mode message words; returns the corrected words
/// and the number of corrections, within the guard.
fn correct_mode(mut words: Vec<u16>, check: usize) -> Result<(Vec<u16>, u32), Reject> {
    match AZTEC_PARAM.decode(&mut words, check, &[]) {
        Ok(c) if 2 * c.errors <= budget(check) => Ok((words, c.errors as u32)),
        _ => Err(Reject::ErrorCorrection),
    }
}

fn decode_rune(grid: &BitMatrix, mirrored: bool) -> Result<DecodedAztec, Reject> {
    finder_score(grid, true).ok_or(Reject::NoPattern)?;
    let bits = read_mode_bits(grid, true);
    let mask = (0..28).map(|i| spec::RUNE_MASK >> (27 - i) & 1 == 1);
    let bits: Vec<bool> = bits.into_iter().zip(mask).map(|(b, m)| b ^ m).collect();
    let (words, errors) = correct_mode(words4(&bits), spec::COMPACT_MODE_CHECK)?;
    let value = (words[0] << 4 | words[1]) as u8;
    let data = value.to_string().into_bytes();
    Ok(DecodedAztec {
        compact: true,
        layers: 0,
        data_codewords: 0,
        ec_codewords: 0,
        errors_corrected: errors,
        erasures: 0,
        mirrored,
        rune: Some(value),
        segments: vec![DataSegment {
            start: 0,
            end: data.len(),
            charset: Charset::Ascii,
        }],
        data,
        eci: Vec::new(),
        aim_id: *b"]zC",
    })
}

/// Layers and data codewords from the mode message of `compact`, within the
/// guard.
fn read_mode(grid: &BitMatrix, compact: bool) -> Result<(Layout, usize, u32), Reject> {
    let bits = read_mode_bits(grid, compact);
    let (words, check) = if compact {
        (words4(&bits), spec::COMPACT_MODE_CHECK)
    } else {
        (words4(&bits), spec::FULL_MODE_CHECK)
    };
    let (words, errors) = correct_mode(words, check)?;
    let (layers, data) = if compact {
        let v = usize::from(words[0] << 4 | words[1]);
        (v >> 6, v & 0x3f)
    } else {
        let v = usize::from(words[0]) << 12
            | usize::from(words[1]) << 8
            | usize::from(words[2]) << 4
            | usize::from(words[3]);
        (v >> 11, v & 0x7ff)
    };
    Ok((
        Layout {
            compact,
            layers: layers + 1,
        },
        data + 1,
        errors,
    ))
}

fn rs_decode(
    word_size: usize,
    block: &mut [u16],
    ec: usize,
    erasures: &[usize],
) -> Result<Correction, RsError> {
    match word_size {
        6 => AZTEC_DATA_6.decode(block, ec, erasures),
        8 => AZTEC_DATA_8.decode(block, ec, erasures),
        10 => AZTEC_DATA_10.decode(block, ec, erasures),
        _ => AZTEC_DATA_12.decode(block, ec, erasures),
    }
}

fn decode_layout(
    grid: &BitMatrix,
    expected: Layout,
    mirrored: bool,
) -> Result<DecodedAztec, Reject> {
    let (layout, data_cw, mode_errors) = read_mode(grid, expected.compact)?;
    if layout != expected {
        return Err(Reject::Format);
    }
    let total = layout.total_codewords();
    if data_cw + spec::MIN_CHECK_WORDS > total {
        return Err(Reject::Format);
    }
    let ec = total - data_cw;
    let w = layout.word_size();

    let positions = spec::data_positions(layout);
    let bits: Vec<bool> = positions.iter().map(|&(x, y)| grid.get(x, y)).collect();
    let mut block: Vec<u16> = bits[layout.start_pad()..]
        .chunks_exact(w)
        .map(|c| c.iter().fold(0u16, |acc, &b| acc << 1 | u16::from(b)))
        .collect();
    debug_assert_eq!(block.len(), total);

    // A data codeword of all zeros or all ones cannot occur after bit
    // stuffing: it is known to be wrong.
    let ones = (1u16 << w) - 1;
    let erasures: Vec<usize> = (0..data_cw)
        .filter(|&k| block[k] == 0 || block[k] == ones)
        .collect();
    if erasures.len() > budget(ec) {
        return Err(Reject::ErrorCorrection);
    }
    let corr = rs_decode(w, &mut block, ec, &erasures).map_err(|_| Reject::ErrorCorrection)?;
    if 2 * corr.errors + erasures.len() > budget(ec) {
        return Err(Reject::ErrorCorrection);
    }

    let mut stream: Vec<bool> = Vec::with_capacity(data_cw * w);
    for &word in &block[..data_cw] {
        let top = word >> 1;
        let stuffed = top == 0 || top == ones >> 1;
        if stuffed && (word & 1 == 1) == (top & 1 == 1) {
            return Err(Reject::Format);
        }
        let keep = if stuffed { w - 1 } else { w };
        stream.extend((0..keep).map(|i| word >> (w - 1 - i) & 1 == 1));
    }
    let parsed = text::parse(&stream)?;

    let modifier = match parsed.fnc1 {
        Fnc1::None => b'0',
        Fnc1::First => b'1',
        Fnc1::Second => b'2',
    } + if parsed.eci.is_empty() { 0 } else { 3 };
    Ok(DecodedAztec {
        compact: layout.compact,
        layers: layout.layers as u8,
        data_codewords: data_cw as u16,
        ec_codewords: ec as u16,
        errors_corrected: mode_errors + corr.errors as u32,
        erasures: corr.erasures as u32,
        mirrored,
        rune: None,
        data: parsed.data,
        segments: parsed.segments,
        eci: parsed.eci,
        aim_id: [b']', b'z', modifier],
    })
}
