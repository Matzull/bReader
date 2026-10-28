# breader — Architecture

Status: draft v0.1 (2026-10-06). See [REQUIREMENTS.md](REQUIREMENTS.md) for the requirement IDs referenced here. This page is the cross-cutting overview; per-family designs are in [symbologies/](symbologies/README.md), and the documentation map is [README.md](README.md).

## 1. Workspace

```
crates/
  breader-core      types, traits, FrameContext, Pipeline. No dependencies.
  breader-imgproc   pixel conversion, preprocessors, binarizers
  breader-ecc       Reed-Solomon: GF(256), GF(929) (PDF417) and generic GF(2^m) (Aztec)
  breader-linear    1D detection and decoders (EAN/UPC, Code 128, Code 39, Code 93, ITF, Codabar, GS1 DataBar)
  breader-qr        QR detection, sampling and decoding
  breader-datamatrix  Data Matrix ECC 200 detection, sampling and decoding
  breader-pdf417    PDF417 detection, row sampling, grid votes and decoding (GF(929) Reed-Solomon)
  breader-aztec     Aztec Code: bullseye search, core fit, reference-grid sampling, spiral decoding (GF(16) to GF(4096) Reed-Solomon, high-level modes)
  breader-content   charset/ECI decoding, GS1 parsing, content classification, dedup
  breader           facade: Reader, default pipeline, feature flags
  breader-ffi       C interface (cdylib and staticlib, generated header include/breader.h; the only crate with unsafe code)
  breader-py        Python bindings (outside the workspace build, built with maturin)
tools/
  bench             dataset harness, regression gate and baseline comparison
  synth             synthetic dataset generator
  video-bench       video dataset harness and gate
  fetch-datasets.sh, check-regression.sh
```

