# API stability

Documentation hub: [README.md](README.md). Decision: [D-068](decisions.md#d-068). Release process: [CI](CI.md).

This page says what breader promises from version 1.0 on, what may change in a minor release, and which items are not part of the promise. It is written before 1.0 so that the public API can grow without breaking callers: the audit below is the reason for the `#[non_exhaustive]` attributes and the `with_*` methods that exist today.

## What 1.0 guarantees

| Surface | Guarantee |
|---|---|
| Rust API of the facade crate `breader` (its own items, the re-exports from `breader-core` and `breader-content`, `breader::modules`, `gs1`, `charset`, the tracker) | [Semantic Versioning](https://semver.org). A breaking change (removed or renamed item, changed signature, new required trait method, new variant of an exhaustive enum, new field of an exhaustive struct) needs a major version. [cargo-semver-checks](CI.md#semver-check) enforces it on pull requests (on `breader`, `breader-core` and `breader-content`). |
| `breader-core` | Covered only through the facade: the items the facade re-exports are the same items, so their rules are the facade's. Depend on `breader`, not on `breader-core`. Items `breader-core` exports that the facade does not re-export are plumbing between the workspace crates and are `#[doc(hidden)]`, outside SemVer. |
| `breader-imgproc`, `breader-content`, `breader-ecc`, `breader-linear`, `breader-qr`, `breader-datamatrix`, `breader-pdf417`, `breader-aztec` | Implementation crates, versioned in lock-step with `breader` and not covered by SemVer. What reaches users is what `breader` re-exports (`breader::modules::*`, `breader::gs1`, `breader::charset`, `breader::Gs1Error`). Their other `pub` items exist for the other workspace crates, the benchmarks and the fuzz targets and may change in any release. Where a `pub` item is only there for that purpose it is `#[doc(hidden)]`. |
| C ABI (`breader-ffi`) | The rules of [D-064](decisions.md#d-064) and the [crate README](../crates/breader-ffi/README.md#versioning): within a major version functions and constants keep their name, meaning and signature and are never renumbered; additions are new symbols or new constants; options are an opaque handle with setters; enumerations are integers that the library validates; a value newer than the caller's header (for example a symbology) reads as `UNKNOWN`. The header is regenerated in CI and a difference fails the build. |
| Python package (`breader-py`) | The package version is the crate version. Function and class names, keyword names and the string values of `effort`, `escalation`, `ean_addon` and `format` keep their meaning in a major version; new keyword arguments get defaults, and new attributes may appear on results (`Symbol`, `meta`). Code must not depend on the exact set of attributes or on `repr`. |
| WebAssembly package (`breader-wasm`) | Same rules as Python: the same option names and string values; a new option has a default; unknown option keys already throw, so a script written for a newer package fails loudly on an older one instead of silently ignoring a setting. |
| Command line (`breader-cli`) | Option names, the exit status (0 found, 1 none, 2 error or unreadable input) and the documented JSON Lines fields keep their meaning. New options and new JSON fields may be added, so parse JSON by key. The human-readable format (`--format text`) may change in a minor release. |

### MSRV

`rust-version` is 1.94 (the workspace manifest). Raising it is a minor release, never a patch release, and it is written in the [CHANGELOG](../CHANGELOG.md). The project tests with the current stable toolchain; the oldest toolchain is not yet a CI job, so the number is the lowest version known to work when it was set, not a tested floor. Adding an MSRV job is a recommendation of D-068.

## What may change in a minor release

The product goal is to read more and never read wrong, so the output of a scan may change when a decoder improves.

| May change in a minor release | Never changes in a minor release |
|---|---|
| A frame that read nothing, or fewer symbols, reads more (new symbologies, harder images, higher `Fast` or `Balanced` recall). | The data of a symbol that is read: `data`, `text`, `segments`, the decoded `content` for the same payload. A change that alters the bytes of an existing read is a bug fix in a patch release at the earliest, and a major release if it is a deliberate reinterpretation. |
| `quad` (the corners), `orientation_deg` and `quality`: they get more precise; `quality` is an estimate and is never used to accept a result. | The no-false-positives policy: a symbol is returned only when it passed every check of its symbology ([ARCHITECTURE 2.5](ARCHITECTURE.md#25-validation-and-false-positive-control)). The gates keep 0 false positives; a release that would add one is not released. |
| `LinearMeta::lines_agreeing`, `errors_corrected`, `erasures` and the other counters of the metas: they depend on how many scanlines or passes see the symbol. | The meaning of a field: `lines_agreeing` is still the number of independent scanlines that gave the result, only its value for a given image can change. |
| The order of the symbols in the returned vector within a frame, the time a scan takes, the memory it uses (the steady-state no-allocation rule of NFR-4 stays). | Determinism: the same input and options give the same output on one build, with and without the `rayon` feature (NFR-3). |
| Defaults: only through an owner decision recorded in [decisions](decisions.md), listed in the CHANGELOG, and not in a patch release. A changed default is announced as "Changed". | An option is not removed or given another meaning within a major version; an option that stops having an effect is documented as such and kept. |
| New variants of the `#[non_exhaustive]` enums, new fields of the `#[non_exhaustive]` structs, new methods, new items. | |

A reader that must not change at all across updates (for example for a regression test of an application) should pin the exact version; the output rules above are about what a scan may do, not a promise of identical results to the last bit.

## How growth is made non-breaking

Rules that produced the audit below, applied to every public item of the facade and of `breader-core` ([D-068](decisions.md#d-068)).

1. **Enums that can gain variants are `#[non_exhaustive]`.** A caller must write a wildcard arm. This applies to `Symbology`, `Content`, `SymbolMeta`, `Charset`, `EcLevel`, `PixelFormat`, `Effort`, `Escalation`, `EanAddonPolicy`, `Reject`, `Error`, `FileError`, `Gs1Error` and `TrackEvent`. Inside the workspace a match on an effort preset treats an unknown future preset like `Exhaustive`.
2. **Input structs (options) are `#[non_exhaustive]` and built from `Default` or a constructor.** `ScanOptions`, `Code39Options`, `ItfOptions`, `TrackerConfig` and `LocalBinarizer` keep their fields `pub`, so reading and assigning a field works from any crate (`options.max_symbols = 1`), and each has chained `with_*` methods. What no longer compiles outside the defining crate is the struct literal, including `ScanOptions { max_symbols: 1, ..Default::default() }`, because a literal would break when a field is added. Use `ScanOptions::default().with_max_symbols(1)`, or `ScanOptions::with_effort(Effort::Fast)` and then assign. `with_effort` stays the constructor that takes the effort, so there is no chained method of that name.
3. **Output structs are `#[non_exhaustive]`.** `Symbol`, `QrMeta`, `LinearMeta`, `DataMatrixMeta`, `Pdf417Meta`, `MacroPdf417`, `AztecMeta`, `Track` and `FrameUpdate` are read by callers (fields are `pub`, and a pattern with `..` matches them). Code outside the defining crate cannot build them with a literal; custom readers and tests use `Symbol::new` and the metas' `new` (or `Default` for `LinearMeta` and `MacroPdf417`) and assign the rest.
4. **Small value types whose fields are their definition stay exhaustive:** `Point` (`x`, `y`), `Quad` (`corners`), `DataSegment` (`start`, `end`, `charset`), `Gs1Element` (`ai`, `value`). Adding a field to one of these would be a different type.
5. **Extension traits** (`Preprocessor`, `Binarizer`, `SymbologyReader`, `PostProcessor`, `Detector`, `SymbolDecoder`) are not sealed, so users implement them. A method added after 1.0 has a default body. `FrameContext`, `Scratch`, `Pass` (build with `Pass::new`), `BitMatrix`, `ImageBuffer` and `ImageView` have private fields or are `#[non_exhaustive]`.
6. **Plumbing is hidden, not removed.** `FrameCache`, `FrameContext::shared`, `Homography`, `Perspective`, `Plane`, `parallel` and `Upscale::FACTOR` are `pub` because other workspace crates use them and are `#[doc(hidden)]`: not documented, not covered by SemVer, and ignored by cargo-semver-checks. The `fuzzing` modules of the reader crates were already hidden.

## Audit

Classification: **stable** = part of the 1.0 promise and not expected to change shape; **grows** = part of the promise, expected to gain variants, fields or methods without a major release (marked `#[non_exhaustive]` or open by construction); **internal** = not part of the promise.

### Facade `breader`

| Item | Class | Notes |
|---|---|---|
| `Reader` (`new`, `with_options`, `from_pipeline`, `options`, `options_mut`, `scan`, `scan_pixels`, `scan_file`) | grows | Methods may be added. A scan takes `&mut self`. |
| `default_pipeline()` | stable | What it contains is not promised beyond "the standard modules for the enabled features". |
| `FileError` (feature `image`) | grows | `#[non_exhaustive]`. |
| `Tracker`, `Tracker::new/config/tracks/process` | grows | |
| `TrackerConfig` | grows | `#[non_exhaustive]`, `with_*`. |
| `Track`, `FrameUpdate` | grows | `#[non_exhaustive]`, output only. |
| `TrackEvent` | grows | `#[non_exhaustive]`. |
| `modules::{QrReader, DataMatrixReader, LinearReader, Pdf417Reader, AztecReader}` | stable | Unit structs; behind the reader features. |
| `modules::{Dedup, TextDecoder, ContentClassifier, OtsuBinarizer, Upscale}` | stable | Unit structs. |
| `modules::Downscale` (`new`, `factor`) | stable | |
| `modules::LocalBinarizer` | grows | `#[non_exhaustive]`; `Default`, assign fields. |
| `gs1::parse`, `gs1::hri`, `Gs1Error` | stable / grows | `Gs1Error` is `#[non_exhaustive]`. |
| `charset::decode` | stable | |
| Cargo features `qr`, `linear`, `datamatrix`, `pdf417`, `aztec`, `image`, `rayon` | grows | A new reader feature may be added (default on); removing one needs a major version. |

### Re-exports from `breader-core`

| Item | Class | Notes |
|---|---|---|
| `Symbology`, `SymbologySet` | grows | `Symbology` is `#[non_exhaustive]` with `ALL`, `name`, `is_linear`; `SymbologySet` is an opaque set (current limit 64 members). |
| `Symbol` | grows | `#[non_exhaustive]`, `Symbol::new`. |
| `SymbolMeta`, `Content`, `Charset`, `EcLevel` | grows | `#[non_exhaustive]` enums. |
| `QrMeta`, `LinearMeta`, `DataMatrixMeta`, `Pdf417Meta`, `MacroPdf417`, `AztecMeta` | grows | `#[non_exhaustive]`, `new` constructors. |
| `DataSegment`, `Gs1Element` | stable | Exhaustive, two to three fields. |
| `ScanOptions`, `Code39Options`, `ItfOptions` | grows | `#[non_exhaustive]`, `Default`, `with_*`. `ScanOptions` also has `with_effort`, `invert_enabled`, `upscale_max_side`, `escalation()`. `DEFAULT_UPSCALE_MAX_SIDE` is not re-exported by the facade. |
| `Effort`, `Escalation`, `EanAddonPolicy` | grows | `#[non_exhaustive]`. |
| `PixelFormat` | grows | `#[non_exhaustive]`. |
| `Error` | grows | `#[non_exhaustive]`. |
| `ImageView`, `ImageBuffer`, `BitMatrix`, `Point`, `Quad` | stable | `Point` and `Quad` are exhaustive. |
| `Pipeline`, `PipelineBuilder` | grows | Builder methods may be added. |
| `Preprocessor`, `Binarizer`, `SymbologyReader`, `PostProcessor`, `Detector`, `SymbolDecoder`, `Reject` | grows | Open traits (new methods have defaults); `Reject` is `#[non_exhaustive]`. |
| `FrameContext`, `Scratch`, `Pass` | grows | `FrameContext::shared` is hidden plumbing; `Pass` is `#[non_exhaustive]` with `Pass::new`. |

### `breader-core` only (not re-exported by the facade)

| Item | Class | Notes |
|---|---|---|
| `FrameCache`, `Homography`, `Perspective`, `Plane`, `parallel` | internal | `#[doc(hidden)]`, used between the workspace crates. |
| `DEFAULT_UPSCALE_MAX_SIDE` | stable | Reachable through `ScanOptions::upscale_max_side`; the constant itself is only in `breader-core`. |

### Other crates

| Crate | Public surface | Class |
|---|---|---|
| `breader-linear`, `-qr`, `-datamatrix`, `-pdf417`, `-aztec` | One reader type each (re-exported in `breader::modules`) and a `#[doc(hidden)] fuzzing` module (the linear crate also a hidden `stacked` module). | The reader type is stable through the facade; everything else is internal. |
| `breader-content` | `ContentClassifier`, `Dedup`, `TextDecoder`, `charset`, `gs1`, `Gs1Error` (all re-exported). | As the facade. |
| `breader-imgproc` | The five modules (re-exported), `luma_view`, `to_luma`. | The modules as the facade; the two functions are internal. |
| `breader-ecc` | Reed-Solomon fields (`Gf256`, `Gf2m`, `Gf929`, `RsError`, constants). | Internal. |
| `breader-ffi`, `breader-py`, `breader-wasm`, `breader-cli` | A C library, a Python module, a JavaScript package, a command line tool. | Their own rules, above. The Rust items of these crates are not an API. |

## Checking it

- `cargo-semver-checks` runs on pull requests against the base branch for `breader`, `breader-core` and `breader-content` ([CI](CI.md#semver-check)). A failure is either a mistake or an intended major change; the second needs the owner.
- Adding a variant or a field to an item marked `#[non_exhaustive]` is a minor change and needs a CHANGELOG line ([contributing](contributing.md#definition-of-done)); adding a new public item without the attribute, when it can grow, is a review comment.
- A new public enum or struct states in its doc comment whether it is `#[non_exhaustive]`, and why not when it is not.
