# Changelog

All notable changes to breader are listed here, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project follows [Semantic Versioning](https://semver.org) from 1.0 on ([docs/API-STABILITY.md](docs/API-STABILITY.md)). A user-visible change adds a line under `Unreleased` in the same change that makes it.

## [Unreleased]

Work towards 1.0. Nothing is published yet ([D-066](docs/decisions.md#d-066)).

### Changed

- Public API prepared for growth without breaking callers ([D-068](docs/decisions.md#d-068)), breaking compared with the 0.1.0 snapshot:
  - `#[non_exhaustive]` on `Effort`, `Escalation`, `EanAddonPolicy`, `Error`, `FileError`, `EcLevel`, `TrackEvent` (`Symbology`, `Content`, `SymbolMeta`, `Charset`, `Reject`, `PixelFormat` and `Gs1Error` already were): add a wildcard arm to a `match`.
  - `#[non_exhaustive]` on `ScanOptions`, `Code39Options`, `ItfOptions`, `TrackerConfig` and `LocalBinarizer`: a struct literal (even with `..Default::default()`) no longer compiles outside the crate; use `Default` or `ScanOptions::with_effort`, then assign fields or chain the new `with_*` methods.
  - `#[non_exhaustive]` on `Symbol`, `QrMeta`, `LinearMeta`, `DataMatrixMeta`, `Pdf417Meta`, `MacroPdf417`, `AztecMeta`, `Track` and `FrameUpdate`.
  - `Pass` is `#[non_exhaustive]`.
  - `FrameCache`, `FrameContext::shared`, `Homography`, `Perspective`, `Plane` and `Upscale::FACTOR` are `#[doc(hidden)]` and outside SemVer.

### Added

- `ScanOptions::with_symbologies`, `with_try_invert`, `with_escalation`, `with_max_symbols`, `with_time_budget`, `with_upscale_retry`, `with_min_line_agreement`, `with_ean_addon`, `with_code39`, `with_itf`; `Code39Options::with_*`, `ItfOptions::with_*`, `TrackerConfig::with_*`.
- Constructors for the non-exhaustive output types, for custom readers and tests: `Symbol::new`, `QrMeta::new`, `DataMatrixMeta::new`, `AztecMeta::new`, `Pdf417Meta::new`, `MacroPdf417::new`, `LinearMeta::new`, `Pass::new`.
- `docs/API-STABILITY.md`: the SemVer scope, MSRV policy, output stability and the audit of the public API; a `semver` job in CI running cargo-semver-checks on `breader`, `breader-core` and `breader-content` for pull requests.

## [0.1.0] - unreleased snapshot

The state of the repository at the end of the first development phase. No release was made; the version number is the one in the manifests.

### Added

- **Symbologies:** QR Code (Model 2), Micro QR Code (M1 to M4), rMQR Code (all 32 sizes), Data Matrix ECC 200 (square, rectangular and DMRE), PDF417 (including compact and Macro PDF417), Aztec Code (compact, full-range and Runes), EAN-13, EAN-8, UPC-A, UPC-E (with EAN-2 and EAN-5 add-ons), Code 128 (including GS1-128), Code 39 (optionally Full ASCII), Code 93 (Full ASCII), Interleaved 2 of 5, Codabar and GS1 DataBar (Omnidirectional and Truncated, Limited, Expanded, and the stacked variants).
- **Content:** ECI and Shift JIS character sets, charset heuristics for undeclared byte mode, GS1 element string parsing with human-readable form, content classification (text, binary, URL, GS1), de-duplication.
- **Reading conditions:** light-on-dark and mirrored symbols, rotation and perspective, an up-scale retry for tiny symbols, a template fallback for thin Code 39, multiple symbols per frame.
- **Rust API** (`breader`): `Reader`, `ScanOptions` with `Effort` presets (`Fast`, `Balanced`, `Exhaustive`), escalation and a time budget, pixel formats luma, RGB, RGBA, BGRA, NV12, NV21 and I420, `scan_file` (PNG and JPEG, feature `image`), a video `Tracker`, custom pipelines through the `Preprocessor`, `Binarizer`, `SymbologyReader` and `PostProcessor` traits, optional multi-threading with identical output (feature `rayon`).
- **Python** (`breader-py`): `breader.read` and a `Reader` class, NumPy and Pillow input, one abi3 wheel for Python 3.9 and later.
- **Command line** (`breader-cli`): `breader scan` over files and directories, text and JSON Lines output, parallel jobs ([D-067](docs/decisions.md#d-067)).
- **C library** (`breader-ffi`): `libbreader` with a generated header, opaque handles and status codes ([D-064](docs/decisions.md#d-064)).
- **WebAssembly** (`breader-wasm`): a JavaScript `Reader` over `ImageData` and raw buffers ([D-065](docs/decisions.md#d-065)).
- **Quality gates:** regression baselines with zero false positives on every gated dataset (synthetic, Dynamsoft, zxing-cpp samples, BarBeR, video), fuzz targets for every decoder and the content stage, a release-only allocation test.
- **Release preparation:** crate metadata, a release workflow that builds crates, wheels, binaries and the C library and drafts a GitHub release; publishing stays manual ([D-066](docs/decisions.md#d-066)).

### Notable decisions

- Clean-room implementation from the specifications ([D-001](docs/decisions.md#d-001)); accuracy first, zero false positives as a hard constraint ([D-002](docs/decisions.md#d-002)); license `AGPL-3.0-only` plus a commercial license ([D-009](docs/decisions.md#d-009)).
- Code 39 Full ASCII off by default ([D-006](docs/decisions.md#d-006)); ITF accepts any even length of 6 or more by default, with the agreement rule of [D-059](docs/decisions.md#d-059).
- Optional threads that never change the output ([D-013](docs/decisions.md#d-013)).

