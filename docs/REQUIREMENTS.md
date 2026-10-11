# breader — Requirements

Status: draft v0.1 (2026-10-06). Requirement IDs are stable; amend rather than renumber.

## 1. Purpose

breader is a barcode and QR code reading library written in Rust, intended as an alternative to zbar, ZXing and Dynamsoft Barcode Reader. The two primary goals, in order:

1. **Accuracy**: never return wrong data, and read as many symbols as possible under hard real-world conditions.
2. **Speed**: suitable for real-time video on desktop and ARM hardware.

The library is built as a pipeline of pluggable modules so symbologies and processing techniques can be added without changing the core.

## 2. Scope

### 2.1 In scope (v1)

| Family | Symbologies | Details |
|---|---|---|
| 2D matrix | QR Code Model 2 | Versions 1–40, EC levels L/M/Q/H, numeric/alphanumeric/byte/kanji modes, ECI |
| Retail 1D | EAN-13, EAN-8, UPC-A, UPC-E | EAN-2 / EAN-5 add-ons |
| Industrial 1D | Code 128 | Code sets A/B/C, FNC1 (GS1-128) |
| | Code 39 | Standard, optional Full ASCII, optional mod-43 check |
| | ITF (Interleaved 2 of 5) | ITF-14, configurable accepted lengths |

### 2.2 Out of scope (v1)

- Encoding / generating symbols (encoder crates are used only as test dev-dependencies).
- WebAssembly and mobile (Android/iOS) targets.
- Dynamically loaded plugins (`.so`/`.dll`).
- Micro QR, rMQR, QR Structured Append, Model 1 QR.
- MaxiCode, MicroPDF417, other 1D symbologies (planned later; the architecture must accommodate them). Data Matrix, PDF417 and Aztec are implemented.

## 3. Functional requirements

### Input

- **FR-1** Accept images as a borrowed 8-bit luma plane with explicit width, height and row stride, without copying.
- **FR-2** Accept `Rgb8`, `Rgba8`, `Bgra8` (converted to luma once per frame into a reused buffer) and `Nv12`, `Nv21`, `I420` (Y plane used directly, zero-copy).
- **FR-3** Optionally (cargo feature `image`) load PNG and JPEG files.
- **FR-4** Reject invalid dimensions, strides or undersized buffers with an error; never panic.

### Detection and decoding

- **FR-10** Detect and decode every enabled symbol in an image (multiple symbols, mixed symbologies).
- **FR-11** Support arbitrary in-plane rotation and perspective distortion.
- **FR-12** Support inverted symbols (light on dark).
- **FR-13** Tolerate uneven illumination, shadows and glare (local/adaptive binarization).
- **FR-14** Tolerate blur and low resolution (target: QR modules ≥ 2 px, 1D narrow bar ≥ 1.5 px; to be confirmed by benchmarks).
- **FR-15** QR: correct errors with Reed-Solomon; read mirrored symbols; report version, EC level, mask, number of corrected errors and ECI designators.
- **FR-16** EAN/UPC: validate check digit; add-on policy configurable (ignore / optional / required).
- **FR-17** Code 39: Full ASCII and mod-43 checksum configurable; configurable minimum length.
- **FR-18** ITF: configurable accepted lengths (default: 14 only) and minimum length.
- **FR-19** Code 128: validate the mod-103 checksum; report FNC1 as GS1 data.

### Output

- **FR-20** For each symbol return: symbology, raw payload bytes, text (when the charset is known), position as a quadrilateral in image coordinates, orientation in degrees, AIM symbology identifier (ISO/IEC 15424), symbology-specific metadata, and a quality estimate.
- **FR-21** Decode text using ECI when present, otherwise the symbology default charset, with a documented heuristic for undeclared QR byte mode (UTF-8 / ISO-8859-1 / Shift-JIS).
- **FR-22** Parse GS1 element strings (Application Identifiers) for GS1-128, GS1 QR and EAN/UPC.
- **FR-23** Classify content (URL, text, binary, GS1). Additional semantic parsers (vCard, Wi-Fi, ...) are later modules.
- **FR-24** Report the same physical symbol once per frame (deduplication across passes and scanlines).

