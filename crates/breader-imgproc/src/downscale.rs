use std::cell::Cell;

use breader_core::{ImageBuffer, ImageView, Preprocessor};

const MIN_OUTPUT: usize = 32;

/// Box-filter reduction by an integer factor; partial edge blocks are
/// dropped. Images that would shrink below 32 px are copied unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Downscale {
    factor: u8,
}

impl Downscale {
    /// `factor` is clamped to `2..=4`.
    pub fn new(factor: u8) -> Self {
        Self {
            factor: factor.clamp(2, 4),
        }
    }

    pub const fn factor(&self) -> u8 {
        self.factor
    }
}

impl Preprocessor for Downscale {
    fn id(&self) -> &'static str {
        match self.factor {
            2 => "downscale2",
            3 => "downscale3",
            _ => "downscale4",
        }
    }

    fn apply(&self, src: ImageView<'_>, dst: &mut ImageBuffer) {
        let f = usize::from(self.factor);
        let (ow, oh) = (src.width() / f, src.height() / f);
        if ow < MIN_OUTPUT || oh < MIN_OUTPUT {
            dst.resize(src.width(), src.height());
            for y in 0..src.height() {
                dst.row_mut(y).copy_from_slice(src.row(y));
            }
            return;
        }
        dst.resize(ow, oh);
        match f {
            2 => reduce::<2>(src, dst, ow, oh),
            3 => reduce::<3>(src, dst, ow, oh),
            _ => reduce::<4>(src, dst, ow, oh),
        }
    }
}

/// Sums the `F` rows of each output row vertically in `u16` (at most
/// 4 * 255), then reduces `F` neighbouring sums with rounding. The constant
/// factor lets the compiler turn the division into shifts or multiplies.
fn reduce<const F: usize>(src: ImageView<'_>, dst: &mut ImageBuffer, ow: usize, oh: usize) {
    #[cfg(feature = "rayon")]
    if crate::is_large(src.width(), src.height()) {
        use rayon::prelude::*;
        dst.as_mut_slice()
            .par_chunks_mut(ow * crate::PAR_BAND_ROWS)
            .enumerate()
            .for_each_init(
                || vec![0u16; ow * F],
                |vsum, (band, chunk)| {
                    for (i, out) in chunk.chunks_mut(ow).enumerate() {
                        reduce_row::<F>(src, band * crate::PAR_BAND_ROWS + i, out, vsum);
                    }
                },
            );
        return;
    }
    let mut vsum = VSUM.take();
    vsum.clear();
    vsum.resize(ow * F, 0);
    for oy in 0..oh {
        reduce_row::<F>(src, oy, dst.row_mut(oy), &mut vsum);
    }
    VSUM.set(vsum);
}

thread_local! {
    /// Vertical sums of one output row, kept so that scanning does not allocate.
    static VSUM: Cell<Vec<u16>> = const { Cell::new(Vec::new()) };
}

#[inline(always)]
fn reduce_row<const F: usize>(src: ImageView<'_>, oy: usize, out: &mut [u8], vsum: &mut [u16]) {
    let area = (F * F) as u32;
    let half = area / 2;
    let width = out.len() * F;
    let first = &src.row(oy * F)[..width];
    for (s, &v) in vsum.iter_mut().zip(first) {
        *s = u16::from(v);
    }
    for dy in 1..F {
        let row = &src.row(oy * F + dy)[..width];
        for (s, &v) in vsum.iter_mut().zip(row) {
            *s += u16::from(v);
        }
    }
    for (o, group) in out.iter_mut().zip(vsum.chunks_exact(F)) {
        let sum: u32 = group.iter().map(|&v| u32::from(v)).sum();
        *o = ((sum + half) / area) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(d: Downscale, pixels: &[u8], w: usize, h: usize) -> ImageBuffer {
        let mut dst = ImageBuffer::default();
        d.apply(ImageView::from_packed(pixels, w, h).unwrap(), &mut dst);
        dst
    }

    #[test]
    fn ids_and_clamping() {
        assert_eq!(Downscale::new(0).factor(), 2);
        assert_eq!(Downscale::new(9).factor(), 4);
        assert_eq!(Downscale::new(2).id(), "downscale2");
        assert_eq!(Downscale::new(3).id(), "downscale3");
        assert_eq!(Downscale::new(4).id(), "downscale4");
    }

    #[test]
    fn exact_block_averages() {
        let (w, h) = (128usize, 64usize);
        let mut pixels = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                pixels[y * w + x] = match (x % 2, y % 2) {
                    (0, 0) => 10,
                    (1, 0) => 20,
                    (0, 1) => 30,
                    _ => 41,
                };
            }
        }
        let out = run(Downscale::new(2), &pixels, w, h);
        assert_eq!((out.width(), out.height()), (64, 32));
        let view = out.view();
        assert!(view.rows().flatten().all(|&v| v == 25));
    }

    #[test]
    fn factor_three_with_odd_sizes() {
        let (w, h) = (100usize, 101usize);
        let pixels: Vec<u8> = (0..w * h).map(|i| (i % 256) as u8).collect();
        let out = run(Downscale::new(3), &pixels, w, h);
        assert_eq!((out.width(), out.height()), (33, 33));
        let view = out.view();
        let mut sum = 0u32;
        for dy in 0..3 {
            for dx in 0..3 {
                sum += u32::from(pixels[(3 + dy) * w + 6 + dx]);
            }
        }
        assert_eq!(u32::from(view.get(2, 1)), (sum + 4) / 9);
    }

    #[test]
    fn factor_four_gradient() {
        let (w, h) = (130usize, 129usize);
        let pixels: Vec<u8> = (0..w * h).map(|i| (i % w) as u8).collect();
        let out = run(Downscale::new(4), &pixels, w, h);
        assert_eq!((out.width(), out.height()), (32, 32));
        assert_eq!(out.view().get(5, 7), 22);
    }

    #[test]
    fn tiny_output_copies_source() {
        let (w, h) = (63usize, 90usize);
        let pixels: Vec<u8> = (0..w * h).map(|i| (i * 7 % 256) as u8).collect();
        let out = run(Downscale::new(2), &pixels, w, h);
        assert_eq!((out.width(), out.height()), (w, h));
        let copy: Vec<u8> = out.view().rows().flatten().copied().collect();
        assert_eq!(copy, pixels);
    }
}
