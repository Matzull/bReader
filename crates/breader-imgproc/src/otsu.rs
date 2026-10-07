use breader_core::{Binarizer, BitMatrix, ImageView};

use crate::pack::pack_row;

/// Global Otsu threshold; pixels below the threshold are dark.
#[derive(Debug, Clone, Copy, Default)]
pub struct OtsuBinarizer;

fn otsu_threshold(hist: &[u64; 256], total: u64) -> u8 {
    let sum_all: u64 = hist.iter().zip(0u64..).map(|(&c, v)| c * v).sum();
    let mut weight_dark = 0u64;
    let mut sum_dark = 0u64;
    let mut best = 0.0f64;
    let mut threshold = 0u8;
    for t in 1..=255usize {
        weight_dark += hist[t - 1];
        sum_dark += hist[t - 1] * (t as u64 - 1);
        let weight_light = total - weight_dark;
        if weight_dark == 0 || weight_light == 0 {
            continue;
        }
        let mean_dark = sum_dark as f64 / weight_dark as f64;
        let mean_light = (sum_all - sum_dark) as f64 / weight_light as f64;
        let diff = mean_dark - mean_light;
        let variance = weight_dark as f64 * weight_light as f64 * diff * diff;
        if variance > best {
            best = variance;
            threshold = t as u8;
        }
    }
    threshold
}

/// Four interleaved sub-histograms break the store-to-load dependency chain
/// on repeated values.
fn histogram(src: ImageView<'_>) -> [u64; 256] {
    const FLUSH_AT: usize = 1 << 30;
    let mut hist = [0u64; 256];
    let mut sub = [[0u32; 256]; 4];
    let mut pending = 0usize;
    let flush = |sub: &mut [[u32; 256]; 4], hist: &mut [u64; 256]| {
        for s in sub.iter_mut() {
            for (h, c) in hist.iter_mut().zip(s.iter_mut()) {
                *h += u64::from(*c);
                *c = 0;
            }
        }
    };
    for row in src.rows() {
        let mut chunks = row.chunks_exact(4);
        for c in &mut chunks {
            sub[0][usize::from(c[0])] += 1;
            sub[1][usize::from(c[1])] += 1;
            sub[2][usize::from(c[2])] += 1;
            sub[3][usize::from(c[3])] += 1;
        }
        for &v in chunks.remainder() {
            sub[0][usize::from(v)] += 1;
        }
        pending += row.len();
        if pending >= FLUSH_AT {
            flush(&mut sub, &mut hist);
            pending = 0;
        }
    }
    flush(&mut sub, &mut hist);
    hist
}

impl Binarizer for OtsuBinarizer {
    fn id(&self) -> &'static str {
        "otsu"
    }

    fn binarize(&self, src: ImageView<'_>, out: &mut BitMatrix) {
        let hist = frame_histogram(src);
        let total = (src.width() * src.height()) as u64;
        let threshold = otsu_threshold(&hist, total);
        if out.width() != src.width() || out.height() != src.height() {
            out.resize(src.width(), src.height());
        }
        #[cfg(feature = "rayon")]
        if is_large(&src) {
            use rayon::prelude::*;
            let (words, wpr) = out.words_mut();
            words
                .par_chunks_mut(wpr.max(1) * crate::PAR_BAND_ROWS)
                .enumerate()
                .for_each(|(band, chunk)| {
                    let y0 = band * crate::PAR_BAND_ROWS;
                    for (i, row) in chunk.chunks_mut(wpr.max(1)).enumerate() {
                        pack_row(src.row(y0 + i), threshold, row);
                    }
                });
            return;
        }
        for y in 0..src.height() {
            pack_row(src.row(y), threshold, out.row_words_mut(y));
        }
    }
}

#[cfg(feature = "rayon")]
fn is_large(src: &ImageView<'_>) -> bool {
    crate::is_large(src.width(), src.height())
}

/// Histogram of the whole frame; large frames are counted in row bands and
/// the integer counts merged, which gives the same result in any order.
fn frame_histogram(src: ImageView<'_>) -> [u64; 256] {
    #[cfg(feature = "rayon")]
    if is_large(&src) {
        use rayon::prelude::*;
        let bands = src.height().div_ceil(crate::PAR_BAND_ROWS);
        return (0..bands)
            .into_par_iter()
            .map(|b| {
                let y0 = b * crate::PAR_BAND_ROWS;
                let rows = crate::PAR_BAND_ROWS.min(src.height() - y0);
                src.crop(0, y0, src.width(), rows)
                    .map_or([0u64; 256], histogram)
            })
            .reduce(
                || [0u64; 256],
                |mut a, b| {
                    for (x, y) in a.iter_mut().zip(b) {
                        *x += y;
                    }
                    a
                },
            );
    }
    histogram(src)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bimodal_split() {
        let (w, h) = (8, 4);
        let pixels: Vec<u8> = (0..w * h)
            .map(|i| if i % w < w / 2 { 40 } else { 210 })
            .collect();
        let view = ImageView::from_packed(&pixels, w, h).unwrap();
        let mut out = BitMatrix::new(w, h);
        OtsuBinarizer.binarize(view, &mut out);
        for y in 0..h {
            for x in 0..w {
                assert_eq!(out.get(x, y), x < w / 2);
            }
        }
    }

    #[test]
    fn uniform_images_are_all_light() {
        for v in [0u8, 128, 255] {
            let pixels = vec![v; 16];
            let view = ImageView::from_packed(&pixels, 4, 4).unwrap();
            let mut out = BitMatrix::new(4, 4);
            OtsuBinarizer.binarize(view, &mut out);
            assert!((0..4).all(|y| (0..4).all(|x| !out.get(x, y))));
        }
    }

    #[test]
    fn id() {
        assert_eq!(OtsuBinarizer.id(), "otsu");
    }
}
