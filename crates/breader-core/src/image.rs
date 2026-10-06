use crate::Error;

/// Pixel layouts accepted at the API boundary. Everything is reduced to an
/// 8-bit luma plane before entering the pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PixelFormat {
    Luma8,
    Rgb8,
    Rgba8,
    Bgra8,
    /// Y plane followed by interleaved UV; only the Y plane is read.
    Nv12,
    Nv21,
    /// Planar Y, U, V; only the Y plane is read.
    I420,
}

impl PixelFormat {
    /// Bytes per pixel of the plane that is read, or `None` for formats
    /// whose luma plane is used directly.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Luma8 | Self::Nv12 | Self::Nv21 | Self::I420 => 1,
            Self::Rgb8 => 3,
            Self::Rgba8 | Self::Bgra8 => 4,
        }
    }

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

    pub fn from_packed(data: &'a [u8], width: usize, height: usize) -> Result<Self, Error> {
        Self::new(data, width, height, width)
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub const fn stride(&self) -> usize {
        self.stride
    }

    pub fn row(&self, y: usize) -> &'a [u8] {
        let start = y * self.stride;
        &self.data[start..start + self.width]
    }

    pub fn get(&self, x: usize, y: usize) -> u8 {
        self.data[y * self.stride + x]
    }

    pub fn rows(&self) -> impl Iterator<Item = &'a [u8]> + '_ {
        (0..self.height).map(move |y| self.row(y))
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

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.data
    }

    pub fn row_mut(&mut self, y: usize) -> &mut [u8] {
        let start = y * self.width;
        &mut self.data[start..start + self.width]
    }

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
}
