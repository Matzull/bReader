//! The only module of this crate that contains `unsafe`.
//!
//! x86_64 has SSE2 in its baseline, so no runtime detection is needed.
//! Other targets use the scalar functions (autovectorized by the compiler).

#![allow(unsafe_code)]

/// Bit `i` is set when `v[i] < t[i]`.
#[inline]
pub(crate) fn lt_mask64(v: &[u8; 64], t: &[u8; 64]) -> u64 {
    #[cfg(target_arch = "x86_64")]
    {
        x86::lt_mask64(v, t)
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        lt_mask64_scalar(v, t)
    }
}

#[inline]
#[cfg_attr(target_arch = "x86_64", cfg(test))]
pub(crate) fn lt_mask64_scalar(v: &[u8; 64], t: &[u8; 64]) -> u64 {
    let mut acc = 0u64;
    for i in 0..64 {
        acc |= u64::from(v[i] < t[i]) << i;
    }
    acc
}

/// Luma of 4-byte pixels whose channel order is `[c0, G, c2, _]`;
/// `w0` and `w2` are the weights of `c0` and `c2` (RGBA: 77, 29).
/// `src.len() == 4 * dst.len()`.
#[inline]
pub(crate) fn luma4_row(src: &[u8], dst: &mut [u8], w0: u16, w2: u16) {
    debug_assert_eq!(src.len(), dst.len() * 4);
    #[cfg(target_arch = "x86_64")]
    let done = x86::luma4_blocks(src, dst, w0, w2);
    #[cfg(not(target_arch = "x86_64"))]
    let done = 0;
    luma4_row_scalar(&src[done * 4..], &mut dst[done..], w0, w2);
}

#[inline]
pub(crate) fn luma4_row_scalar(src: &[u8], dst: &mut [u8], w0: u16, w2: u16) {
    for (o, px) in dst.iter_mut().zip(src.chunks_exact(4)) {
        *o = ((w0 * u16::from(px[0]) + 150 * u16::from(px[1]) + w2 * u16::from(px[2]) + 128) >> 8)
            as u8;
    }
}

#[cfg(target_arch = "x86_64")]
mod x86 {
    use std::arch::x86_64::{
        __m128i, _mm_add_epi32, _mm_and_si128, _mm_cmpeq_epi8, _mm_loadu_si128, _mm_madd_epi16,
        _mm_movemask_epi8, _mm_packs_epi32, _mm_packus_epi16, _mm_set1_epi32, _mm_setzero_si128,
        _mm_srli_epi16, _mm_srli_epi32, _mm_storeu_si128, _mm_subs_epu8,
    };

    #[inline]
    pub(super) fn lt_mask64(v: &[u8; 64], t: &[u8; 64]) -> u64 {
        let mut acc = 0u64;
        for (i, (vc, tc)) in v.chunks_exact(16).zip(t.chunks_exact(16)).enumerate() {
            // SAFETY: SSE2 is part of the x86_64 baseline. `vc` and `tc` are
            // exactly 16 readable bytes (chunks_exact(16) of 64-byte arrays)
            // and `_mm_loadu_si128` has no alignment requirement.
            let m = unsafe {
                let vv: __m128i = _mm_loadu_si128(vc.as_ptr().cast());
                let tv: __m128i = _mm_loadu_si128(tc.as_ptr().cast());
                // t - v saturates to zero exactly when v >= t.
                let ge = _mm_cmpeq_epi8(_mm_subs_epu8(tv, vv), _mm_setzero_si128());
                !(_mm_movemask_epi8(ge) as u32) & 0xFFFF
            };
            acc |= u64::from(m) << (16 * i);
        }
        acc
    }
    /// Converts whole blocks of 16 pixels and returns the pixel count done.
    pub(super) fn luma4_blocks(src: &[u8], dst: &mut [u8], w0: u16, w2: u16) -> usize {
        let blocks = src.chunks_exact(64).zip(dst.chunks_exact_mut(16));
        let mut done = 0;
        for (s, d) in blocks {
            // SAFETY: SSE2 is part of the x86_64 baseline. `s` is 64
            // readable bytes (four 16-byte loads) and `d` is 16 writable
            // bytes (one 16-byte store); both intrinsics are unaligned.
            unsafe {
                let mask = _mm_set1_epi32(0x00FF_00FF);
                // 16-bit lane pairs are (c0, c2) and (G, A).
                let w02 = _mm_set1_epi32(i32::from(w2) << 16 | i32::from(w0));
                let wg = _mm_set1_epi32(150);
                let round = _mm_set1_epi32(128);
                let mut y = [_mm_setzero_si128(); 4];
                for (i, out) in y.iter_mut().enumerate() {
                    let v: __m128i = _mm_loadu_si128(s.as_ptr().add(16 * i).cast());
                    let c02 = _mm_and_si128(v, mask);
                    let ga = _mm_and_si128(_mm_srli_epi16(v, 8), mask);
                    let sum = _mm_add_epi32(_mm_madd_epi16(c02, w02), _mm_madd_epi16(ga, wg));
                    *out = _mm_srli_epi32(_mm_add_epi32(sum, round), 8);
                }
                let lo = _mm_packs_epi32(y[0], y[1]);
                let hi = _mm_packs_epi32(y[2], y[3]);
                _mm_storeu_si128(d.as_mut_ptr().cast(), _mm_packus_epi16(lo, hi));
            }
            done += 16;
        }
        done
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luma4_matches_scalar() {
        let mut state = 0x1234_5678_9ABC_DEF1u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for n in [0usize, 1, 15, 16, 17, 31, 32, 33, 100, 257] {
            for (w0, w2) in [(77u16, 29u16), (29, 77)] {
                for fill in [None, Some(0u8), Some(255)] {
                    let src: Vec<u8> = (0..n * 4)
                        .map(|_| fill.unwrap_or_else(|| (next() >> 11) as u8))
                        .collect();
                    let mut a = vec![0u8; n];
                    let mut b = vec![0u8; n];
                    luma4_row(&src, &mut a, w0, w2);
                    luma4_row_scalar(&src, &mut b, w0, w2);
                    assert_eq!(a, b, "n={n} w0={w0}");
                }
            }
        }
    }

    #[test]
    fn mask_matches_scalar() {
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for round in 0..2000 {
            let mut v = [0u8; 64];
            let mut t = [0u8; 64];
            for i in 0..64 {
                let r = next();
                // Bias towards equal and extreme values.
                v[i] = match r % 5 {
                    0 => 0,
                    1 => 255,
                    _ => (r >> 8) as u8,
                };
                t[i] = match (r >> 16) % 4 {
                    0 => v[i],
                    1 => v[i].wrapping_add(1),
                    _ => (r >> 24) as u8,
                };
            }
            assert_eq!(lt_mask64(&v, &t), lt_mask64_scalar(&v, &t), "round {round}");
        }
    }
}
