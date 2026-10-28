use std::time::Duration;

use crate::SymbologySet;

/// Speed/accuracy preset: how many passes a scan may run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
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
#[non_exhaustive]
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
#[non_exhaustive]
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
///
/// Non-exhaustive: start from [`Default`], then assign fields or chain the
/// `with_*` methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
///
/// Non-exhaustive: start from [`Default`], then assign fields or chain the
/// `with_*` methods.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ItfOptions {
    /// Accepted payload lengths in digits. Empty (the default) means any even
    /// length >= `min_length`, where every length other than 14 (GTIN-14) needs
    /// one more agreeing scanline (decisions D-041 and D-059). An explicit list
    /// is chosen by the caller: every listed length then needs the plain ITF
    /// agreement.
    pub allowed_lengths: Vec<usize>,
    /// Minimum number of digits.
    pub min_length: usize,
}

impl Code39Options {
    /// Sets [`full_ascii`](Self::full_ascii).
    #[must_use]
    pub fn with_full_ascii(mut self, full_ascii: bool) -> Self {
        self.full_ascii = full_ascii;
        self
    }

    /// Sets [`require_checksum`](Self::require_checksum).
    #[must_use]
    pub fn with_require_checksum(mut self, require_checksum: bool) -> Self {
        self.require_checksum = require_checksum;
        self
    }

    /// Sets [`min_length`](Self::min_length).
    #[must_use]
    pub fn with_min_length(mut self, min_length: usize) -> Self {
        self.min_length = min_length;
        self
    }
}

impl Default for ItfOptions {
    fn default() -> Self {
        Self {
            allowed_lengths: Vec::new(),
            min_length: 6,
        }
    }
}

impl ItfOptions {
    /// Sets [`allowed_lengths`](Self::allowed_lengths).
    #[must_use]
    pub fn with_allowed_lengths(mut self, allowed_lengths: Vec<usize>) -> Self {
        self.allowed_lengths = allowed_lengths;
        self
    }

    /// Sets [`min_length`](Self::min_length).
    #[must_use]
    pub fn with_min_length(mut self, min_length: usize) -> Self {
        self.min_length = min_length;
        self
    }
}

/// Default largest side, in pixels, of a frame eligible for the up-scale retry.
pub const DEFAULT_UPSCALE_MAX_SIDE: usize = 600;

/// Options for one scan; see `docs/GUIDE.md` for a field-by-field guide.
///
/// Non-exhaustive, so fields can be added in a minor release: build it from
/// [`ScanOptions::default`] or [`ScanOptions::with_effort`], then assign
/// fields (`options.max_symbols = 1`) or chain the `with_*` methods. A struct
/// literal, even with `..Default::default()`, does not compile outside this crate.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
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
    /// Largest `max(width, height)` for which a frame that yields nothing is
    /// rescanned up-scaled; 0 disables. Overrides the effort preset when set.
    pub upscale_retry: Option<usize>,
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
            upscale_retry: None,
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

    /// Sets [`symbologies`](Self::symbologies).
    #[must_use]
    pub fn with_symbologies(mut self, symbologies: SymbologySet) -> Self {
        self.symbologies = symbologies;
        self
    }

    /// Sets [`try_invert`](Self::try_invert) to an explicit choice.
    #[must_use]
    pub fn with_try_invert(mut self, try_invert: bool) -> Self {
        self.try_invert = Some(try_invert);
        self
    }

    /// Sets [`escalation`](Self::escalation) to an explicit choice.
    #[must_use]
    pub fn with_escalation(mut self, escalation: Escalation) -> Self {
        self.escalation = Some(escalation);
        self
    }

    /// Sets [`max_symbols`](Self::max_symbols).
    #[must_use]
    pub fn with_max_symbols(mut self, max_symbols: usize) -> Self {
        self.max_symbols = max_symbols;
        self
    }

    /// Sets [`time_budget`](Self::time_budget).
    #[must_use]
    pub fn with_time_budget(mut self, time_budget: Duration) -> Self {
        self.time_budget = Some(time_budget);
        self
    }

    /// Sets [`upscale_retry`](Self::upscale_retry) to an explicit largest side (0 disables).
    #[must_use]
    pub fn with_upscale_retry(mut self, max_side: usize) -> Self {
        self.upscale_retry = Some(max_side);
        self
    }

    /// Sets [`min_line_agreement`](Self::min_line_agreement).
    #[must_use]
    pub fn with_min_line_agreement(mut self, lines: u16) -> Self {
        self.min_line_agreement = lines;
        self
    }

    /// Sets [`ean_addon`](Self::ean_addon).
    #[must_use]
    pub fn with_ean_addon(mut self, policy: EanAddonPolicy) -> Self {
        self.ean_addon = policy;
        self
    }

    /// Sets [`code39`](Self::code39).
    #[must_use]
    pub fn with_code39(mut self, code39: Code39Options) -> Self {
        self.code39 = code39;
        self
    }

    /// Sets [`itf`](Self::itf).
    #[must_use]
    pub fn with_itf(mut self, itf: ItfOptions) -> Self {
        self.itf = itf;
        self
    }

    /// Whether light-on-dark passes run: `try_invert`, or on unless the effort is `Fast`.
    pub fn invert_enabled(&self) -> bool {
        self.try_invert
            .unwrap_or(!matches!(self.effort, Effort::Fast))
    }

    /// Effective up-scale retry limit: the override, or 0 for `Fast` and
    /// [`DEFAULT_UPSCALE_MAX_SIDE`] otherwise.
    pub fn upscale_max_side(&self) -> usize {
        self.upscale_retry.unwrap_or(match self.effort {
            Effort::Fast => 0,
            Effort::Balanced | Effort::Exhaustive => DEFAULT_UPSCALE_MAX_SIDE,
        })
    }

    /// Effective escalation: the override, or `UntilFound` for `Fast` and `All` otherwise.
    pub fn escalation(&self) -> Escalation {
        self.escalation.unwrap_or(match self.effort {
            Effort::Fast => Escalation::UntilFound,
            Effort::Balanced | Effort::Exhaustive => Escalation::All,
        })
    }
}
