//! Entry points for fuzz targets. Not part of the public API.

use crate::build::{Bits, fit, render, render_rune};
use crate::decode::{self, text};
use breader_core::BitMatrix;

/// Decodes a `size` x `size` module grid whose cells are the bits of `bits`
/// (LSB first, row-major; missing bits are light). Returns the payload.
pub fn decode_grid(size: usize, bits: &[u8], mirrored: bool) -> Option<Vec<u8>> {
    if !(1..=160).contains(&size) {
        return None;
    }
    let mut grid = BitMatrix::new(size, size);
    for i in 0..size * size {
        if bits.get(i / 8).is_some_and(|b| b >> (i % 8) & 1 == 1) {
            grid.set(i % size, i / size, true);
        }
    }
    let grid = if mirrored {
        decode::transpose(&grid)
    } else {
        grid
    };
    decode::decode_grid(&grid, mirrored).ok().map(|d| d.data)
}

/// Decodes the high-level bit stream made of the bits of `bytes` (most
/// significant first). Returns the payload.
pub fn decode_bits(bytes: &[u8]) -> Option<Vec<u8>> {
    let bits: Vec<bool> = bytes
        .iter()
        .flat_map(|b| (0..8).rev().map(move |i| b >> i & 1 == 1))
        .collect();
    text::parse(&bits).ok().map(|p| p.data)
}

/// Builds a symbol around the bits of `stream` as the high-level bit stream
/// (stuffed, padded, with valid check words and mode message) and decodes
/// it. `rune` makes a Rune of that value instead. Reaches the codeword and
/// text layers, which random grids almost never do.
pub fn decode_stream(rune: Option<u8>, extra_ec: u8, stream: &[u8]) -> Option<Vec<u8>> {
    if let Some(v) = rune {
        let d = decode::decode_grid(&render_rune(v), false).ok()?;
        assert_eq!(d.rune, Some(v));
        return Some(d.data);
    }
    let mut bits = Bits::default();
    for &b in stream.iter().take(2000) {
        bits.push(u32::from(b), 8);
    }
    let (layout, words) = fit(&bits.0, 3 + usize::from(extra_ec))?;
    let d = decode::decode_grid(&render(layout, &words), false).ok()?;
    Some(d.data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Rng;

    #[test]
    fn arbitrary_input_does_not_panic() {
        let mut rng = Rng(5);
        for _ in 0..400 {
            let size = [11, 15, 19, 23, 27, 31, 75, 151][rng.below(8)];
            let bytes: Vec<u8> = (0..size * size / 8 + 1).map(|_| rng.next() as u8).collect();
            assert_eq!(decode_grid(size, &bytes, rng.below(2) == 0), None);
            let _ = decode_bits(&bytes[..rng.below(bytes.len())]);
            let _ = decode_stream(None, rng.below(5) as u8, &bytes);
        }
        assert_eq!(decode_grid(0, &[], false), None);
        assert_eq!(decode_grid(161, &[], false), None);
    }
}
