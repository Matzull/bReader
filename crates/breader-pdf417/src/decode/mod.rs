//! Codeword matrix to payload: Reed-Solomon, symbol length descriptor and
//! compaction modes.

mod compaction;
#[cfg(test)]
mod tests;

use crate::spec::{self, Layout};
use breader_core::{DataSegment, MacroPdf417, Reject};
use breader_ecc::PDF417_FIELD;

/// Marks an unreadable cell. Any value above 928 is treated the same way.
pub(crate) const ERASED: u16 = u16::MAX;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedPdf417 {
    pub layout: Layout,
    /// Codewords outside the erasure list that Reed-Solomon changed.
    pub errors_corrected: u32,
    /// Cells that were passed in as erasures.
    pub erasures: u32,
    /// Payload bytes. ECI, macro and mode codewords are not included.
    pub data: Vec<u8>,
    /// Charset of each byte range: ISO-8859-1 until an ECI says otherwise.
    pub segments: Vec<DataSegment>,
    pub macro_pdf417: Option<MacroPdf417>,
    /// The symbol starts with the reader initialisation codeword (921): its
    /// payload programs the reader and is not user data.
    pub reader_init: bool,
    /// ISO/IEC 15424 identifier: `]L1` when the symbol uses ECI, else `]L2`.
    pub aim_id: [u8; 3],
}

/// Check codewords that the correction budget keeps in reserve against
/// miscorrection: the code can fix `2 * errors + erasures <= ec`, we accept
/// `2 * errors + erasures <= ec - reserve`. Level 0 (2 check codewords) can
/// only detect.
pub(crate) fn reserve(ec: usize) -> usize {
    if ec <= 16 { 2 } else { 1 }
}

/// Decodes the data region of a symbol, row-major: `layout.rows` times
/// `layout.cols` codewords, with [`ERASED`] for unreadable cells.
///
/// Reed-Solomon correction is accepted only when
/// `2 * errors + erasures <= ec - reserve` (see [`reserve`]). The first
/// codeword must be the symbol length descriptor and equal the number of data
/// codewords, padding included.
pub(crate) fn decode_codewords(cells: &[u16], layout: Layout) -> Result<DecodedPdf417, Reject> {
    let n = layout.rows * layout.cols;
    let ec = spec::ec_codewords(layout.ec_level);
    if cells.len() != n || n > spec::MAX_CODEWORDS || n <= ec {
        return Err(Reject::Format);
    }
    let mut block = [0u16; spec::MAX_CODEWORDS];
    let block = &mut block[..n];
    let mut erasures = Vec::new();
    for (i, (b, &c)) in block.iter_mut().zip(cells).enumerate() {
        if usize::from(c) >= spec::CODEWORD_COUNT {
            erasures.push(i);
        } else {
            *b = c;
        }
    }
    let budget = ec - reserve(ec).min(ec);
    if erasures.len() > budget {
        return Err(Reject::ErrorCorrection);
    }
    let fixed = PDF417_FIELD
        .decode(block, ec, &erasures)
        .map_err(|_| Reject::ErrorCorrection)?;
    if 2 * fixed.errors + erasures.len() > budget {
        return Err(Reject::ErrorCorrection);
    }
    // The descriptor counts itself, the data and the padding.
    if usize::from(block[0]) != n - ec {
        return Err(Reject::Format);
    }
    let parsed = compaction::parse(&block[1..n - ec])?;
    let aim_modifier = if parsed.eci_used { b'1' } else { b'2' };
    Ok(DecodedPdf417 {
        layout,
        errors_corrected: fixed.errors as u32,
        erasures: erasures.len() as u32,
        data: parsed.data,
        segments: parsed.segments,
        macro_pdf417: parsed.macro_pdf417,
        reader_init: parsed.reader_init,
        aim_id: [b']', b'L', aim_modifier],
    })
}
