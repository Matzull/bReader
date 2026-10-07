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

        let grid = Grid {
            sat: &sat,
            stride: sat_stride,
            tw,
            th,
        };
        #[cfg(feature = "rayon")]
        if crate::is_large(w, h) {
            use rayon::prelude::*;
            let (words, wpr) = out.words_mut();
            words
                .par_chunks_mut(wpr.max(1) * block)
                .enumerate()
                .for_each_init(
                    || vec![0u8; w],
                    |pixel_thr, (ty, chunk)| {
                        self.pack_tile_row(src, block, ty, &grid, pixel_thr, chunk, wpr.max(1));
                    },
                );
            return;
        }
        let mut pixel_thr = vec![0u8; w];
        let (words, wpr) = out.words_mut();
        for (ty, chunk) in words.chunks_mut(wpr.max(1) * block).enumerate() {
            self.pack_tile_row(src, block, ty, &grid, &mut pixel_thr, chunk, wpr.max(1));
        }
    }
}

/// Summed-area table of the tile thresholds.
struct Grid<'a> {
    sat: &'a [u64],
    stride: usize,
    tw: usize,
    th: usize,
}

impl LocalBinarizer {
    /// Smooths the thresholds over the window and packs the pixel rows of
    /// tile row `ty` into `chunk`, which holds those rows' words.
    #[allow(clippy::too_many_arguments)]
    fn pack_tile_row(
        &self,
        src: ImageView<'_>,
        block: usize,
        ty: usize,
        grid: &Grid<'_>,
        pixel_thr: &mut [u8],
        chunk: &mut [u64],
        wpr: usize,
    ) {
        let (w, h) = (src.width(), src.height());
        let (sat, sat_stride, tw, th) = (grid.sat, grid.stride, grid.tw, grid.th);
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
        for (i, words) in chunk.chunks_mut(wpr).enumerate() {
            let y = ty * block + i;
            if y < h {
                pack_row_with(src.row(y), pixel_thr, words);
            }
        }
    }
}

/// Per-column and per-tile accumulators for one tile row.
#[derive(Debug, Default)]
struct RowStats {
    col_min: Vec<u8>,
    col_max: Vec<u8>,
    col_sum: Vec<u16>,
}

impl RowStats {
    fn new(w: usize) -> Self {
        Self {
            col_min: vec![0; w],
            col_max: vec![0; w],
            col_sum: vec![0; w],
        }
    }

    /// Min, max and sum of every tile of tile row `ty`.
    fn tiles(
        &mut self,
        src: ImageView<'_>,
        block: usize,
        ty: usize,
        mins: &mut [u8],
        maxs: &mut [u8],
        sums: &mut [u64],
    ) {
        let h = src.height();
        mins.fill(u8::MAX);
        maxs.fill(0);
        sums.fill(0);
        let y_end = ((ty + 1) * block).min(h);
        // Bands of at most 256 rows keep the `u16` column sums exact.
        for band_start in (ty * block..y_end).step_by(256) {
            let band_end = (band_start + 256).min(y_end);
            // Vertical pass: per-column statistics over the band rows.
            let first = src.row(band_start);
            self.col_min.copy_from_slice(first);
            self.col_max.copy_from_slice(first);
            for (s, &v) in self.col_sum.iter_mut().zip(first) {
                *s = u16::from(v);
            }
            for y in band_start + 1..band_end {
                let row = src.row(y);
                for (((lo, hi), s), &v) in self
                    .col_min
                    .iter_mut()
                    .zip(self.col_max.iter_mut())
                    .zip(self.col_sum.iter_mut())
                    .zip(row)
                {
                    *lo = (*lo).min(v);
                    *hi = (*hi).max(v);
                    *s += u16::from(v);
                }
            }
            // Horizontal pass: reduce each tile's columns.
            let stats = self
                .col_min
                .chunks(block)
                .zip(self.col_max.chunks(block))
                .zip(self.col_sum.chunks(block));
            for (tx, ((lo, hi), sum)) in stats.enumerate() {
                let (lo, hi, sum) = reduce_columns(lo, hi, sum);
                mins[tx] = mins[tx].min(lo);
                maxs[tx] = maxs[tx].max(hi);
                sums[tx] += sum;
            }
        }
    }
}