Dependency direction is strictly downward: every module crate depends only on `breader-core` (and `breader-ecc` where needed); only the facade knows about all of them. The one documented exception: `breader-pdf417` also depends on `breader-linear`, because the PDF417 start and stop patterns are found inside the linear scan and shared through `FrameContext::shared` ([D-026](decisions.md#d-026)). A third-party crate can provide a module by depending on `breader-core` alone.

## 2. Pipeline

```
 pixels ─► Ingest ─► ImageView (luma, borrowed)
                         │
          ┌──────────────┴─────────── for each Pass ───────────────────────┐
          │  Preprocessor? ─► image variant   (cached per frame)            │
          │  Binarizer     ─► BitMatrix       (cached per frame, lazily)    │
          │  inversion     ─► flipped copy    (only if the pass asks)       │
          │                                                                 │
          │  SymbologyReader × N:  Detector ─► candidates ─► Decoder ─► Symbol
          └──────────────┬──────────────────────────────────────────────────┘
                         ▼
               PostProcessor × M  (dedup, content parsing)
                         ▼
                    Vec<Symbol>
```

### 2.1 Ingest

`ImageView<'a>` is a borrowed luma plane (width, height, stride). Luma and YUV inputs are used without copying; RGB variants are converted once into an `ImageBuffer` owned by the caller's reader. File decoding lives only in the facade behind the `image` feature.

### 2.2 Passes and escalation

A `Pass` is `{ preprocessor: Option<usize>, binarizer: usize, inverted: bool }`. `Pipeline::passes` orders them from cheapest to most expensive according to the effort preset:

| Effort | Image variants | Binarizers | Inversion |
|---|---|---|---|
| Fast | original | first | none as a pass; all five readers handle polarity themselves: 1D, QR and PDF417 read both polarities in every effort, Data Matrix and Aztec search dark-on-light only at Fast |
| Balanced | original + first preprocessor | first two | yes |
| Exhaustive | all | all | yes |

Fine-grained options override the preset (`try_invert`, `upscale_retry`, later per-stage lists). The pipeline stops early when `max_symbols` is reached or the `time_budget` expires; the budget is checked between passes, never inside one.

**Up-scale retry.** After the last regular pass, if the frame produced no symbol and `max(w, h) <= ScanOptions::upscale_max_side()` (600 for Balanced and Exhaustive, 0 for Fast), the pipeline applies its `upscale` preprocessor (`Upscale`, Catmull-Rom 3x) to the original frame and runs the same pass list on that copy as if it were a new frame, with its own `FrameContext` and a nested `Scratch`. Readers therefore see an ordinary frame (1D readers included); the pipeline maps each symbol's quad back to the original coordinates by the size ratio before the post-processors run. The `time_budget` and `max_symbols` apply across both scans. The preprocessor is registered with `PipelineBuilder::upscale`, not in the regular preprocessor list, so it never takes part in `Exhaustive` passes. `Tracker` disables the retry. Measured: +28 reads on the zxing-cpp samples at Balanced, 0 false positives; running it also when something was found gained one more read and is not done.

### 2.3 FrameContext and caching

`FrameContext` is the only thing readers see. It exposes the original image, the current pass, the pass image and the pass binarization. Derived products are computed on first request and cached for the rest of the frame, so readers sharing a pass share the work. QR, Data Matrix and Aztec use the binarization and so binarize once between them; the linear and PDF417 readers work on grayscale (1D readers read edges on grayscale scanlines, which is more robust to blur than binarized rows) and never trigger binarization. A reader can also publish its own per-frame product through `FrameContext::shared`: the linear scan records the PDF417 start and stop patterns this way, so the PDF417 reader needs no scan of its own, and the Aztec reader leaves a "suspect" flag when a pass saw a bullseye it could not read, which makes the later passes search again.

All buffers live in `Scratch`, owned by the caller and reused between frames, which keeps steady-state scanning allocation-free (NFR-4).

### 2.4 Module traits

| Trait | Object-safe | Role |
|---|---|---|
| `Preprocessor` | yes | Produce an image variant (downscale, contrast normalization) |
| `Binarizer` | yes | Luma → `BitMatrix` (dark = `true`) |
| `SymbologyReader` | yes | One family end to end; registered in the pipeline. `uses_binary()` tells the pipeline whether binarizer-only passes are relevant to it |
| `PostProcessor` | yes | Operate on the final result list |
| `Detector` | no (associated type) | Family-internal: produce candidates |
| `SymbolDecoder` | no (associated type) | Family-internal: candidate → `Symbol` or `Reject` |

Dynamic dispatch happens only at stage boundaries (a handful of calls per pass). Inside a family, detector and decoder are composed statically so hot loops are monomorphized and inlined. Registration is compile-time: `Pipeline::builder().binarizer(..).reader(..).post_processor(..).build()`.

### 2.5 Validation and false-positive control

Validation is part of each decoder, not an optional post-step: a `SymbologyReader` must only push results that passed every check for its symbology (NFR-1). Checks per family:

- **QR**: format information BCH, version information BCH (v ≥ 7), Reed-Solomon success with all blocks, bitstream well-formed until terminator/padding.
- **rMQR**: before Reed-Solomon, the timing patterns of the four edges and of the alignment columns found as boundaries, both quiet rings, the finder with its separators and the other function patterns on the sampled grid, both copies of the BCH(18,6) format information agreeing and naming the sampled size. Reed-Solomon per block with the corrections capped (reserve 2 up to 8 check codewords, 1 above), strict padding after the terminator. See [rmqr](symbologies/rmqr.md).
- **EAN/UPC**: check digit, guard patterns, quiet zones, module-width consistency, ≥ `min_line_agreement` agreeing scanlines.
- **Code 128**: mod-103 checksum, stop pattern, quiet zones, scanline agreement.
- **Code 39**: optional mod-43, inter-character gaps, minimum length, quiet zones, scanline agreement.
- **Code 93**: start, stop and termination bar, every character quantized to the 9-module structure, both mod-47 check characters, valid shift sequences, 4-module quiet zones measured on both sides (never open at the profile border), scanline agreement as Code 39 ([D-049](decisions.md#d-049)).
- **GS1 DataBar**: every character a member of its combination set, mod 79 / 89 / 211 checksum, finder pair or check pattern agreeing with it, pieces with their module counts ([D-037](decisions.md#d-037)).
- **Codabar**: start and stop in A-D with none inside, minimum data length, clear wide/narrow classification of every element, bounded gaps, 10-module quiet zones, one scanline more than `min_line_agreement` ([D-036](decisions.md#d-036)).
- **ITF**: accepted length list, quiet zones, bar-width ratio consistency, scanline agreement (ITF is the most false-positive-prone symbology and is gated hardest).
- **Data Matrix**: before Reed-Solomon, a precheck of the L finder and clock tracks on the candidate, a size that is in the ECC 200 table, and a plausibility bound on the sampled grid's finder and clock mismatch. Reed-Solomon per block with corrections capped below the standard for small blocks (reserve 2 up to 7 check codewords, 1 up to 12). After it, the pad sequence must follow the 253-state rule for corrected or small blocks, and the encodation stream must be well formed (no data after an unlatch it does not allow, no empty payload). See [datamatrix](symbologies/datamatrix.md).
- **PDF417**: before Reed-Solomon, start and stop patterns of the same polarity and module width, row numbers and column count voted from the row indicators, and an erasure budget. Reed-Solomon over GF(929) with `2 * errors + erasures <= ec - reserve` (reserve 2 up to 16 check codewords, 1 above; level 0 detects only). After it, the symbol length descriptor must match exactly and the compaction stream must be well formed. See [pdf417](symbologies/pdf417.md).
- **Aztec**: before Reed-Solomon, a finder gate (80 % of the bullseye, 10 of 12 orientation-mark modules) and a mode message that is itself Reed-Solomon protected, capped at 1 (compact) or 2 (full-range) corrections and consistent with the grid side. Data Reed-Solomon over GF(64) to GF(4096) with the same reserve rule as Data Matrix; data codewords of all zeros or all ones are erasures. After it, bit unstuffing and the high-level tables must parse without leftovers. See [aztec](symbologies/aztec.md).

`Reject` reasons are returned by decoders for diagnostics and tests; they are not part of the public result.

### 2.6 Results

`Symbol` carries symbology, raw bytes, optional text, `Content` classification, `Quad` (corners in symbol order, image coordinates of the original frame, independent of the preprocessor scale), orientation, AIM identifier, `SymbolMeta` (`Qr`, `Linear`, `DataMatrix`, `Pdf417` or `Aztec`) and a quality estimate. `quality` is informational only and never used to accept a result.

## 3. Symbology families

The per-family designs live in [symbologies/](symbologies/README.md) (one page per family, with status and spec references). Sections 3.3 and 3.4 are shared by all families and stay here.

### 3.1 Linear (`breader-linear`)

EAN/UPC, Code 128, Code 39, Code 93, ITF, Codabar and GS1 DataBar over gray scanline profiles: scan plan, edges, runs, per-symbology decoders, aggregation with conflict removal. Design: [symbologies/linear.md](symbologies/linear.md).

### 3.2 QR (`breader-qr`)

Finder patterns, triplets, homography sampling, Reed-Solomon, retries (local threshold, pitch correction, nested symbols). Design: [symbologies/qr.md](symbologies/qr.md). Micro QR (M1 to M4) is read by the same `QrReader` from the finders of the same scan that no decoded symbol consumed: [symbologies/microqr.md](symbologies/microqr.md). rMQR is read by the same `QrReader` in the same candidate loop, from the timing patterns of all four edges: [symbologies/rmqr.md](symbologies/rmqr.md).

### 3.3 Error correction (`breader-ecc`)

GF(256) arithmetic with log/antilog tables built at compile time (`const fn`), syndrome computation, Berlekamp–Massey, Chien search, Forney. Returns corrected count for metadata and quality. Reusable by Data Matrix and PDF417. PDF417 uses the prime field GF(929) (generator 3, roots 3^1 to 3^ec, `block[0]` the highest coefficient, check codewords the negated remainder as in ISO/IEC 15438) through `Gf929`: Berlekamp–Massey seeded with the erasure locator, Chien search and Forney, with `decode` reporting `errors` (changed codewords outside the erasure list) and `erasures` separately so that the caller applies its own bound ([PDF417](symbologies/pdf417.md)). Aztec Code uses `Gf2m<Q>`, a binary extension field with `u16` symbols (GF(16), GF(64), GF(256), GF(1024), GF(4096), first root alpha^1) with the same erasure support and `errors` / `erasures` split ([Aztec](symbologies/aztec.md)).

### 3.4 Content (`breader-content`)

Decoders never interpret charsets. They return raw `data` plus `segments`: contiguous byte ranges, each with a `Charset`. An empty list means the symbology default. Every charset rule lives in one place, and module crates stay independent of each other.

Post-processors, in this order: `Dedup`, `TextDecoder`, `ContentClassifier`.

1. **TextDecoder** sets `text`:
   - **Declared segments:** each segment is decoded with its charset and the results are concatenated. Any unknown ECI or invalid sequence leaves `text = None`; `data` is always kept.
   - **Defaults per symbology:**
     - Code 128: Latin-1.
     - Code 39, Code 93, ITF, Codabar, EAN/UPC: ASCII (Code 93 Full ASCII is expanded by the decoder).
     - QR without segments: same rule as `Undeclared`.
   - **`Undeclared` QR byte mode, in this order:**
     1. strict UTF-8;
     2. Shift JIS, when every byte forms valid Shift JIS and at least one double-byte character appears;
     3. otherwise ISO-8859-1, which is the ISO 18004 default.
   - **ECI mapping:**
     - 0 and 2: CP437.
     - 1 and 3: ISO-8859-1.
     - 4–13 and 15–18: the corresponding ISO-8859 parts.
     - 20: Shift JIS.
     - 21–24: Windows-1250, 1251, 1252 and 1256.
     - 25 and 33: UTF-16 BE and LE.
     - 26: UTF-8.
     - 27 and 170: ASCII.
     - 28: Big5.
     - 29: GB2312.
     - 30: EUC-KR.
     - 31: GBK.
     - 32: GB18030.
     - 34 and 35: UTF-32 BE and LE.
     - 899: binary, so `text = None`.
     - Others: unknown.
   - **Dependency:** `encoding_rs`. CP437 and UTF-32 are implemented locally.
2. **ContentClassifier** sets `content`:
   - **GS1** (`Content::Gs1`) when the AIM identifier marks GS1 data (`]C1`, `]Q3`, `]Q4`) and the GS1 parser accepts the whole payload. EAN/UPC symbols map to AI 01 with the GTIN padded to 14 digits.
   - **URL** for `http://`, `https://` and `ftp://` (case-insensitive) with no whitespace.
   - **Binary** when `text` is `None` or contains control characters other than TAB, CR, LF, GS, RS or EOT.
   - **Text** otherwise.

   A parse failure never rejects a symbol; it only lowers the classification.
3. **GS1 parser** — element strings with GS (0x1D) separators. It uses a table of AIs with their format (numeric or alphanumeric, fixed or variable length, maximum length) and the list of predefined fixed lengths. It validates digits, check digits (GTIN, SSCC, GLN and other mod-10 keys) and dates (YYMMDD, where DD = 00 is allowed). It produces `Gs1Element { ai, value }`. `gs1::hri(&[Gs1Element])` renders the bracketed human-readable form.

Further semantic parsers (vCard, Wi-Fi, GS1 Digital Link, ...) are added as `Content` variants and classifier rules.

### 3.5 Data Matrix (`breader-datamatrix`, milestone M8)

ECC 200, square, rectangular and DMRE sizes; candidates from pooled dark-pixel components, L finder and clock fit, Annex F placement, all encodation modes. Design: [symbologies/datamatrix.md](symbologies/datamatrix.md).

### 3.6 PDF417 (`breader-pdf417`, milestone M9)

Stacked linear symbology read from gray profiles: seeds from the linear scan's pattern hits, boundary growth, row lines, grid votes, GF(929) Reed-Solomon with erasures. Design and as-built notes: [symbologies/pdf417.md](symbologies/pdf417.md).

### 3.7 Aztec Code (`breader-aztec`, milestone M10)

Bullseye finder on the binary plane (both polarities through the shared `Plane` view), mode message with orientation marks, layer spiral with a reference grid, Reed-Solomon over GF(16) to GF(4096). Design and as-built notes: [symbologies/aztec.md](symbologies/aztec.md).

## 4. Facade (`breader`)

`Reader` owns a `Pipeline`, `ScanOptions`, `Scratch` and a luma conversion buffer. `Reader::new()` builds the default pipeline for the enabled features; `Reader::from_pipeline` accepts any custom pipeline. Cargo features: `qr`, `linear`, `datamatrix`, `pdf417`, `aztec` (all five default), `image`, `rayon`.

## 5. Performance plan (M7)

Measured on 2026-10-07 on a Ryzen 5 5500U (a historical profile; the current reference machine is a Ryzen 7 3700X, see [BASELINES](BASELINES.md#status-2026-10-20-new-machine-ryzen-7-3700x)), single thread, in ms per image:

| | challenging (3 MP) | synth (1.1 MP) |
|---|---|---|
| Balanced, all | 60.4 | 17.3 |
| Balanced, 1D only | 41.3 | 9.8 |
| Balanced, QR only | 23.9 | 9.4 |
| Fast, all | 10.9 | 2.7 |
| Fast, QR only | 8.6 | 2.3 |
| Local binarizer / Otsu / downscale ×2 | 6.9 / 3.7 / 4.8 | 2.0 / 2.4 / 1.8 |

Work items, in priority order. Each must keep read counts and 0 FP unchanged.

1. **Image kernels** (`breader-imgproc`) — done: all ≤ 0.4 ns/px at 1080p, bit-identical output, SSE2 only for bit packing and RGBA/BGRA luma. Luma conversion, Otsu, local binarizer and downscale are at about 2 ns/px; the target is ≤ 0.5 ns/px. Rewrite them to be autovectorization-friendly first: row-wise passes, integer arithmetic, no bounds checks in inner loops. Explicit SIMD (`std::arch`, AVX2 / NEON with runtime detection and a scalar fallback) is allowed only in a `simd` module with one `unsafe` block per kernel, each tested against the scalar version (NFR-9). Downscale ×2 gets a specialized path.
2. **1D in Balanced** — done: 41.3 → 14.4 ms/img on challenging, 9.6 → 3.3 on synth, no read change:
   - Non-axis scanlines step with fixed-point DDA and nearest or linear sampling, instead of bilinear sampling with floats.
   - The smoothed edge detector runs only on lines whose unsmoothed pass produced a partial match (start pattern found, decode failed).
   - Measure the read contribution of the downscaled variant for 1D, and keep it only if it adds reads.
3. **QR polarity sharing** (done): run boundaries are extracted once per row and the 1:1:3:1:1 test runs for both parities; finders and triplets carry their polarity. Balanced time is unchanged, because confirmation and triplet work still run per polarity. Fast now reads inverted QR for about 10 % more QR time (held-out set: 159 → 179 QR in Fast).
4. **Parallelism** (`rayon` feature) — done. Measured on idle hardware with 12 threads, rayon cut challenging Balanced from 23.7 to 13.2 ms/img and Exhaustive from 53.6 to 26.8, and synth Fast from 1.63 to 1.48. Speculative QR triplet decoding was built, made exactly deterministic, then removed because it gave no gain. Original plan: image variants and binarizations are computed in parallel; inside a pass, 1D scanline chunks and QR candidates are processed in parallel. Results are merged in a fixed order so output stays deterministic (NFR-3). Single-threaded remains the default.
5. **Video tracker** (`breader::Tracker`) — done. The periodic full scan became a rolling strip scan: 3 horizontal strips with 50 % overlap, one strip at full effort per frame. On 12 videos: 40/74 reads against 38 for stateless Balanced, 12.6 ms against 15.3 ms mean per frame, 24 flickers against 212. Original plan:
   - **Each frame:** scans the regions of symbols from recent frames first, using a cropped `ImageView` with the quad expanded by 50 %, at full effort for those regions. It then runs a Fast global scan, followed by a Balanced global scan every N frames (configurable, default 10) or when nothing is tracked.
   - **Tracks:** each holds symbology, data, last quad, first and last frame seen, and hit count. A track is reported as `New`, `Updated` or `Lost` (not seen for K frames, default 5).
   - **Output:** stable symbols across frames, which avoids the flicker of per-frame results.
   - **Validation:** the Dynamsoft `video-based-testing` set, with code sets per video taken from its CSVs. The metrics are time to first read, the fraction of each video's codes ever read, FPs, and per-frame latency.

## 6. Planned extensions

- **More symbologies**: others as new `SymbologyReader` crates reusing the cached binarizations.

### Known technical debt

- QR (`sample.rs`: `Warped`, `timing_warp`) and Data Matrix (`sample.rs`: `Warp`, `clock_warp`) carry two near-identical per-axis pitch corrections.
- Data Matrix keeps its own `f64` homography `Mapper` instead of `breader_core::Homography`.

Unify only with a gate run (`tools/check-regression.sh --tier b`) proving that reads are unchanged.
