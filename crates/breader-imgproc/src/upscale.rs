use breader_core::{ImageBuffer, ImageView, Preprocessor};

/// Cubic (Catmull-Rom) enlargement by 3. Output sample `3i + 1` sits on source
/// sample `i`; samples `3i` and `3i + 2` lie a third of a pixel before and
/// after it, so the kernel reduces to two integer tap sets with divisor 27.
/// Results are clamped to `0..=255`; edges repeat the border sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Upscale;

/// Taps over source samples `i - 1 ..= i + 2` for the output sample a third
/// of a pixel after `i`.
const AFTER: [i32; 4] = [-2, 21, 9, -1];
/// Taps over source samples `i - 2 ..= i + 1` for the sample a third of a
/// pixel before `i`.
const BEFORE: [i32; 4] = [-1, 9, 21, -2];

/// Columns computed per stack chunk in the vertical pass.
const CHUNK: usize = 256;

impl Upscale {
    /// Enlargement factor. Not covered by SemVer.
    #[doc(hidden)]
    pub const FACTOR: usize = 3;
}

#[inline(always)]
fn round27(sum: i32) -> u8 {
    ((sum + 13).div_euclid(27)).clamp(0, 255) as u8
}

#[inline(always)]
fn clamped(i: isize, len: usize) -> usize {
    i.clamp(0, len as isize - 1) as usize
}

impl Preprocessor for Upscale {
    fn id(&self) -> &'static str {
        "upscale3"
    }

    /// The horizontal pass fills rows `3y + 1` of `dst`; the vertical pass
    /// then derives rows `3y` and `3y + 2` from those rows only, so no
    /// temporary image is needed.
    fn apply(&self, src: ImageView<'_>, dst: &mut ImageBuffer) {
        let (w, h) = (src.width(), src.height());
        let width = w * Self::FACTOR;
        dst.resize(width, h * Self::FACTOR);
        for y in 0..h {
            let row = src.row(y);
            let out = dst.row_mut(3 * y + 1);
            for x in 0..w {
                let at = |d: isize| i32::from(row[clamped(x as isize + d, w)]);
                let (m2, m1, c, p1, p2) = (at(-2), at(-1), at(0), at(1), at(2));
                out[3 * x] =
                    round27(BEFORE[0] * m2 + BEFORE[1] * m1 + BEFORE[2] * c + BEFORE[3] * p1);
                out[3 * x + 1] = row[x];
                out[3 * x + 2] =
                    round27(AFTER[0] * m1 + AFTER[1] * c + AFTER[2] * p1 + AFTER[3] * p2);
            }
        }
        let data = dst.as_mut_slice();
        let mut chunk = [0u8; CHUNK];
        for y in 0..h {
            let tap = |d: isize| (clamped(y as isize + d, h) * 3 + 1) * width;
            for (taps, weights, out_row) in [
                ([tap(-2), tap(-1), tap(0), tap(1)], BEFORE, 3 * y),
                ([tap(-1), tap(0), tap(1), tap(2)], AFTER, 3 * y + 2),
            ] {
                for x0 in (0..width).step_by(CHUNK) {
                    let n = CHUNK.min(width - x0);
                    for (i, o) in chunk[..n].iter_mut().enumerate() {
                        let x = x0 + i;
                        let sum: i32 = taps
                            .iter()
                            .zip(weights)
                            .map(|(&t, k)| k * i32::from(data[t + x]))
                            .sum();
                        *o = round27(sum);
                    }
                    let start = out_row * width + x0;
                    data[start..start + n].copy_from_slice(&chunk[..n]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(pixels: &[u8], w: usize, h: usize) -> ImageBuffer {
        let mut dst = ImageBuffer::default();
        Upscale.apply(ImageView::from_packed(pixels, w, h).unwrap(), &mut dst);
        dst
    }

    #[test]
    fn size_and_source_samples() {
        let (w, h) = (7usize, 5usize);
        let pixels: Vec<u8> = (0..w * h).map(|i| (i * 37 % 251) as u8).collect();
        let out = run(&pixels, w, h);
        assert_eq!((out.width(), out.height()), (21, 15));
        let v = out.view();
        for y in 0..h {
            for x in 0..w {
                assert_eq!(v.get(3 * x + 1, 3 * y + 1), pixels[y * w + x]);
            }
        }
    }

    #[test]
    fn flat_and_linear_images_are_exact() {
        let out = run(&[90u8; 12], 4, 3);
        assert!(out.view().rows().flatten().all(|&v| v == 90));
        // A horizontal ramp is reproduced by the cubic kernel away from the border.
        let (w, h) = (12usize, 4usize);
        let pixels: Vec<u8> = (0..w * h).map(|i| (i % w * 9) as u8).collect();
        let v = run(&pixels, w, h);
        let v = v.view();
        for x in 6..3 * w - 6 {
            let want = (x as f32 + 0.5) / 3.0 - 0.5;
            assert!((f32::from(v.get(x, 5)) - want * 9.0).abs() <= 1.0, "x {x}");
        }
    }

    #[test]
    fn step_is_clamped_and_single_pixel_works() {
        let out = run(&[0, 0, 255, 255, 0, 0, 255, 255], 4, 2);
        assert_eq!((out.width(), out.height()), (12, 6));
        assert_eq!(run(&[200], 1, 1).view().get(2, 2), 200);
    }
}
