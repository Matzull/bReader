use std::fmt;

/// Errors from image validation and pipeline construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Width or height is zero.
    InvalidDimensions {
        /// Requested width in pixels.
        width: usize,
        /// Requested height in pixels.
        height: usize,
    },
    /// Row stride is smaller than the row length.
    InvalidStride {
        /// Given stride in bytes.
        stride: usize,
        /// Minimum acceptable stride in bytes.
        min: usize,
    },
    /// The buffer does not hold the whole image.
    BufferTooSmall {
        /// Buffer length in bytes.
        len: usize,
        /// Bytes needed for the described image.
        required: usize,
    },
    /// A pipeline was built without any binarizer.
    NoBinarizer,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDimensions { width, height } => {
                write!(f, "invalid image dimensions {width}x{height}")
            }
            Self::InvalidStride { stride, min } => write!(f, "stride {stride} smaller than {min}"),
            Self::BufferTooSmall { len, required } => {
                write!(f, "buffer of {len} bytes, {required} required")
            }
            Self::NoBinarizer => write!(f, "pipeline has no binarizer registered"),
        }
    }
}

impl std::error::Error for Error {}
