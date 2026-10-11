use breader_core::ImageView;

use crate::plan::Scanline;

/// Samples the image along `line` at one pixel steps into `out`.
///
/// Sample `i` lies at distance `i` from `line.from`. Axis-aligned lines read
/// pixels directly, other directions use bilinear interpolation. With
/// `inverted` the values become `255 - v`, so dark bars are always low.
pub fn sample(img: &ImageView<'_>, line: &Scanline, inverted: bool, out: &mut Vec<u8>) {
    out.clear();
    let (w, h) = (img.width(), img.height());
    let n = line.length().floor() as usize + 1;
    let flip = |v: u8| if inverted { 255 - v } else { v };

    if line.from.y == line.to.y {
        let y = (line.from.y.round() as usize).min(h - 1);
        let x0 = (line.from.x.round() as usize).min(w - 1);
        let reverse = line.to.x < line.from.x;
        let row = img.row(y);
        out.extend((0..n).map(|i| {
            let x = if reverse {
                x0.saturating_sub(i)
            } else {
                (x0 + i).min(w - 1)
            };
            flip(row[x])
        }));
    } else if line.from.x == line.to.x {
        let x = (line.from.x.round() as usize).min(w - 1);
        let y0 = (line.from.y.round() as usize).min(h - 1);
        let reverse = line.to.y < line.from.y;
        out.extend((0..n).map(|i| {
            let y = if reverse {
                y0.saturating_sub(i)
            } else {
                (y0 + i).min(h - 1)
            };
            flip(img.get(x, y))
        }));
    } else {
        let oblique = Oblique::new(line);
        out.resize(oblique.len, 0);
        oblique.sample(img, 0, inverted, out);
    }
}

/// Fixed-point stepping state of a line that is neither a row nor a column.
/// Positions are 16.16 fixed point and advance by one pixel of distance per
/// sample; sample `i` is `start + i * step`, so any sample range can be
/// computed on its own with the same result as stepping from the start.
#[derive(Debug, Clone, Copy)]
pub struct Oblique {
    x: i64,
    y: i64,
    sx: i64,
    sy: i64,
    /// Number of samples.
    pub len: usize,
}

impl Oblique {
    pub fn new(line: &Scanline) -> Self {
        let (dx, dy) = line.direction();
        let fixed = |v: f32| (f64::from(v) * FIXED_ONE as f64).round() as i64;
        Self {
            x: fixed(line.from.x),
            y: fixed(line.from.y),
            sx: fixed(dx),
            sy: fixed(dy),
            len: line.length().floor() as usize + 1,
        }
    }

    pub fn is_oblique(line: &Scanline) -> bool {
        line.from.y != line.to.y && line.from.x != line.to.x
    }

    /// First sample whose row is at or below `row`, or `len` when the line
    /// ends above it. Only meaningful for lines that go down the image.
    fn first_at_row(&self, row: usize) -> usize {
        if self.sy <= 0 {
            return 0;
        }
        let target = (row as i64) << FIXED_BITS;
        let steps = (target - self.y + self.sy - 1).div_euclid(self.sy);
        (steps.max(0) as usize).min(self.len)
    }

    /// Writes samples `first..first + out.len()` into `out`.
    pub fn sample(&self, img: &ImageView<'_>, first: usize, inverted: bool, out: &mut [u8]) {
        let (w, h) = (img.width(), img.height());
        let (max_x, max_y) = (
            ((w - 1) as i64) << FIXED_BITS,
            ((h - 1) as i64) << FIXED_BITS,
        );
        let mask = if inverted { 0xff } else { 0x00 };
        let (mut fx, mut fy) = (
            self.x + self.sx * first as i64,
            self.y + self.sy * first as i64,
        );
        let last = h - 1;
        let mut y0 = usize::MAX;
        let (mut r0, mut r1): (&[u8], &[u8]) = (&[], &[]);
        for slot in out.iter_mut() {
            let (x, y) = (fx.clamp(0, max_x), fy.clamp(0, max_y));
            let yi = (y >> FIXED_BITS) as usize;
            if yi != y0 {
                y0 = yi;
                r0 = img.row(yi);
                r1 = img.row((yi + 1).min(last));
            }
            *slot = bilinear_fixed(r0, r1, x, y) ^ mask;
            fx += self.sx;
            fy += self.sy;
        }
    }
}

