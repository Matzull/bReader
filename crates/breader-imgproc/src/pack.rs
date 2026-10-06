/// Packs `row[i] < threshold(i)` into LSB-first words. Bits past the row
/// end stay zero.
#[inline]
pub(crate) fn pack_row(row: &[u8], threshold: impl Fn(usize) -> u8, words: &mut [u64]) {
    for (wi, (chunk, word)) in row.chunks(64).zip(words.iter_mut()).enumerate() {
        let base = wi * 64;
        let mut acc = 0u64;
        for (i, &v) in chunk.iter().enumerate() {
            acc |= u64::from(v < threshold(base + i)) << i;
        }
        *word = acc;
    }
}

/// Same as `pack_row` with a per-pixel threshold row of equal length.
#[inline]
pub(crate) fn pack_row_with(row: &[u8], thresholds: &[u8], words: &mut [u64]) {
    for ((chunk, tchunk), word) in row
        .chunks(64)
        .zip(thresholds.chunks(64))
        .zip(words.iter_mut())
    {
        let mut acc = 0u64;
        for (i, (&v, &t)) in chunk.iter().zip(tchunk).enumerate() {
            acc |= u64::from(v < t) << i;
        }
        *word = acc;
    }
}
