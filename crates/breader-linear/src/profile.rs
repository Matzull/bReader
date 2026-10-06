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
        let (dx, dy) = line.direction();
        out.extend((0..n).map(|i| {
            let t = i as f32;
            flip(bilinear(img, line.from.x + dx * t, line.from.y + dy * t))
        }));
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

fn bilinear(img: &ImageView<'_>, x: f32, y: f32) -> u8 {
    let (w, h) = (img.width(), img.height());
    let x = x.clamp(0.0, (w - 1) as f32);
    let y = y.clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x as usize, y as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let top = f32::from(img.get(x0, y0)) * (1.0 - fx) + f32::from(img.get(x1, y0)) * fx;
    let bottom = f32::from(img.get(x0, y1)) * (1.0 - fx) + f32::from(img.get(x1, y1)) * fx;
    (top * (1.0 - fy) + bottom * fy + 0.5) as u8
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
}
