use breader_core::{Binarizer, BitMatrix, ImageView};

use crate::otsu::OtsuBinarizer;
use crate::pack::pack_row_with;

/// Block-adaptive binarizer: per-tile thresholds smoothed over a window of
/// neighbouring tiles. Pixels below the threshold are dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalBinarizer {
    /// Tile side in pixels.
    pub block: usize,
    /// Minimum max-min range for a tile to be considered textured.
    pub min_contrast: u8,
    /// Tile radius of the smoothing window (`2 * window + 1` tiles square).
    pub window: usize,
}

impl Default for LocalBinarizer {
    fn default() -> Self {
        Self {
            block: 8,
            min_contrast: 24,
            window: 2,
        }
    }
}

impl Binarizer for LocalBinarizer {
    fn id(&self) -> &'static str {
        "local"
    }

    fn binarize(&self, src: ImageView<'_>, out: &mut BitMatrix) {
        let (w, h) = (src.width(), src.height());
        let block = self.block.max(1);
        let (tw, th) = (w.div_ceil(block), h.div_ceil(block));
        let span = self.window.saturating_mul(2).saturating_add(1);
        if tw < span || th < span {
            OtsuBinarizer.binarize(src, out);
            return;
        }
        if out.width() != w || out.height() != h {
            out.resize(w, h);
        }

        let thresholds = self.tile_thresholds(src, block, tw, th);
        let sat = summed_area(&thresholds, tw, th);
        let sat_stride = tw + 1;

        let mut pixel_thr = vec![0u8; w];
        for ty in 0..th {
            let y0 = ty.saturating_sub(self.window);
            let y1 = (ty + self.window + 1).min(th);
            for tx in 0..tw {
                let x0 = tx.saturating_sub(self.window);
                let x1 = (tx + self.window + 1).min(tw);
                let sum = sat[y1 * sat_stride + x1] + sat[y0 * sat_stride + x0]
                    - sat[y0 * sat_stride + x1]
                    - sat[y1 * sat_stride + x0];
                let count = ((x1 - x0) * (y1 - y0)) as u64;
                let t = (sum / count) as u8;
                let px0 = tx * block;
                pixel_thr[px0..(px0 + block).min(w)].fill(t);
            }
            for y in ty * block..((ty + 1) * block).min(h) {
                pack_row_with(src.row(y), &pixel_thr, out.row_words_mut(y));
            }
        }
    }
}

impl LocalBinarizer {
    fn tile_thresholds(&self, src: ImageView<'_>, block: usize, tw: usize, th: usize) -> Vec<u8> {
        let (w, h) = (src.width(), src.height());
        let mut thresholds = vec![0u8; tw * th];
        let mut mins = vec![0u8; tw];
        let mut maxs = vec![0u8; tw];
        let mut sums = vec![0u64; tw];
        for ty in 0..th {
            mins.fill(u8::MAX);
            maxs.fill(0);
            sums.fill(0);
            let y_end = ((ty + 1) * block).min(h);
            for y in ty * block..y_end {
                let row = src.row(y);
                for (tx, chunk) in row.chunks(block).enumerate() {
                    let mut lo = mins[tx];
                    let mut hi = maxs[tx];
                    let mut sum = 0u64;
                    for &v in chunk {
                        lo = lo.min(v);
                        hi = hi.max(v);
                        sum += u64::from(v);
                    }
                    mins[tx] = lo;
                    maxs[tx] = hi;
                    sums[tx] += sum;
                }
            }
            let rows = y_end - ty * block;
            for tx in 0..tw {
                let cols = ((tx + 1) * block).min(w) - tx * block;
                let (lo, hi) = (mins[tx], maxs[tx]);
                let t = if hi - lo >= self.min_contrast {
                    (sums[tx] / (rows * cols) as u64) as u8
                } else {
                    let mut acc = 0u32;
                    let mut n = 0u32;
                    if ty > 0 {
                        acc += u32::from(thresholds[(ty - 1) * tw + tx]);
                        n += 1;
                    }
                    if tx > 0 {
                        acc += u32::from(thresholds[ty * tw + tx - 1]);
                        n += 1;
                    }
                    if ty > 0 && tx > 0 {
                        acc += u32::from(thresholds[(ty - 1) * tw + tx - 1]);
                        n += 1;
                    }
                    match acc.checked_div(n) {
                        Some(avg) if avg > u32::from(lo) => avg as u8,
                        _ => lo / 2,
                    }
                };
                thresholds[ty * tw + tx] = t;
            }
        }
        thresholds
    }
}

