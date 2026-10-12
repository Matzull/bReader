//! Shared ground-truth and matching code of the bench tools.

pub mod barber;
#[cfg(feature = "baselines")]
pub mod baselines;
pub mod engine;
pub mod gate;
pub mod groundtruth;
pub mod matching;
pub mod zxing_samples;