impl LocalBinarizer {
    /// Thresholds of tile row `ty` from its statistics. Tiles without
    /// contrast borrow from the already computed neighbours above and left.
    #[allow(clippy::too_many_arguments)]
    fn row_thresholds(
        &self,
        src: ImageView<'_>,
        block: usize,
        ty: usize,
        stats: (&[u8], &[u8], &[u64]),
        above: Option<&[u8]>,
        row: &mut [u8],
    ) {
        let (w, h) = (src.width(), src.height());
        let (mins, maxs, sums) = stats;
        let rows = ((ty + 1) * block).min(h) - ty * block;
        for tx in 0..row.len() {
            let cols = ((tx + 1) * block).min(w) - tx * block;
            let (lo, hi) = (mins[tx], maxs[tx]);
            let t = if hi - lo >= self.min_contrast {
                (sums[tx] / (rows * cols) as u64) as u8
            } else {
                let mut acc = 0u32;
                let mut n = 0u32;
                if let Some(above) = above {
                    acc += u32::from(above[tx]);
                    n += 1;
                }
                if tx > 0 {
                    acc += u32::from(row[tx - 1]);
                    n += 1;
                }
                if let (Some(above), true) = (above, tx > 0) {
                    acc += u32::from(above[tx - 1]);
                    n += 1;
                }
                match acc.checked_div(n) {
                    Some(avg) if avg > u32::from(lo) => avg as u8,
                    _ => lo / 2,
                }
            };
            row[tx] = t;
        }
    }

    fn tile_thresholds(&self, src: ImageView<'_>, block: usize, tw: usize, th: usize) -> Vec<u8> {
        let mut thresholds = vec![0u8; tw * th];
        #[cfg(feature = "rayon")]
        if crate::is_large(src.width(), src.height()) {
            use rayon::prelude::*;
            // Statistics of different tile rows are independent; only the
            // fallback for flat tiles needs the previous row, so that part
            // runs in order afterwards.
            let mut mins = vec![0u8; tw * th];
            let mut maxs = vec![0u8; tw * th];
            let mut sums = vec![0u64; tw * th];
            mins.par_chunks_mut(tw)
                .zip(maxs.par_chunks_mut(tw))
                .zip(sums.par_chunks_mut(tw))
                .enumerate()
                .for_each_init(
                    || RowStats::new(src.width()),
                    |scratch, (ty, ((mn, mx), sm))| scratch.tiles(src, block, ty, mn, mx, sm),
                );
            for ty in 0..th {
                let (done, rest) = thresholds.split_at_mut(ty * tw);
                let above = (ty > 0).then(|| &done[(ty - 1) * tw..]);
                let r = ty * tw..(ty + 1) * tw;
                let stats = (&mins[r.clone()], &maxs[r.clone()], &sums[r]);
                self.row_thresholds(src, block, ty, stats, above, &mut rest[..tw]);
            }
            return thresholds;
        }
        let mut scratch = RowStats::new(src.width());
        let mut mins = vec![0u8; tw];
        let mut maxs = vec![0u8; tw];
        let mut sums = vec![0u64; tw];
        for ty in 0..th {
            scratch.tiles(src, block, ty, &mut mins, &mut maxs, &mut sums);
            let (done, rest) = thresholds.split_at_mut(ty * tw);
            let above = (ty > 0).then(|| &done[(ty - 1) * tw..]);
            self.row_thresholds(
                src,
                block,
                ty,
                (&mins, &maxs, &sums),
                above,
                &mut rest[..tw],
            );
        }
        thresholds
    }
}

/// Min, max and sum over one tile's column statistics.
#[inline]
fn reduce_columns(lo: &[u8], hi: &[u8], sum: &[u16]) -> (u8, u8, u64) {
    if let (Ok(lo), Ok(hi), Ok(sum)) = (
        <&[u8; 8]>::try_from(lo),
        <&[u8; 8]>::try_from(hi),
        <&[u16; 8]>::try_from(sum),
    ) {
        let min = lo.iter().fold(u8::MAX, |a, &b| a.min(b));
        let max = hi.iter().fold(0, |a, &b| a.max(b));
        let total = sum.iter().map(|&s| u32::from(s)).sum::<u32>();
        return (min, max, u64::from(total));
    }
    (
        lo.iter().copied().min().unwrap_or(u8::MAX),
        hi.iter().copied().max().unwrap_or(0),
        sum.iter().map(|&s| u64::from(s)).sum(),
    )
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
