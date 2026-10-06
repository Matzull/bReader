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

1. Finder pattern candidates from the binary matrix (1:1:3:1:1 run ratio along rows, confirmed vertically and diagonally), clustered.
2. Triplet selection by geometry (right angle, similar module size), estimated version, alignment pattern search for v ≥ 2.
3. Perspective transform from the four reference points; grid sampling with local refinement.
4. Format and version information (BCH), unmasking, codeword extraction, de-interleaving, Reed-Solomon (`breader-ecc`), bitstream decoding with ECI.
5. Mirrored read retried when format information fails.

### 3.3 Error correction (`breader-ecc`)

GF(256) arithmetic with log/antilog tables built at compile time (`const fn`), syndrome computation, Berlekamp–Massey, Chien search, Forney. Returns corrected count for metadata and quality. Reusable by Data Matrix and PDF417 later (PDF417 needs GF(929), so the field becomes a trait parameter when that lands).

## 4. Facade (`breader`)

`Reader` owns a `Pipeline`, `ScanOptions`, `Scratch` and a luma conversion buffer. `Reader::new()` builds the default pipeline for the enabled features; `Reader::from_pipeline` accepts any custom pipeline. Cargo features: `qr`, `linear` (default), `image`, `rayon`.

## 5. Planned extensions

- **Video tracking**: a `Tracker` wrapping `Reader` keeps symbols from previous frames, scans their neighborhoods first and stabilizes results across frames. Lives outside the per-frame pipeline so the core stays stateless.
- **Parallelism** (`rayon`): parallel scanlines and parallel candidate decoding within a pass; results merged deterministically.
- **SIMD**: binarization and luma conversion kernels with scalar fallback (NFR-9).
- **More symbologies**: Data Matrix, PDF417, Aztec as new `SymbologyReader` crates reusing the cached binarizations.