fn summed_area(grid: &[u8], tw: usize, th: usize) -> Vec<u64> {
    let stride = tw + 1;
    let mut sat = vec![0u64; stride * (th + 1)];
    for y in 0..th {
        let mut run = 0u64;
        for x in 0..tw {
            run += u64::from(grid[y * tw + x]);
            sat[(y + 1) * stride + x + 1] = sat[y * stride + x + 1] + run;
        }
    }
    sat
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<B: Binarizer>(b: &B, pixels: &[u8], w: usize, h: usize) -> BitMatrix {
        let view = ImageView::from_packed(pixels, w, h).unwrap();
        let mut out = BitMatrix::new(w, h);
        b.binarize(view, &mut out);
        out
    }

    #[test]
    fn id_and_defaults() {
        let b = LocalBinarizer::default();
        assert_eq!(b.id(), "local");
        assert_eq!((b.block, b.min_contrast, b.window), (8, 24, 2));
    }

    #[test]
    fn checkerboard_under_gradient() {
        let (w, h, cell) = (256usize, 128usize, 5usize);
        let mut pixels = vec![0u8; w * h];
        let mut truth = vec![false; w * h];
        for y in 0..h {
            for x in 0..w {
                let dark = ((x / cell) + (y / cell)) % 2 == 0;
                let base = 20 + x * 170 / (w - 1);
                let v = if dark { base } else { base + 60 };
                pixels[y * w + x] = v as u8;
                truth[y * w + x] = dark;
            }
        }
        let otsu = run(&OtsuBinarizer, &pixels, w, h);
        let local = run(&LocalBinarizer::default(), &pixels, w, h);
        let (mut ok_local, mut ok_otsu, mut n) = (0usize, 0usize, 0usize);
        for y in 16..h - 16 {
            for x in 16..w - 16 {
                n += 1;
                ok_local += usize::from(local.get(x, y) == truth[y * w + x]);
                ok_otsu += usize::from(otsu.get(x, y) == truth[y * w + x]);
            }
        }
        assert!(ok_local * 100 > n * 98, "local {ok_local}/{n}");
        assert!(ok_otsu * 100 < n * 90, "otsu {ok_otsu}/{n}");
    }

    #[test]
    fn uniform_is_all_light() {
        for v in [0u8, 1, 100, 255] {
            let out = run(&LocalBinarizer::default(), &vec![v; 100 * 90], 100, 90);
            assert!((0..90).all(|y| (0..100).all(|x| !out.get(x, y))), "{v}");
        }
    }

    #[test]
    fn large_dark_square_is_solid() {
        let (w, h, side) = (160usize, 160usize, 48usize);
        let (x0, y0) = (56usize, 56usize);
        let mut pixels = vec![230u8; w * h];
        for y in y0..y0 + side {
            for x in x0..x0 + side {
                pixels[y * w + x] = 30;
            }
        }
        let out = run(&LocalBinarizer::default(), &pixels, w, h);
        for y in y0 + 4..y0 + side - 4 {
            for x in x0 + 4..x0 + side - 4 {
                assert!(out.get(x, y), "({x},{y})");
            }
        }
        assert!(!out.get(5, 5));
        assert!(!out.get(w - 5, h - 5));
    }

    #[test]
    fn tiny_image_matches_otsu() {
        let (w, h) = (30usize, 20usize);
        let pixels: Vec<u8> = (0..w * h).map(|i| ((i * 37) % 251) as u8).collect();
        let a = run(&LocalBinarizer::default(), &pixels, w, h);
        let b = run(&OtsuBinarizer, &pixels, w, h);
        assert_eq!(a, b);
    }

    #[test]
    fn odd_widths_keep_padding_zero() {
        for (w, h) in [(83usize, 61usize), (131, 67), (64, 64), (100, 45)] {
            let pixels: Vec<u8> = (0..w * h)
                .map(|i| ((i * 91 + i / w * 13) % 200) as u8)
                .collect();
            for b in [
                &LocalBinarizer::default() as &dyn Binarizer,
                &OtsuBinarizer as &dyn Binarizer,
            ] {
                let view = ImageView::from_packed(&pixels, w, h).unwrap();
                let mut out = BitMatrix::new(w, h);
                b.binarize(view, &mut out);
                for y in 0..h {
                    let last = *out.row_words(y).last().unwrap();
                    let tail = w % 64;
                    if tail != 0 {
                        assert_eq!(last >> tail, 0, "{} {w}x{h} row {y}", b.id());
                    }
                }
            }
        }
    }

    #[test]
    fn dirty_output_is_overwritten() {
        let (w, h) = (90usize, 90usize);
        let pixels = vec![200u8; w * h];
        let mut out = BitMatrix::new(w, h);
        for y in 0..h {
            out.row_words_mut(y).fill(u64::MAX);
        }
        let view = ImageView::from_packed(&pixels, w, h).unwrap();
        LocalBinarizer::default().binarize(view, &mut out);
        assert!((0..h).all(|y| out.row_words(y).iter().all(|&x| x == 0)));
    }
}
