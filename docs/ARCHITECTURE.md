# breader — Architecture

Status: draft v0.1 (2026-10-06). See [REQUIREMENTS.md](REQUIREMENTS.md) for the requirement IDs referenced here.

## 1. Workspace

```
crates/
  breader-core      types, traits, FrameContext, Pipeline. No dependencies.
  breader-imgproc   pixel conversion, preprocessors, binarizers
  breader-ecc       Reed-Solomon over GF(256)
  breader-linear    1D detection and decoders (EAN/UPC, Code 128, Code 39, ITF)
  breader-qr        QR detection, sampling and decoding
  breader-content   charset/ECI decoding, GS1 parsing, content classification, dedup
  breader           facade: Reader, default pipeline, feature flags
  breader-py        Python bindings (outside the workspace build, built with maturin)
tools/
  bench             dataset harness and baseline comparison
  fetch-datasets.sh
```

Dependency direction is strictly downward: every module crate depends only on `breader-core` (and `breader-ecc` where needed); only the facade knows about all of them. A third-party crate can provide a module by depending on `breader-core` alone.

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
| Fast | original | first | no |
| Balanced | original + first preprocessor | first two | yes |
| Exhaustive | all | all | yes |

Fine-grained options override the preset (`try_invert`, later per-stage lists). The pipeline stops early when `max_symbols` is reached or the `time_budget` expires; the budget is checked between passes, never inside one.

### 2.3 FrameContext and caching

`FrameContext` is the only thing readers see. It exposes the original image, the current pass, the pass image and the pass binarization. Derived products are computed on first request and cached for the rest of the frame, so readers sharing a pass share the work: the QR reader and a future Data Matrix reader binarize once. Grayscale-based readers (1D readers read edges on grayscale scanlines, which is more robust to blur than binarized rows) never trigger binarization.

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
- **EAN/UPC**: check digit, guard patterns, quiet zones, module-width consistency, ≥ `min_line_agreement` agreeing scanlines.
- **Code 128**: mod-103 checksum, stop pattern, quiet zones, scanline agreement.
- **Code 39**: optional mod-43, inter-character gaps, minimum length, quiet zones, scanline agreement.
- **ITF**: accepted length list, quiet zones, bar-width ratio consistency, scanline agreement (ITF is the most false-positive-prone symbology and is gated hardest).

`Reject` reasons are returned by decoders for diagnostics and tests; they are not part of the public result.

### 2.6 Results

`Symbol` carries symbology, raw bytes, optional text, `Content` classification, `Quad` (corners in symbol order, image coordinates of the original frame, independent of the preprocessor scale), orientation, AIM identifier, `SymbolMeta` (`Qr` or `Linear`) and a quality estimate. `quality` is informational only and never used to accept a result.

## 3. Symbology families (planned design)

### 3.1 Linear (`breader-linear`)

`LinearReader` returns `uses_binary() == false` and `handles_inversion() == true`: it reads grayscale profiles and decodes both polarities from the same edges, so the pipeline runs it once per image variant.

Internal modules and data flow for one pass:

```
ScanPlan ─► Scanline ─► profile (u8) ─► Edges ─► Runs ─┬─ dark bars  ─┐
                                                        └─ light bars ─┤
                       LinearDecoder × K (EAN/UPC, 128, 39, ITF) ◄─────┘
                                   │ LineHit
                                   ▼
                    Aggregator ◄─► Refinement ─► Symbol
```

1. **ScanPlan** — scanlines for the pass image, ordered center-out so early hits come quickly. The spacing is computed from the image extent perpendicular to each direction, capped at 48 / 24 / 12 px (Fast / Balanced / Exhaustive), and multiplied by 1.5 for non-axis directions. The cap keeps small symbols in large photos reachable.

   | Effort | Directions |
   |---|---|
   | Fast | horizontal, vertical |
   | Balanced | 8 (every 22.5°) |
   | Exhaustive | 12 (every 15°) |

