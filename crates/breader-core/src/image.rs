use crate::Error;

/// Pixel layouts accepted at the API boundary. Everything is reduced to an
/// 8-bit luma plane before entering the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelFormat {
    /// One byte per pixel.
    Luma8,
    /// Three bytes per pixel: R, G, B.
    Rgb8,
    /// Four bytes per pixel: R, G, B, A.
    Rgba8,
    /// Four bytes per pixel: B, G, R, A.
    Bgra8,
    /// Y plane followed by interleaved UV; only the Y plane is read.
    Nv12,
    /// Like [`Nv12`](Self::Nv12) with V before U; the luma plane is identical.
    Nv21,
    /// Planar Y, U, V; only the Y plane is read.
    I420,
}

impl PixelFormat {
    /// Bytes per pixel of the plane that is read (the luma plane for YUV formats).
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Luma8 | Self::Nv12 | Self::Nv21 | Self::I420 => 1,
            Self::Rgb8 => 3,
            Self::Rgba8 | Self::Bgra8 => 4,
        }
    }

    /// Whether the luma plane can be scanned in place without conversion.
    pub const fn has_luma_plane(self) -> bool {
        matches!(self, Self::Luma8 | Self::Nv12 | Self::Nv21 | Self::I420)
    }
}

/// Borrowed 8-bit luma image with an arbitrary row stride.
#[derive(Debug, Clone, Copy)]
pub struct ImageView<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
    stride: usize,
}

impl<'a> ImageView<'a> {
    /// Wraps `data` as a `width` x `height` image with `stride` bytes per row.
    /// Fails when a dimension is zero, `stride < width`, or the buffer is shorter
    /// than `stride * (height - 1) + width`.
    pub fn new(data: &'a [u8], width: usize, height: usize, stride: usize) -> Result<Self, Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidDimensions { width, height });
        }
        if stride < width {
            return Err(Error::InvalidStride { stride, min: width });
        }
        let required = stride * (height - 1) + width;
        if data.len() < required {
            return Err(Error::BufferTooSmall {
                len: data.len(),
                required,
            });
        }
        Ok(Self {
            data,
            width,
            height,
            stride,
        })
    }

    /// Like [`new`](Self::new) with `stride == width`.
    pub fn from_packed(data: &'a [u8], width: usize, height: usize) -> Result<Self, Error> {
        Self::new(data, width, height, width)
    }

    /// Width in pixels.
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> usize {
        self.height
    }

    /// Row pitch in bytes.
    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// Pixels of row `y` (exactly `width` bytes). Panics when `y >= height`.
    pub fn row(&self, y: usize) -> &'a [u8] {
        let start = y * self.stride;
        &self.data[start..start + self.width]
    }

    /// Pixel at column `x`, row `y`.
    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.stride + x]
    }

    /// Iterator over all rows, top to bottom.
    pub fn rows(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
        (0..self.height).map(move |y| self.row(y))
    }

    /// Zero-copy sub-rectangle, clipped to the image. `None` when the
    /// clipped rectangle is empty.
    pub fn crop(&self, x: usize, y: usize, width: usize, height: usize) -> Option<ImageView<'a>> {
        let x1 = x.saturating_add(width).min(self.width);
        let y1 = y.saturating_add(height).min(self.height);
        if x >= x1 || y >= y1 {
            return None;
        }
        let (w, h) = (x1 - x, y1 - y);
        let start = y * self.stride + x;
        let end = (y1 - 1) * self.stride + x1;
        Some(ImageView {
            data: &self.data[start..end],
            width: w,
            height: h,
            stride: self.stride,
        })
    }
}

/// Owned, tightly packed 8-bit luma image; reused across frames to avoid
/// per-frame allocation.
#[derive(Debug, Clone, Default)]
pub struct ImageBuffer {
    data: Vec<u8>,
    width: usize,
    height: usize,
}

impl ImageBuffer {
    /// Creates a zero-filled image.
    pub fn new(width: usize, height: usize) -> Self {
        Self {
            data: vec![0; width * height],
            width,
            height,
        }
    }

    /// Resizes keeping the allocation when possible. Contents are unspecified afterwards.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.data.resize(width * height, 0);
    }

    /// Width in pixels.
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    pub const fn height(&self) -> usize {
        self.height
    }

    /// All pixels, row after row, without padding.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    /// Mutable pixels of row `y`.
    pub fn row_mut(&mut self, y: usize) -> &mut [u8] {
        let start = y * self.width;
        &mut self.data[start..start + self.width]
    }

    /// Borrowed view with stride equal to the width.
    pub fn view(&self) -> ImageView<'_> {
        ImageView {
            data: &self.data,
            width: self.width,
            height: self.height,
            stride: self.width,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn view_validation() {
        let buf = [0u8; 10];
        assert!(ImageView::new(&buf, 3, 2, 4).is_ok());
        assert_eq!(
            ImageView::new(&buf, 4, 3, 4).unwrap_err(),
            Error::BufferTooSmall {
                len: 10,
                required: 12
            }
        );
        assert!(ImageView::new(&buf, 4, 1, 3).is_err());
        assert!(ImageView::new(&buf, 0, 1, 1).is_err());
    }

    #[test]
    fn strided_rows() {
        let buf = [1, 2, 9, 3, 4, 9];
        let v = ImageView::new(&buf, 2, 2, 3).unwrap();
        assert_eq!(v.row(1), &[3, 4]);
        assert_eq!(v.get(1, 0), 2);
    }

    #[test]
    fn crop_is_clipped_and_zero_copy() {
        let buf: Vec<u8> = (0..60).collect();
        let img = ImageView::new(&buf, 5, 6, 10).unwrap();
        let c = img.crop(2, 1, 10, 2).unwrap();
        assert_eq!((c.width(), c.height(), c.stride()), (3, 2, 10));
        assert_eq!(c.row(0), &[12, 13, 14]);
        assert_eq!(c.row(1), &[22, 23, 24]);
        assert!(img.crop(5, 0, 1, 1).is_none());
        assert!(img.crop(0, 6, 1, 1).is_none());
        assert!(img.crop(0, 0, 0, 1).is_none());
        let full = img.crop(0, 0, usize::MAX, usize::MAX).unwrap();
        assert_eq!((full.width(), full.height()), (5, 6));
        assert_eq!(full.get(4, 5), 54);
    }
}
