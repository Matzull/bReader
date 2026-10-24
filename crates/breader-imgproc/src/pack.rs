use crate::simd::lt_mask64;

/// Packs `row[i] < thresholds[i]` into LSB-first words. Bits past the row
/// end stay zero.
#[inline]
pub(crate) fn pack_row_with(row: &[u8], thresholds: &[u8], words: &mut [u64]) {
    let (vs, v) = row.as_chunks::<64>();
    let (ts, t) = thresholds.as_chunks::<64>();
    let mut ws = words.iter_mut();
    for ((v, t), word) in vs.iter().zip(ts).zip(&mut ws) {
        *word = lt_mask64(v, t);
    }
    if let (false, Some(word)) = (v.is_empty(), ws.next()) {
        let (mut vb, mut tb) = ([u8::MAX; 64], [0u8; 64]);
        vb[..v.len()].copy_from_slice(v);
        tb[..v.len()].copy_from_slice(&t[..v.len()]);
        *word = lt_mask64(&vb, &tb);
    }
}

/// Same as `pack_row_with` with one threshold for the whole row.
#[inline]
pub(crate) fn pack_row(row: &[u8], threshold: u8, words: &mut [u64]) {
    let tb = [threshold; 64];
    let (vs, v) = row.as_chunks::<64>();
    let mut ws = words.iter_mut();
    for (v, word) in vs.iter().zip(&mut ws) {
        *word = lt_mask64(v, &tb);
    }
    if let (false, Some(word)) = (v.is_empty(), ws.next()) {
        let mut vb = [u8::MAX; 64];
        vb[..v.len()].copy_from_slice(v);
        *word = lt_mask64(&vb, &tb);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_naive_for_odd_lengths() {
        for len in [1usize, 2, 15, 63, 64, 65, 127, 128, 130, 200] {
            let row: Vec<u8> = (0..len).map(|i| (i * 37 % 256) as u8).collect();
            let thr: Vec<u8> = (0..len).map(|i| (i * 11 % 256) as u8).collect();
            let n = len.div_ceil(64);
            let mut a = vec![u64::MAX; n];
            let mut b = vec![u64::MAX; n];
            pack_row_with(&row, &thr, &mut a);
            pack_row(&row, 100, &mut b);
            for i in 0..len {
                assert_eq!((a[i / 64] >> (i % 64)) & 1 == 1, row[i] < thr[i]);
                assert_eq!((b[i / 64] >> (i % 64)) & 1 == 1, row[i] < 100);
            }
            if len % 64 != 0 {
                assert_eq!(a[n - 1] >> (len % 64), 0);
                assert_eq!(b[n - 1] >> (len % 64), 0);
            }
        }
    }
}
