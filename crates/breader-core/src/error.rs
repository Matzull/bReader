use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    InvalidDimensions { width: usize, height: usize },
    InvalidStride { stride: usize, min: usize },
    BufferTooSmall { len: usize, required: usize },
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
