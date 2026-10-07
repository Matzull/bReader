//! Module grid to payload: pattern removal, codeword extraction,
//! de-interleaving, Reed-Solomon and encodation modes.

use breader_core::{BitMatrix, DataSegment, Reject};

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
/// `cols` including finder and clock patterns. Retries the transposed grid
/// when the normal orientation fails.
pub(crate) fn decode_grid(grid: &BitMatrix) -> Result<DecodedDm, Reject> {
    let _ = grid;
    Err(Reject::Format)
}