2. **Profile** — samples along the scanline at 1 px steps: direct reads when axis-aligned (vertical lines in one row-major sweep), bilinear interpolation otherwise.
3. **Edges** — central difference of the profile. An edge is a local extremum with `|d| ≥ max(min_edge, 0.12 · local_range, noise term)`, where the noise is estimated on flat blocks only and the threshold is capped at 0.3 · range. Signs must alternate; for consecutive same-sign extrema, the stronger one is kept. Each edge is then moved to the half-level crossing between the neighbouring extremes, which gives sub-pixel positions and corrects print gain. A [1 2 1]-smoothed variant runs as well in Balanced and Exhaustive (it is the only variant in Fast) for noisy lines; unsmoothed is the default because smoothing merges 1.5–2 px bars.
4. **Runs** — widths between consecutive edges, edge positions, and the polarity of the first run. Decoders read both polarities by swapping the bar/space roles. They never allocate.
5. **LinearDecoder** (family-internal trait, static dispatch through a tuple of decoders): `decode(&self, runs, opts, out: &mut Vec<LineHit>)`. Each decoder tries both reading directions:
   - finds its start (or reversed stop) pattern;
   - checks quiet zones against the estimated module width;
   - matches characters by normalizing runs to the character's module count, with print-gain bias and module-width drift (≤ 1.8×) estimated from guard patterns;
   - accepts a character only when its best match beats the second best by a margin; at most one weak character is allowed when a check digit protects it;
   - validates check characters, stop pattern and length rules.

   `LineHit` holds the symbology, text, add-on, polarity, positions along the line and module width; the reader maps positions to image points.
6. **Aggregator** — groups hits by (symbology, text, polarity) with single-link proximity: center distance below the longer segment, any direction, so lines at different angles confirm each other. A group becomes a `Symbol` with at least `min_line_agreement` lines (ITF: at least 3).
   - **Conflicts:** if two groups of the same family overlap (segment and rectangle overlap) with different text, the one with fewer lines survives only when the other has at least twice as many; otherwise both are dropped (NFR-1). This also removes partial reads such as a UPC-E inside an EAN-13 or 12 digits of an ITF-14.
   - **Refinement:** a group that is still short of agreement gets parallel lines on both sides (at most 24 per side, stopping after 3 misses, at most 48 seeds per pass). Adjacent lines see nearly the same pixels, so a group backed by a single line of the sparse plan needs `min_line_agreement + 1` lines.
   - **Coordinates:** the quad comes from the endpoint extents along and across the mean direction, mapped to original-frame coordinates with `FrameContext::scale`.

EAN/UPC specifics: EAN-13 encodes its first digit in the L/G parity of the left half. An EAN-13 with leading 0 is reported as UPC-A (12 digits) when UPC-A is enabled, and as EAN-13 otherwise. UPC-E is expanded to UPC-A only for check digit validation; the reported data is the 8-digit UPC-E. Add-ons are searched after the end guard (gap of 5–14 modules, start pattern 1011) and need their own line agreement. Minimum quiet zones are 4.5 / 5.5 / 6.5 modules for EAN-13 and UPC-A / EAN-8 / UPC-E. A symbol whose end guard cannot be verified is never accepted, even when the digits and check digit would be. EAN-2 parity comes from the value mod 4, EAN-5 parity from its weighted checksum. `EanAddonPolicy` decides acceptance.

### 3.2 QR (`breader-qr`)

`QrReader` uses the binarized pass image (`uses_binary() == true`) and runs on every pass; inverted passes cover light-on-dark symbols.

Modules: `detect` (finder patterns, triplets), `alignment` (Annex E table), `sample` (homography and grid sampling), `spec` (dimensions, block layouts), `decode` (grid → payload), `reader` (orchestration).

