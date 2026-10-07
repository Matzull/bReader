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
        let hist = histogram(src);
        let total = (src.width() * src.height()) as u64;
        let threshold = otsu_threshold(&hist, total);
        if out.width() != src.width() || out.height() != src.height() {
            out.resize(src.width(), src.height());
        }
        for y in 0..src.height() {
            pack_row(src.row(y), threshold, out.row_words_mut(y));
        }
    }
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
