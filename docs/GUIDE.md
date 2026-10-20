# Options guide

How to configure breader for your workload. API reference: `cargo doc --open -p breader`. Python equivalents are given next to each Rust item.

## Basics

```rust
use breader::{Effort, Reader, ScanOptions};

let mut reader = Reader::with_options(ScanOptions::with_effort(Effort::Balanced));
let symbols = reader.scan_pixels(&pixels, breader::PixelFormat::Luma8, width, height, width)?;
```

```python
reader = breader.Reader("balanced")
symbols = reader.scan(gray_array)
```

A `Reader` keeps scratch buffers between scans: create it once and reuse it. Scanning takes `&mut self`, so use one `Reader` per thread (in Python, one `Reader` per thread to scale; calls on a shared one are serialized).

Input is converted to an 8-bit luma plane. `Luma8`, `Nv12`, `Nv21` and `I420` are scanned in place (only the Y plane is read); `Rgb8`, `Rgba8` and `Bgra8` are converted into a reused buffer. `stride` is the row pitch in bytes.

## ScanOptions

| Rust field | Python option | Default | Meaning |
|---|---|---|---|
| `symbologies` | `symbologies` | all | Symbologies to look for. Readers whose symbologies are all disabled are skipped, so restricting this saves time. |
| `effort` | `effort` | `Balanced` | Preset, see [Effort presets](#effort-presets). |
| `try_invert` | `try_invert` | by effort | `Some(true)` adds light-on-dark passes, `Some(false)` removes them. `None`: on unless the effort is `Fast`. |
| `escalation` | `escalation` | by effort | When to stop running passes, see [Escalation](#escalation). |
| `max_symbols` | `max_symbols` | `0` | Stop after this many symbols (0 = unlimited). The result is truncated to this count. |
| `time_budget` | `time_budget_ms` | none | Checked between passes. A pass in progress is never interrupted, so a scan can overshoot by one pass. |
| `upscale_retry` | `upscale_max_side` | by effort | Largest `max(width, height)` of a frame that is rescanned 3x up-scaled when nothing was found, see [Up-scale retry](#up-scale-retry). `Some(0)` disables; `None`: 600 for `Balanced` and `Exhaustive`, off for `Fast`. |
| `min_line_agreement` | `min_line_agreement` | `2` | Independent scanlines that must produce the same 1D result. 1 reads more and raises false positives. |
| `ean_addon` | `ean_addon` | `Ignore` | `Ignore`, `Optional` (report the EAN-2/EAN-5 add-on if present, accept the symbol without it) or `Required`. |
| `code39.full_ascii` | `code39_full_ascii` | `false` | Decode Code 39 Full ASCII escape pairs. |
| `code39.require_checksum` | `code39_checksum` | `false` | Require a valid mod-43 check character (it stays in the payload). |
| `code39.min_length` | `code39_min_length` | `3` | Minimum data characters. |
| `itf.allowed_lengths` | `itf_lengths` | `[14, 8, 10, 12, 16, 18]` | Accepted digit counts. Empty means any even length >= `min_length`. Lengths other than 14 need 4 agreeing scanlines (14 needs 3) while the list is the default one. |
| `itf.min_length` | `itf_min_length` | `6` | Minimum digits. |

GS1 DataBar (Omnidirectional/Truncated, Limited, Expanded in one row) has no options either and is on by default ([D-037](decisions.md#d-037)): `data` is the GS1 element string (FNC1 as GS), AIM `]e0`, `content` is GS1; `meta.databar_segments` counts the symbol characters of an Expanded row. Stacked, Stacked Omnidirectional and Expanded Stacked are read too and reported as `DataBar` and `DataBarExpanded` ([D-048](decisions.md#d-048)); `meta.databar_rows` is their number of rows (None for one row). Codabar has no options. It is on by default with the guards of [D-036](decisions.md#d-036): `data` excludes the start and stop characters (A-D), which `meta.codabar_start_stop` (Python) / `LinearMeta::codabar_start_stop` (Rust) report; at least 4 data characters, 10-module quiet zones, and one more agreeing scanline than `min_line_agreement`. Disable it with `symbologies` if you do not expect it.

ITF has no start/stop redundancy strong enough to reject truncated reads, so the default accepts 14 digits (GTIN-14, check digit verified, 3 agreeing scanlines) and 8, 10, 12, 16 and 18 digits with 4 agreeing scanlines ([D-041](decisions.md#d-041)). All ITF symbols need a quiet zone of 10 narrow widths on both sides. A list you set yourself is taken as chosen: every listed length then needs the 3 lines. Widen it only for formats you expect, e.g. `[6, 14]`.

## Effort presets

A scan is a list of passes. A pass is an image variant (original or a preprocessed one), a binarizer, and a polarity. Cheap passes run first.

| Effort | Passes | Intended use |
|---|---|---|
| `Fast` | original image, first binarizer, normal polarity | Video, real time |
| `Balanced` | original and first preprocessor, first two binarizers, both polarities | Default |
| `Exhaustive` | every preprocessor and binarizer, both polarities | Hard stills |

Measured with the default pipeline on an AMD Ryzen 7 3700X (release build, single thread, one image at a time, decode call only), on an idle machine, 2026-10-20, see [Status 2026-10-20](BASELINES.md#status-2026-10-20-new-machine-ryzen-7-3700x) (earlier numbers were from a Ryzen 5 5500U). Latency is the median of 5 interleaved runs; the read rate is reads over expected symbols, and false positives are 0 everywhere:

| Dataset | Effort | Read rate | False positives | Latency p50 / p95 (ms) |
|---|---|---|---|---|
| challenging-images (448 symbols) | Fast | 74.3 % | 0 | 5.0 / 40.5 |
| challenging-images | Balanced | 91.7 % | 0 | 20.8 / 146.9 |
| challenging-images | Exhaustive | 94.6 % | 0 | 53.9 / 409.3 |
| BarBeR 1/8 subset (1093 symbols) | Fast | 63.6 % | 0 | 5.8 / 23.3 |
| BarBeR 1/8 subset | Balanced | 74.1 % | 0 | 23.0 / 112.1 |
| BarBeR 1/8 subset | Exhaustive | 76.4 % | 0 | 59.7 / 301.8 |
| synth (511 symbols) | Fast | 87.3 % | 0 | 1.6 / 9.4 |
| synth | Balanced | 98.8 % | 0 | 5.7 / 48.9 |
| synth | Exhaustive | 99.0 % | 0 | 13.3 / 122.2 |

For comparison, zxing-cpp at default options reads 86.8 % of challenging-images (p50 21.0 ms) and 70.2 % of the BarBeR subset (p50 29.5 ms, through Python); Balanced is faster on both and reads more.

`Exhaustive` reads more than `Balanced` on every set above, at 2.5 to 3 times the median latency (and a long tail: p95 up to 360 ms on 3 MP photos). Measure on your own images.

## Escalation

`Escalation` decides when to stop running further passes:

- `All`: run every pass of the preset. Finds the most symbols; default for `Balanced` and `Exhaustive`.
- `UntilFound`: stop after the first pass that ends with at least one symbol. Default for `Fast`.
- `UntilStable`: after a symbol was found, stop at the first pass that adds none.

In Python the values are `"all"`, `"found"` and `"stable"`.

Use `UntilFound` with `max_symbols = 1` for "read one code as fast as possible". Use `All` when images hold several symbols.

## Time budget

`time_budget` bounds latency softly: it is compared with the elapsed time before each pass. Combine it with `Exhaustive` to get "best effort within N ms"; passes are ordered cheap first, so a tight budget behaves like a lower effort.

## Up-scale retry

Small images often hold symbols with modules of about one pixel (thumbnails, icons, screenshots of codes) that no regular pass can sample. When every regular pass of a frame returns no symbol and the frame is at most `upscale_retry` pixels on its longer side (600 by default for `Balanced` and `Exhaustive`), the pipeline enlarges it 3x with a cubic (Catmull-Rom) kernel and runs the regular passes once more on the copy. Corners are reported in the coordinates of the original frame. The retry never runs when something was found, so multi-symbol images are not rescanned, and it costs time only on small frames without a symbol: a blank 600x600 frame takes about 15 ms more at `Balanced` (a noise-filled one about 140 ms), so lower the limit (or set 0) when most of your small frames are expected to be empty. `Tracker` turns it off.

## Parallelism (rayon)

Build with `--features rayon` to parallelize the image kernels, 1D scanline chunks and QR candidates. Output is identical to the single-threaded build. By default rayon uses its global pool (all cores, or `RAYON_NUM_THREADS`). To bound the threads for a scan, run it inside a pool:

```rust
let pool = rayon::ThreadPoolBuilder::new().num_threads(4).build()?;
let symbols = pool.install(|| reader.scan(image));
```

Parallelism helps large images and `Exhaustive`; on small frames the overhead can outweigh the gain, and video at `Fast` is usually better served by one thread per stream. Measured with 12 threads: challenging-images Balanced 23.7 to 13.2 ms/img, Exhaustive 53.6 to 26.8 (see ARCHITECTURE.md).

## Video: Tracker

`Tracker` keeps symbols alive across frames and spends the scan effort where it pays:

```rust
use breader::{ScanOptions, Tracker, TrackerConfig};

let mut tracker = Tracker::new(ScanOptions::default(), TrackerConfig::default());
let update = tracker.process(frame);        // ImageView of the luma frame
for event in &update.events { /* New / Updated / Lost with a track id */ }
for track in tracker.tracks() { /* id, symbol, first_frame, last_frame, hits */ }
```

Each frame, in order: a scan of the region around each live track (`roi_effort`, margin `roi_margin`), a whole-frame scan (`global_effort`, skipped when `strips == 1`), and one horizontal strip at `full_effort`, cycling through `strips` strips with `strip_overlap`. Tracks match detections by symbology, data and distance. A track is lost after `lost_after` frames without a detection.

| `TrackerConfig` field | Default | Meaning |
|---|---|---|
| `strips` | `3` | Strips per frame cycle. 1 scans the whole frame at `full_effort` every frame. |
| `strip_overlap` | `0.5` | Extra strip height, as a fraction of the strip height. |
| `lost_after` | `5` | Frames without detection before `Lost`. |
| `roi_margin` | `0.5` | Growth of the track bounding box on each side. |
| `roi_effort` | `Balanced` | Effort inside track regions. |
| `global_effort` | `Fast` | Effort for the whole-frame scan. |
| `full_effort` | `Balanced` | Effort for the rolling strip. |

On 12 test videos the default tracker read 40 of 74 symbols against 38 for stateless Balanced, at 12.6 ms against 15.3 ms mean per frame, with 24 flickers against 212. `ScanOptions::effort` is ignored by the tracker; the efforts come from the config. The Python package has no tracker yet.

## Content and GS1

After decoding, post-processors fill `Symbol::text`, `Symbol::content` and `Symbol::segments`:

- `text` is set when the charset is known: ASCII, ISO-8859-1 (Code 128), Shift JIS (QR kanji), ECI designators, or a heuristic for undeclared QR byte mode. `data` always holds the raw bytes.
- `content` is `Text`, `Binary`, `Url` or `Gs1(elements)`. GS1 is recognized in Code 128 with FNC1, GS1 QR (FNC1 in first position) and EAN/UPC (as GTIN).

```rust
if let breader::Content::Gs1(elements) = &symbol.content {
    for e in elements { println!("({}) {}", e.ai, e.value); }
    println!("{}", breader::gs1::hri(elements)); // (01)09506000134352...
}
// Parse a string yourself:
let elements = breader::gs1::parse("0109506000134352\x1d10ABC")?;
```

```python
s.content      # "text" | "binary" | "url" | "gs1" | "unparsed"
s.gs1          # [("01", "09506000134352"), ...]
s.gs1_hri      # "(01)09506000134352"
```

A GS1 parse failure never rejects a symbol; it only leaves `content` as `Text` or `Binary`.

## Custom pipelines

`Reader::new()` uses `breader::default_pipeline()`. For your own modules (a binarizer, a preprocessor, a reader for another symbology, a post-processor) build a `Pipeline` and pass it to `Reader::from_pipeline`. See `crates/breader/examples/custom_module.rs`.

## Examples

```
cargo run --example scan_file --features image -- data/synth/clean/clean_0001.png
cargo run --example scan_raw --features image -- data/synth/clean/clean_0001.png
cargo run --example tracker --features image -- frames_dir/
cargo run --example options --features image -- data/synth/multi/multi_0001.png
cargo run --example custom_module
```