### Configuration

- **FR-30** Enable or disable each symbology individually.
- **FR-31** Effort presets `Fast`, `Balanced`, `Exhaustive`, each overridable by fine-grained options (inversion, binarizers, preprocessors/scales, minimum agreeing scanlines).
- **FR-32** Optional maximum number of symbols, stopping early when reached.
- **FR-33** Optional time budget per frame, checked between passes.

### Extensibility

- **FR-40** New symbologies, binarizers, preprocessors and post-processors are added by implementing a trait and registering it in the pipeline builder, without modifying the core crate. External crates can provide modules.
- **FR-41** Inter-frame tracking for video (reuse of previous locations, result stabilization) is a later optional module; the core API remains stateless per frame.

### Bindings

- **FR-50** Rust crate is the primary API.
- **FR-51** Python bindings (PyO3, built with maturin) exposing scanning of raw pixel buffers and files; the GIL is released during scanning.

## 4. Non-functional requirements

- **NFR-1 False positives**: zero wrong results on the benchmark datasets and on a negative image set (images without symbols / with unsupported symbologies). When in doubt, return nothing. Every 1D result must pass checksum (where defined), quiet-zone checks and agreement of at least `min_line_agreement` independent scanlines (default 2); every QR result must pass format-info BCH and Reed-Solomon decoding.
- **NFR-2 Robustness**: no panics, infinite loops or unbounded allocations on any input; decoders are fuzzed.
- **NFR-3 Determinism**: identical input and options give identical output in single-threaded mode.
- **NFR-4 Allocation**: in steady state (same resolution, `Fast` effort) scanning a frame performs no heap allocation beyond the returned results; buffers live in a reusable `Scratch`.

  Measured and gated by `crates/breader/tests/allocation.rs` (counting global allocator, three warm-up scans of the same frame, then one counted scan; `cargo test --release -p breader --test allocation -- --nocapture` prints the table). 2026-10-11, allocations per scan, default build, 800x600 gray frames (320x240 frames, which take the up-scale retry, give the same zeros and slightly different counts):

  | Reader (alone) | blank Fast / Bal / Exh | texture Fast / Bal / Exh | one QR Fast / Bal / Exh |
  |---|---|---|---|
  | linear | 0 / 0 / 0 (before: 33 / 54 / 60) | 0 / 0 / 0 (49 / 77 / 89) | 0 / 0 / 0 |
  | Data Matrix | 0 / 0 / 0 (12 / 14 / 23) | 0 / 0 / 0 | 0 / 0 / 0 |
  | PDF417 (with linear) | 0 / 0 / 0 (33 / 54 / 60) | 0 / 14 / 38 (49 / 91 / 127) | 0 / 0 / 0 |
  | QR | 0 / 0 / 0 (13 / 27 / 27) | 1 / 6 / 4 (54 / 209 / 193) | 17 / 20 / 20 (result included) |
  | Aztec | 0 / 0 / 0 (12 / 23 / 23) | 360 / 6450 / 16055 (372 / 6473 / 16078) | 0 / 320 / 320 |
  | default pipeline | 0 / 0 / 0 (44 / 78 / 84) | 361 / 6470 / 16097 | 17 / 340 / 340 |

  Blank frames never allocate at any effort. Fixed on the way: per-scan temporaries of the pass loop (pass list, reader list), the local binarizer (tile, summed-area and row buffers), the down-scale row sums, the linear reader's `Work` (now kept per thread, like Data Matrix's) and the QR reader's finder and grid buffers. Remaining: on textured frames the QR (stable sort of triplets), PDF417 (boundary growth) and Aztec readers allocate inside candidate verification (geometry fits and least-squares solves, per candidate); the test asserts measured bounds with margin for them and exactly zero for linear and Data Matrix. A frame with a symbol allocates for the result itself (`Symbol` data, segments, content) and for per-symbol decoding state. With the `rayon` feature the parallel kernels (frames of 2^19 pixels or more, which includes the 3x up-scaled retry copy of a small frame) allocate per task; the test skips the bounds for those frames.
