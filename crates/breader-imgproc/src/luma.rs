use crate::simd::luma4_row;
use breader_core::{Error, ImageBuffer, ImageView, PixelFormat};

fn validate(
    data: &[u8],
    format: PixelFormat,
    width: usize,
    height: usize,
    stride: usize,
) -> Result<(), Error> {
    if width == 0 || height == 0 {
        return Err(Error::InvalidDimensions { width, height });
    }
    let row_bytes = width.saturating_mul(format.bytes_per_pixel());
    if stride < row_bytes {
        return Err(Error::InvalidStride {
            stride,
            min: row_bytes,
        });
    }
    let required = stride.saturating_mul(height - 1).saturating_add(row_bytes);
    if data.len() < required {
        return Err(Error::BufferTooSmall {
            len: data.len(),
            required,
        });
    }
    Ok(())
}

#[inline]
// Weights sum to 256 and the maximum total is 65408, so u16 is exact.
fn luma_bt601(r: u8, g: u8, b: u8) -> u8 {
    ((77 * u16::from(r) + 150 * u16::from(g) + 29 * u16::from(b) + 128) >> 8) as u8
}

/// Converts the first plane of `data` into a tightly packed luma image.
/// `stride` is the byte distance between source rows.
pub fn to_luma(
    data: &[u8],
    format: PixelFormat,
    width: usize,
    height: usize,
    stride: usize,
    dst: &mut ImageBuffer,
) -> Result<(), Error> {
    validate(data, format, width, height, stride)?;
    dst.resize(width, height);
    for y in 0..height {
        let start = y * stride;
        let out = dst.row_mut(y);
        match format {
            PixelFormat::Rgb8 => {
                let src = &data[start..start + width * 3];
                for (o, px) in out.iter_mut().zip(src.as_chunks::<3>().0) {
                    *o = luma_bt601(px[0], px[1], px[2]);
                }
            }
            PixelFormat::Rgba8 => {
                luma4_row(&data[start..start + width * 4], out, 77, 29);
            }
            PixelFormat::Bgra8 => {
                luma4_row(&data[start..start + width * 4], out, 29, 77);
            }
            _ => out.copy_from_slice(&data[start..start + width]),
        }
    }
    Ok(())
}

/// Zero-copy view for formats that carry a luma plane, `None` otherwise.
pub fn luma_view(
    data: &[u8],
    format: PixelFormat,
    width: usize,
    height: usize,
    stride: usize,
) -> Option<Result<ImageView<'_>, Error>> {
    if !format.has_luma_plane() {
        return None;
    }
    Some(
        validate(data, format, width, height, stride)
            .and_then(|()| ImageView::new(data, width, height, stride)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(
        data: &[u8],
        format: PixelFormat,
        w: usize,
        h: usize,
        stride: usize,
    ) -> Result<Vec<u8>, Error> {
        let mut dst = ImageBuffer::default();
        to_luma(data, format, w, h, stride, &mut dst)?;
        Ok(dst.view().rows().flatten().copied().collect())
    }

    #[test]
    fn luma_plane_formats_copy_y() {
        let data = [1, 2, 3, 4, 5, 6, 7, 8];
        for format in [
            PixelFormat::Luma8,
            PixelFormat::Nv12,
            PixelFormat::Nv21,
            PixelFormat::I420,
        ] {
            assert_eq!(convert(&data, format, 2, 2, 2).unwrap(), [1, 2, 3, 4]);
        }
    }

    #[test]
    fn strided_input() {
        let data = [1, 2, 99, 99, 3, 4];
        assert_eq!(
            convert(&data, PixelFormat::Luma8, 2, 2, 4).unwrap(),
            [1, 2, 3, 4]
        );
        let rgb = [
            255, 255, 255, 0, 0, 0, 7, 7, 7, 0, 0, 0, 0, 0, 0, 255, 255, 255,
        ];
        assert_eq!(convert(&rgb, PixelFormat::Rgb8, 2, 2, 9).unwrap().len(), 4);
    }

    #[test]
    fn rgb_family_uses_bt601() {
        assert_eq!(luma_bt601(255, 255, 255), 255);
        assert_eq!(luma_bt601(0, 0, 0), 0);
        let rgb = [255, 0, 0, 0, 255, 0];
        assert_eq!(
            convert(&rgb, PixelFormat::Rgb8, 2, 1, 6).unwrap(),
            [77, 149]
        );
        let rgba = [255, 0, 0, 9, 0, 255, 0, 9];
        assert_eq!(
            convert(&rgba, PixelFormat::Rgba8, 2, 1, 8).unwrap(),
            [77, 149]
        );
        let bgra = [0, 0, 255, 9, 0, 255, 0, 9];
        assert_eq!(
            convert(&bgra, PixelFormat::Bgra8, 2, 1, 8).unwrap(),
            [77, 149]
        );
    }

    #[test]
    fn size_errors() {
        assert_eq!(
            convert(&[0; 4], PixelFormat::Luma8, 0, 2, 2).unwrap_err(),
            Error::InvalidDimensions {
                width: 0,
                height: 2
            }
        );
        assert_eq!(
            convert(&[0; 12], PixelFormat::Rgb8, 2, 2, 5).unwrap_err(),
            Error::InvalidStride { stride: 5, min: 6 }
        );
        assert_eq!(
            convert(&[0; 11], PixelFormat::Rgb8, 2, 2, 6).unwrap_err(),
            Error::BufferTooSmall {
                len: 11,
                required: 12
            }
        );
        assert!(convert(&[0; 4], PixelFormat::Luma8, 2, 2, usize::MAX).is_err());
    }

    #[test]
    fn view_only_for_luma_formats() {
        let data = [1, 2, 3, 4];
        assert!(luma_view(&data, PixelFormat::Rgb8, 1, 1, 3).is_none());
        let v = luma_view(&data, PixelFormat::Nv12, 2, 2, 2)
            .unwrap()
            .unwrap();
        assert_eq!(v.row(1), &[3, 4]);
        assert!(
            luma_view(&data, PixelFormat::Luma8, 3, 2, 3)
                .unwrap()
                .is_err()
        );
    }
}
