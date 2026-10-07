//! Pixel format conversion, preprocessing and binarization.

mod downscale;
mod local;
mod luma;
mod otsu;
mod pack;
mod simd;

pub use downscale::Downscale;
pub use local::LocalBinarizer;
pub use luma::{luma_view, to_luma};
pub use otsu::OtsuBinarizer;

/// Images below this many pixels stay single-threaded: dispatch costs more
/// than the kernels save.
#[cfg(feature = "rayon")]
pub(crate) const PAR_MIN_PIXELS: usize = 1 << 20;

/// Rows per parallel work item.
#[cfg(feature = "rayon")]
pub(crate) const PAR_BAND_ROWS: usize = 64;

/// Whether a frame of this size takes the parallel path.
#[cfg(feature = "rayon")]
pub(crate) fn is_large(width: usize, height: usize) -> bool {
    width * height >= PAR_MIN_PIXELS
        && rayon::current_num_threads() > 1
        && !breader_core::parallel::is_sequential()
}

#[cfg(all(test, feature = "rayon"))]
mod parallel_tests {
    use super::*;
    use breader_core::{Binarizer, BitMatrix, ImageBuffer, ImageView, Preprocessor, parallel};

    fn picture(w: usize, h: usize) -> Vec<u8> {
        (0..w * h)
            .map(|i| {
                let (x, y) = (i % w, i / w);
                let cell = ((x / 7 + y / 5) % 2) * 90;
                let flat = if (y / 64) % 3 == 0 { 0 } else { cell };
                (30 + flat + (x * 40 / w) + ((i * 2654435761usize) >> 28) % 9) as u8
            })
            .collect()
    }

    #[test]
    fn parallel_kernels_equal_sequential() {
        for (w, h) in [(1500usize, 900usize), (1237, 1111)] {
            assert!(is_large(w, h));
            let pixels = picture(w, h);
            let view = ImageView::from_packed(&pixels, w, h).unwrap();
            let binarizers: [&dyn Binarizer; 2] = [&LocalBinarizer::default(), &OtsuBinarizer];
            for b in binarizers {
                let (mut par, mut seq) = (BitMatrix::default(), BitMatrix::default());
                b.binarize(view, &mut par);
                parallel::sequential(|| b.binarize(view, &mut seq));
                assert_eq!(par, seq, "{} {w}x{h}", b.id());
            }
            for f in 2..=4 {
                let d = Downscale::new(f);
                let (mut par, mut seq) = (ImageBuffer::default(), ImageBuffer::default());
                d.apply(view, &mut par);
                parallel::sequential(|| d.apply(view, &mut seq));
                let (a, b) = (par.view(), seq.view());
                assert_eq!((a.width(), a.height()), (b.width(), b.height()));
                assert!(a.rows().eq(b.rows()), "{} {w}x{h}", d.id());
            }
        }
    }
}
