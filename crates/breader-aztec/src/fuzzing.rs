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

struct Threshold;

impl breader_core::Binarizer for Threshold {
    fn id(&self) -> &'static str {
        "threshold"
    }
    fn binarize(&self, src: breader_core::ImageView<'_>, out: &mut BitMatrix) {
        for (y, row) in src.rows().enumerate() {
            for (x, &p) in row.iter().enumerate() {
                out.set(x, y, p < 128);
            }
        }
    }
}

/// Runs the whole reader on an image. With `valid` the image is a symbol
/// built around the bits of `bytes` (3 pixels per module, a margin around it)
/// whose pixels are flipped where `noise` says so; without it `bytes` are the
/// pixels, `width` per row. Returns the number of symbols read, whose payloads
/// are never empty.
pub fn scan_image(valid: bool, effort: u8, width: usize, noise: &[u8], bytes: &[u8]) -> usize {
    use breader_core::{Effort, ImageView, Pipeline, ScanOptions, Scratch};
    let (w, luma) = if valid {
        let mut bits = Bits::default();
        for &b in bytes.iter().take(300) {
            bits.push(u32::from(b), 8);
        }
        let Some((layout, words)) = fit(&bits.0, 3) else {
            return 0;
        };
        let grid = render(layout, &words);
        let n = grid.width();
        let w = (n + 6) * 3;
        let mut luma = vec![235u8; w * w];
        for y in 0..n * 3 {
            for x in 0..n * 3 {
                if grid.get(x / 3, y / 3) {
                    luma[(y + 9) * w + x + 9] = 25;
                }
            }
        }
        for (i, &flip) in noise.iter().take(256).enumerate() {
            let at = (usize::from(flip) * 251 + i * 7919) % (w * w);
            luma[at] = 255 - luma[at];
        }
        (w, luma)
    } else {
        let w = width.clamp(1, 256);
        let mut luma = bytes.to_vec();
        luma.truncate(luma.len() / w * w);
        (w, luma)
    };
    let h = luma.len() / w;
    let Ok(img) = ImageView::from_packed(&luma, w, h) else {
        return 0;
    };
    let effort = match effort % 3 {
        0 => Effort::Fast,
        1 => Effort::Balanced,
        _ => Effort::Exhaustive,
    };
    let pipeline = Pipeline::builder()
        .binarizer(Threshold)
        .reader(crate::AztecReader)
        .build()
        .expect("a pipeline with a binarizer");
    let found = pipeline.scan(img, &ScanOptions::with_effort(effort), &mut Scratch::new());
    assert!(found.iter().all(|s| !s.data.is_empty()));
    found.len()
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
            if rng.below(8) == 0 {
                for valid in [false, true] {
                    for effort in 0..3 {
                        let n = scan_image(valid, effort, 40 + usize::from(effort), &bytes, &bytes);
                        assert!(n <= 1);
                    }
                }
            }
        }
        assert_eq!(decode_grid(0, &[], false), None);
        assert_eq!(decode_grid(161, &[], false), None);
    }

    /// Fuzz artifacts: an empty stream has no symbol (the mode message counts
    /// the data codewords less one), it must not underflow in the builder.
    #[test]
    fn an_empty_stream_builds_no_symbol() {
        assert!(fit(&[], 3).is_none());
        assert_eq!(decode_stream(None, 4, &[]), None);
        assert_eq!(scan_image(true, 0, 0, &[], &[]), 0);
        assert_eq!(scan_image(true, 2, 0, &[0x6c], &[]), 0);
    }
}
