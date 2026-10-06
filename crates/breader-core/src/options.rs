use std::time::Duration;

use crate::SymbologySet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effort {
    /// Single pass, first binarizer, no inversion. Intended for video.
    Fast,
    #[default]
    Balanced,
    /// Every registered binarizer, both polarities, all preprocessors.
    Exhaustive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EanAddonPolicy {
    #[default]
    Ignore,
    /// Report the add-on when found, accept the symbol without it.
    Optional,
    /// Reject symbols without an add-on.
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code39Options {
    pub full_ascii: bool,
    pub require_checksum: bool,
    pub min_length: usize,
}

impl Default for Code39Options {
    fn default() -> Self {
        Self {
            full_ascii: false,
            require_checksum: false,
            min_length: 3,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItfOptions {
    /// Accepted payload lengths in digits. Empty means any even length >= `min_length`.
    pub allowed_lengths: Vec<usize>,
    pub min_length: usize,
}

impl Default for ItfOptions {
    fn default() -> Self {
        Self {
            allowed_lengths: vec![14],
            min_length: 6,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScanOptions {
    pub symbologies: SymbologySet,
    pub effort: Effort,
    /// Overrides the effort preset when set.
    pub try_invert: Option<bool>,
    /// Stop after this many symbols; 0 means unlimited.
    pub max_symbols: usize,
    /// Checked between passes; a pass in progress is never interrupted.
    pub time_budget: Option<Duration>,
    /// Minimum independent scanlines that must agree on a 1D result.
    pub min_line_agreement: u16,
    pub ean_addon: EanAddonPolicy,
    pub code39: Code39Options,
    pub itf: ItfOptions,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            symbologies: SymbologySet::all(),
            effort: Effort::default(),
            try_invert: None,
            max_symbols: 0,
            time_budget: None,
            min_line_agreement: 2,
            ean_addon: EanAddonPolicy::default(),
            code39: Code39Options::default(),
            itf: ItfOptions::default(),
        }
    }
}

impl ScanOptions {
    pub fn with_effort(effort: Effort) -> Self {
        Self {
            effort,
            ..Self::default()
        }
    }

    pub fn invert_enabled(&self) -> bool {
        self.try_invert
            .unwrap_or(!matches!(self.effort, Effort::Fast))
    }
}
