//! Pixel format conversion and global binarization.

mod luma;
mod otsu;

pub use luma::{luma_view, to_luma};
pub use otsu::OtsuBinarizer;