- **NFR-5 Performance**: latency targets (p50/p95 per resolution and effort) are set after the first benchmark against the baselines (§6) and recorded here.
- **NFR-6 Threads**: single-threaded by default; `rayon` parallelism behind a cargo feature.
- **NFR-7 Platforms**: Linux, Windows, macOS on x86_64 and aarch64. MSRV 1.94, edition 2024.
- **NFR-8 Dependencies**: minimal; a well-maintained crate is preferred over reinventing it. Optional dependencies sit behind cargo features.
- **NFR-9 `unsafe`**: denied workspace-wide; allowed only in isolated, documented SIMD kernels with a safe scalar fallback and equivalence tests.
- **NFR-10 Code style**: English identifiers and comments; comments only where they add information.

## 5. Clean-room policy

ZXing (Apache-2.0) and zbar (LGPL-2.1) may be studied for ideas and approaches; no code, tables or structure may be copied or transliterated. The normative references are the specifications:

- ISO/IEC 18004 — QR Code
- ISO/IEC 15420 — EAN/UPC
- ISO/IEC 15417 — Code 128
- ISO/IEC 16388 — Code 39
- ISO/IEC 16390 — Interleaved 2 of 5
- ISO/IEC 15424 — symbology identifiers
- GS1 General Specifications — Application Identifiers

Tables derivable from the specifications (character sets, BCH codes, generator polynomials, version capacities) are generated or transcribed from the specification, not from other implementations.

## 6. Verification

### 6.1 Metrics

Per dataset and per category:

- **Read rate** = correctly decoded expected symbols / expected symbols in supported symbologies.
- **False positives** = returned symbols not present in the ground truth (target: 0).
- **Latency** p50 / p95 / max per image, single-threaded, by resolution and effort preset.

### 6.2 Baselines

zxing-cpp, rxing, rqrr — same images, same machine, same metrics.

### 6.3 Datasets

The current dataset list, terms and gates are in [datasets.md](datasets.md); this section is the original plan.

- [Dynamsoft/datasets-from-dynamsoft](https://github.com/Dynamsoft/datasets-from-dynamsoft): `challenging-images` (68 images, 514 symbols; `annotations.json` with file, format, text and corner points; about 400 symbols in v1 symbologies, the rest Data Matrix/PDF417), `video-based-testing` (1080p videos plus one CSV per video listing the expected set of codes, without per-frame labels), `UPS-Synthetic-Labels` (CC BY 4.0). Downloaded by `tools/fetch-datasets.sh` into `data/`, never committed; used for internal testing only, since most subsets state no license.
- Candidate complement (pending decision): zxing-cpp `test/samples` (Apache-2.0), richer in QR and EAN/UPC images.
- Synthetic set: symbols generated with encoder crates (`qrcode`, `barcoders`) and distorted (rotation, perspective, blur, noise, illumination gradients, inversion, downscaling) with known ground truth.
- Negative set: images without symbols, to measure false positives.

### 6.4 Test types

- Unit tests per stage.
- Property tests (`proptest`) for decoders and math (e.g. Reed-Solomon: any error pattern within capacity is corrected).
- Fuzzing (`cargo-fuzz`) of every decoder entry point.
- Micro-benchmarks (`criterion`) of hot stages.
- Dataset harness (`tools/bench`) producing the metrics in §6.1.

## 7. Roadmap

| Milestone | Content |
|---|---|
| M0 | Requirements, architecture, workspace skeleton, core types and pipeline |
| M1 | Image processing (conversion, local binarizer, downscale), bench harness, dataset fetch, synthetic generator |
| M2 | EAN-13/8, UPC-A/E, add-ons |
| M3 | Code 128, Code 39, ITF |
| M4 | Reed-Solomon, QR detection and decoding |
| M5 | Charsets/ECI, GS1, content classification |
| M6 | Python bindings complete |
| M7 | Performance: SIMD kernels, `rayon`, video tracking module; fix latency targets |

## 8. Open questions

- Add zxing-cpp `test/samples` as a second dataset?
- Final license.
- Whether ISO specification copies are available to the project (otherwise public descriptions and the GS1 General Specifications, which are free, are used).