/// Rows per band when sampling many oblique lines together.
const BAND_ROWS: usize = 32;

/// Samples all `lines` (which must be oblique) into `out`; line `k` lands at
/// `out[offsets[k]..offsets[k + 1]]`. The lines are walked band by band, so
/// the few image rows of a band stay in cache while every line crosses them,
/// instead of each line streaming through the whole image. The values equal
/// those of [`sample`].
/// Buffers of [`sample_oblique_batch`].
#[derive(Debug, Default)]
pub struct ObliqueWork {
    states: Vec<Oblique>,
    cursor: Vec<usize>,
}

pub fn sample_oblique_batch(
    img: &ImageView<'_>,
    lines: &[Scanline],
    inverted: bool,
    offsets: &mut Vec<usize>,
    out: &mut Vec<u8>,
    work: &mut ObliqueWork,
) {
    let ObliqueWork { states, cursor } = work;
    states.clear();
    states.extend(lines.iter().map(Oblique::new));
    offsets.clear();
    let mut total = 0;
    for s in states.iter() {
        offsets.push(total);
        total += s.len;
    }
    offsets.push(total);
    out.clear();
    out.resize(total, 0);
    cursor.clear();
    cursor.resize(states.len(), 0);
    let h = img.height();
    for top in (0..h).step_by(BAND_ROWS) {
        let bottom = top + BAND_ROWS;
        for (k, s) in states.iter().enumerate() {
            let end = if bottom >= h {
                s.len
            } else {
                s.first_at_row(bottom).max(cursor[k])
            };
            if end > cursor[k] {
                let dst = &mut out[offsets[k] + cursor[k]..offsets[k] + end];
                s.sample(img, cursor[k], inverted, dst);
                cursor[k] = end;
            }
        }
    }
    for (k, s) in states.iter().enumerate() {
        if cursor[k] < s.len {
            let dst = &mut out[offsets[k] + cursor[k]..offsets[k] + s.len];
            s.sample(img, cursor[k], inverted, dst);
        }
    }
}

/// Samples whole columns at once. Reading the image row by row touches each
/// cache line once, which is much faster than walking every column down the
/// image. Column `k` lands in `out[k * height..(k + 1) * height]`.
pub fn sample_columns(img: &ImageView<'_>, xs: &[usize], inverted: bool, out: &mut Vec<u8>) {
    let h = img.height();
    out.clear();
    out.resize(xs.len() * h, 0);
    let mask = if inverted { 0xff } else { 0x00 };
    for y in 0..h {
        let row = img.row(y);
        for (k, &x) in xs.iter().enumerate() {
            out[k * h + y] = row[x.min(row.len() - 1)] ^ mask;
        }
    }
}

const FIXED_BITS: u32 = 16;
const FIXED_ONE: i64 = 1 << FIXED_BITS;

