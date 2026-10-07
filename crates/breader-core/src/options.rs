use std::time::Duration;

use crate::SymbologySet;

/// Speed/accuracy preset: how many passes a scan may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Effort {
    /// Single pass, first binarizer, no inversion. Intended for video.
    Fast,
    /// Two binarizers and the first preprocessor, both polarities.
    #[default]
    Balanced,
    /// Every registered binarizer, both polarities, all preprocessors.
    Exhaustive,
}

/// When the pipeline stops running further passes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Escalation {
    /// Run every pass of the effort preset.
    All,
    /// Stop after the first pass that ends with at least one symbol.
    UntilFound,
    /// Stop after a pass that adds no symbol, once at least one was found.
    UntilStable,
}

/// How EAN/UPC supplements (EAN-2, EAN-5) are handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EanAddonPolicy {
    /// Do not look for add-ons.
    #[default]
    Ignore,
    /// Report the add-on when found, accept the symbol without it.
    Optional,
    /// Reject symbols without an add-on.
    Required,
}

/// Code 39 decoding options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Code39Options {
    /// Decode Full ASCII (two-character escape sequences).
    pub full_ascii: bool,
    /// Require a valid modulo-43 check character (kept in the payload).
    pub require_checksum: bool,
    /// Minimum number of data characters.
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

/// Interleaved 2 of 5 decoding options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItfOptions {
    /// Accepted payload lengths in digits. Empty means any even length >= `min_length`.
    pub allowed_lengths: Vec<usize>,
    /// Minimum number of digits.
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

/// Options for one scan; see `docs/GUIDE.md` for a field-by-field guide.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanOptions {
    /// Symbologies to look for. Disabled ones are never reported.
    pub symbologies: SymbologySet,
    /// Speed/accuracy preset.
    pub effort: Effort,
    /// Overrides the effort preset when set.
    pub try_invert: Option<bool>,
    /// Overrides the effort preset when set.
    pub escalation: Option<Escalation>,
    /// Stop after this many symbols; 0 means unlimited.
    pub max_symbols: usize,
    /// Checked between passes; a pass in progress is never interrupted.
    pub time_budget: Option<Duration>,
    /// Minimum independent scanlines that must agree on a 1D result.
    pub min_line_agreement: u16,
    /// EAN/UPC add-on handling.
    pub ean_addon: EanAddonPolicy,
    /// Code 39 options.
    pub code39: Code39Options,
    /// ITF options.
    pub itf: ItfOptions,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            symbologies: SymbologySet::all(),
            effort: Effort::default(),
            try_invert: None,
            escalation: None,
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
    /// Default options with the given effort.
    pub fn with_effort(effort: Effort) -> Self {
        Self {
            effort,
            ..Self::default()
        }
    }

    /// Whether light-on-dark passes run: `try_invert`, or on unless the effort is `Fast`.
    pub fn invert_enabled(&self) -> bool {
        self.try_invert
            .unwrap_or(!matches!(self.effort, Effort::Fast))
    }

    /// Effective escalation: the override, or `UntilFound` for `Fast` and `All` otherwise.
    pub fn escalation(&self) -> Escalation {
        self.escalation.unwrap_or(match self.effort {
            Effort::Fast => Escalation::UntilFound,
            Effort::Balanced | Effort::Exhaustive => Escalation::All,
        })
    }
}