1. **Finder patterns** — rows of the binary matrix are scanned with a step of 3 / 2 / 1 rows (Fast / Balanced / Exhaustive) for the dark-light-dark-light-dark ratio 1:1:3:1:1, with tolerance per run: unit runs in [0.5, 1.5]·u, the center run in [2, 4]·u. Each hit is cross-checked vertically and along one diagonal through its center; the center is refined to the mean of the horizontal and vertical estimates, and the module size is their mean. Hits whose centers fall within one finder width of each other are merged, and the number of confirmations is kept.
2. **Triplets** — among the strongest finders (at most 30), triples are kept when their module sizes agree (max/min ≤ 1.6), the angle at the corner finder is in [60°, 120°], and the two legs differ by at most a factor of 1.6. The corner finder is top-left. Top-right and bottom-left are assigned from the sign of the cross product. A mirrored symbol therefore produces a transposed grid, which the decoder handles. The estimated dimension is `(|TL−TR| + |TL−BL|) / (2·module) + 7`, rounded to the nearest `4k + 1`. Triplets are tried best-score first. A finder may be reused if the triplet that claimed it failed to decode.
3. **Fourth point** — for version ≥ 2, the bottom-right alignment pattern is searched near its predicted position: the parallelogram prediction moved 3 modules inward, within a window of ±4 modules. The search looks for a dark-light-dark 1:1:1 center with module size consistent with the finders. If it is found, a homography maps the module coordinates (3.5, 3.5), (d−3.5, 3.5), (3.5, d−3.5) and (d−6.5, d−6.5) to the image; otherwise the parallelogram (affine) estimate is used.
4. **Sampling** — before alignment search or full sampling, each dimension hypothesis passes cheap plausibility tests: the three finder blocks (8x8 with separators) must match under the mapping (at most 48 of 192 modules wrong), and for dimensions below 45 without a timing-verified hint the timing patterns must alternate (at most 35 % wrong). `sample_grid` repeats both checks on the sampled grid (timing row/column first, then finder blocks) before reading the data area. Refining finder centers and the plain-parallelogram retry only run when some grid passed these tests. Failure budgets are separate for detected and timing-recovered triplets and scale with effort. Module centers are mapped through the homography. Modules under 3 px are read at the center; larger modules take the majority of 5 sub-samples. When version ≥ 7, the version information read from the first sampling overrides the estimate, and the grid is resampled. If decoding fails, dimensions ±4 are tried.
5. **Decoding** — format information (BCH (15,5), both copies, nearest valid word with distance ≤ 3), version information (BCH (18,6), distance ≤ 3), unmasking, codeword placement, de-interleaving per the block layout, Reed-Solomon on every block (`breader-ecc`), and the bitstream (numeric, alphanumeric, byte, kanji, ECI, FNC1 in first and second position; Structured Append headers are parsed and ignored). If format information fails, the transposed grid is tried (mirrored symbol). Any block failing RS rejects the symbol.
6. **Results** — the quad maps module corners (0,0), (d,0), (d,d), (0,d) to the image, scaled to original-frame coordinates. Candidates whose center falls inside a symbol already decoded in this frame are skipped. `QrMeta` records version, level, mask, corrected errors, ECI and mirroring. The AIM identifier follows ISO/IEC 15424: `]Q1` by default, `]Q2` with ECI, `]Q3`/`]Q4` with FNC1 in first position (without/with ECI), and `]Q5`/`]Q6` with FNC1 in second position.

Later improvements, driven by the benchmarks: piecewise homographies over all alignment patterns for large versions, grayscale sampling, and recovery of a missing finder from two finders plus timing patterns.

### 3.3 Error correction (`breader-ecc`)

GF(256) arithmetic with log/antilog tables built at compile time (`const fn`), syndrome computation, Berlekamp–Massey, Chien search, Forney. Returns corrected count for metadata and quality. Reusable by Data Matrix and PDF417 later (PDF417 needs GF(929), so the field becomes a trait parameter when that lands).

### 3.4 Content (`breader-content`)