/// Bilinear sample at a 16.16 fixed-point position between two adjacent rows.
fn bilinear_fixed(r0: &[u8], r1: &[u8], x: i64, y: i64) -> u8 {
    let x0 = (x >> FIXED_BITS) as usize;
    let (fx, fy) = (((x >> 8) & 255) as u32, ((y >> 8) & 255) as u32);
    let (Some(&[a, b]), Some(&[c, d])) = (r0.get(x0..x0 + 2), r1.get(x0..x0 + 2)) else {
        let last = r0.len() - 1;
        return ((u32::from(r0[x0.min(last)]) * (256 - fy) + u32::from(r1[x0.min(last)]) * fy + 128)
            >> 8) as u8;
    };
    let mix = |p: u8, q: u8| u32::from(p) * (256 - fx) + u32::from(q) * fx;
    ((mix(a, b) * (256 - fy) + mix(c, d) * fy + 32768) >> 16) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    use breader_core::Point;

    fn gradient_image() -> (Vec<u8>, usize, usize) {
        let (w, h) = (8, 6);
        let data = (0..h)
            .flat_map(|y| (0..w).map(move |x| (x * 10 + y) as u8))
            .collect();
        (data, w, h)
    }

    #[test]
    fn horizontal_and_reversed_rows_read_directly() {
        let (data, w, h) = gradient_image();
        let img = ImageView::from_packed(&data, w, h).unwrap();
        let mut out = Vec::new();
        let line = Scanline::new(Point::new(0.0, 2.0), Point::new(7.0, 2.0));
        sample(&img, &line, false, &mut out);
        assert_eq!(out, [2, 12, 22, 32, 42, 52, 62, 72]);
        let rev = Scanline::new(Point::new(7.0, 2.0), Point::new(0.0, 2.0));
        sample(&img, &rev, false, &mut out);
        assert_eq!(out, [72, 62, 52, 42, 32, 22, 12, 2]);
    }

    #[test]
    fn vertical_columns_and_inversion() {
        let (data, w, h) = gradient_image();
        let img = ImageView::from_packed(&data, w, h).unwrap();
        let mut out = Vec::new();
        let line = Scanline::new(Point::new(3.0, 0.0), Point::new(3.0, 5.0));
        sample(&img, &line, true, &mut out);
        assert_eq!(out, [225, 224, 223, 222, 221, 220]);
    }

    #[test]
    fn column_batch_matches_single_column_sampling() {
        let (data, w, h) = gradient_image();
        let img = ImageView::from_packed(&data, w, h).unwrap();
        for inverted in [false, true] {
            let mut batch = Vec::new();
            sample_columns(&img, &[1, 4, 7, 20], inverted, &mut batch);
            assert_eq!(batch.len(), 4 * h);
            for (k, x) in [1usize, 4, 7, 7].into_iter().enumerate() {
                let mut single = Vec::new();
                let line = Scanline::new(
                    Point::new(x as f32, 0.0),
                    Point::new(x as f32, h as f32 - 1.0),
                );
                sample(&img, &line, inverted, &mut single);
                assert_eq!(&batch[k * h..(k + 1) * h], &single[..]);
            }
        }
    }

    #[test]
    fn diagonal_uses_bilinear_interpolation() {
        let (data, w, h) = gradient_image();
        let img = ImageView::from_packed(&data, w, h).unwrap();
        let mut out = Vec::new();
        let line = Scanline::new(Point::new(0.0, 0.0), Point::new(5.0, 5.0));
        sample(&img, &line, false, &mut out);
        assert_eq!(out.len(), 8);
        let step = std::f32::consts::FRAC_1_SQRT_2;
        for (i, &v) in out.iter().enumerate() {
            let expected = (i as f32) * step * 11.0;
            assert!(
                (f32::from(v) - expected).abs() <= 1.0,
                "{i}: {v} vs {expected}"
            );
        }
    }

    #[test]
    fn out_of_range_endpoints_are_clamped() {
        let (data, w, h) = gradient_image();
        let img = ImageView::from_packed(&data, w, h).unwrap();
        let mut out = Vec::new();
        let line = Scanline::new(Point::new(-3.0, 9.0), Point::new(20.0, 9.0));
        sample(&img, &line, false, &mut out);
        assert!(!out.is_empty());
        let line = Scanline::new(Point::new(-3.0, -4.0), Point::new(20.0, 30.0));
        sample(&img, &line, false, &mut out);
        assert!(!out.is_empty());
    }

    #[test]
    fn banded_batch_matches_single_line_sampling() {
        let (w, h) = (150usize, 130usize);
        let data: Vec<u8> = (0..w * h).map(|i| (i * 37 % 251) as u8).collect();
        let img = ImageView::from_packed(&data, w, h).unwrap();
        let lines = [
            Scanline::new(Point::new(3.0, 2.0), Point::new(140.0, 120.0)),
            Scanline::new(Point::new(140.5, 4.0), Point::new(10.0, 126.0)),
            Scanline::new(Point::new(0.0, 100.0), Point::new(149.0, 7.0)),
            Scanline::new(Point::new(20.0, 0.0), Point::new(60.0, 129.0)),
        ];
        for inverted in [false, true] {
            let (mut offsets, mut batch) = (Vec::new(), Vec::new());
            sample_oblique_batch(
                &img,
                &lines,
                inverted,
                &mut offsets,
                &mut batch,
                &mut ObliqueWork::default(),
            );
            let mut single = Vec::new();
            for (k, line) in lines.iter().enumerate() {
                sample(&img, line, inverted, &mut single);
                assert_eq!(&batch[offsets[k]..offsets[k + 1]], &single[..], "line {k}");
            }
        }
    }
}
