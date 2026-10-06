//! Pixel format conversion, preprocessing and binarization.

mod downscale;
mod local;
mod luma;
mod otsu;
mod pack;

pub use downscale::Downscale;
pub use local::LocalBinarizer;
pub use luma::{luma_view, to_luma};
pub use otsu::OtsuBinarizer;
