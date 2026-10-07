use crate::{BitMatrix, FrameContext, ImageBuffer, ImageView, Symbol, SymbologySet};

/// Produces a derived luma image (downscale, contrast normalization, ...).
pub trait Preprocessor: Send + Sync {
    /// Stable short name, used in diagnostics.
    fn id(&self) -> &'static str;
    /// Writes the derived image into `dst`, resizing it as needed.
    fn apply(&self, src: ImageView<'_>, dst: &mut ImageBuffer);
}

/// Converts luma to dark/light bits. `out` is already sized to the image.
pub trait Binarizer: Send + Sync {
    /// Stable short name, used in diagnostics.
    fn id(&self) -> &'static str;
    /// Sets `out` bit `true` for dark pixels of `src`.
    fn binarize(&self, src: ImageView<'_>, out: &mut BitMatrix);
}

/// Family-internal stage: finds candidate regions for a decoder.
/// Not object-safe on purpose; families compose detectors and decoders
/// statically and expose themselves through [`SymbologyReader`].
pub trait Detector {
    /// Detection result handed to the decoder.
    type Candidate;
    /// Appends the candidates found in the current pass image to `out`.
    fn detect(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Self::Candidate>);
}

/// Family-internal stage: turns a candidate into a validated symbol.
pub trait SymbolDecoder {
    /// Detection result consumed by this decoder.
    type Candidate;
    /// Decodes and validates one candidate.
    fn decode(&self, ctx: &FrameContext<'_>, candidate: &Self::Candidate)
    -> Result<Symbol, Reject>;
}

/// Reason a candidate was discarded; used for diagnostics and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reject {
    /// No usable pattern at the candidate.
    NoPattern,
    /// Modules could not be sampled reliably.
    Sampling,
    /// Format or version information invalid.
    Format,
    /// Check digit or check character mismatch.
    Checksum,
    /// Error correction failed.
    ErrorCorrection,
    /// Quiet zone too small.
    QuietZone,
    /// Decoded length not accepted by the options.
    Length,
    /// Independent reads disagree.
    Disagreement,
    /// Symbology or feature disabled in the options.
    Disabled,
}

/// Unit registered in the pipeline: one symbology family end to end.
pub trait SymbologyReader: Send + Sync {
    /// Stable short name, used in diagnostics.
    fn name(&self) -> &'static str;
    /// Symbologies this reader can produce; the pipeline skips it when none is enabled.
    fn symbologies(&self) -> SymbologySet;
    /// Readers working on grayscale return `false`; the pipeline then skips
    /// passes that differ from an earlier one only by binarizer.
    fn uses_binary(&self) -> bool {
        true
    }
    /// Readers that decode both polarities themselves return `true`; the
    /// pipeline then never calls them on inverted passes.
    fn handles_inversion(&self) -> bool {
        false
    }
    /// Called once per pass. Must only push symbols that passed validation.
    fn scan(&self, ctx: &mut FrameContext<'_>, out: &mut Vec<Symbol>);
}

/// Runs over the accumulated results after all passes (dedup, content parsing, ...).
pub trait PostProcessor: Send + Sync {
    /// Stable short name, used in diagnostics.
    fn id(&self) -> &'static str;
    /// Edits the final symbol list in place.
    fn process(&self, ctx: &FrameContext<'_>, symbols: &mut Vec<Symbol>);
}