Decoders never interpret charsets. They return raw `data` plus `segments`: contiguous byte ranges, each with a `Charset`. An empty list means the symbology default. Every charset rule lives in one place, and module crates stay independent of each other.

Post-processors, in this order: `Dedup`, `TextDecoder`, `ContentClassifier`.

1. **TextDecoder** sets `text`:
   - **Declared segments:** each segment is decoded with its charset and the results are concatenated. Any unknown ECI or invalid sequence leaves `text = None`; `data` is always kept.
   - **Defaults per symbology:**
     - Code 128: Latin-1.
     - Code 39, ITF, EAN/UPC: ASCII.
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

## 4. Facade (`breader`)

`Reader` owns a `Pipeline`, `ScanOptions`, `Scratch` and a luma conversion buffer. `Reader::new()` builds the default pipeline for the enabled features; `Reader::from_pipeline` accepts any custom pipeline. Cargo features: `qr`, `linear` (default), `image`, `rayon`.

## 5. Performance plan (M7)

Measured on 2026-10-07 on a Ryzen 5 5500U, single thread, in ms per image:

| | challenging (3 MP) | synth (1.1 MP) |
|---|---|---|
| Balanced, all | 60.4 | 17.3 |
| Balanced, 1D only | 41.3 | 9.8 |
| Balanced, QR only | 23.9 | 9.4 |
| Fast, all | 10.9 | 2.7 |
| Fast, QR only | 8.6 | 2.3 |
| Local binarizer / Otsu / downscale ×2 | 6.9 / 3.7 / 4.8 | 2.0 / 2.4 / 1.8 |

Work items, in priority order. Each must keep read counts and 0 FP unchanged.

1. **Image kernels** (`breader-imgproc`). Luma conversion, Otsu, local binarizer and downscale are at about 2 ns/px; the target is ≤ 0.5 ns/px. Rewrite them to be autovectorization-friendly first: row-wise passes, integer arithmetic, no bounds checks in inner loops. Explicit SIMD (`std::arch`, AVX2 / NEON with runtime detection and a scalar fallback) is allowed only in a `simd` module with one `unsafe` block per kernel, each tested against the scalar version (NFR-9). Downscale ×2 gets a specialized path.
2. **1D in Balanced**:
   - Non-axis scanlines step with fixed-point DDA and nearest or linear sampling, instead of bilinear sampling with floats.
   - The smoothed edge detector runs only on lines whose unsmoothed pass produced a partial match (start pattern found, decode failed).
   - Measure the read contribution of the downscaled variant for 1D, and keep it only if it adds reads.
3. **QR polarity sharing**: `QrReader::handles_inversion() == true`. The finder search detects both polarities from one run-length pass over the binary matrix, candidates carry their polarity, and sampling XORs it. This removes the inverted copy and the second finder scan.
4. **Parallelism** (`rayon` feature): image variants and binarizations are computed in parallel; inside a pass, 1D scanline chunks and QR candidates are processed in parallel. Results are merged in a fixed order so output stays deterministic (NFR-3). Single-threaded remains the default.
5. **Video tracker** (`breader::Tracker`, wraps a `Reader`):
   - **Each frame:** scans the regions of symbols from recent frames first, using a cropped `ImageView` with the quad expanded by 50 %, at full effort for those regions. It then runs a Fast global scan, followed by a Balanced global scan every N frames (configurable, default 10) or when nothing is tracked.
   - **Tracks:** each holds symbology, data, last quad, first and last frame seen, and hit count. A track is reported as `New`, `Updated` or `Lost` (not seen for K frames, default 5).
   - **Output:** stable symbols across frames, which avoids the flicker of per-frame results.
   - **Validation:** the Dynamsoft `video-based-testing` set, with code sets per video taken from its CSVs. The metrics are time to first read, the fraction of each video's codes ever read, FPs, and per-frame latency.

## 6. Planned extensions

- **More symbologies**: Data Matrix, PDF417, Aztec as new `SymbologyReader` crates reusing the cached binarizations.
