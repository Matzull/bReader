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
| `itf.allowed_lengths` | `itf_lengths` | `[14]` | Accepted digit counts. Empty means any even length >= `min_length`. |
| `itf.min_length` | `itf_min_length` | `6` | Minimum digits. |

Codabar has no options. It is on by default with the guards of [D-036](decisions.md#d-036): `data` excludes the start and stop characters (A-D), which `meta.codabar_start_stop` (Python) / `LinearMeta::codabar_start_stop` (Rust) report; at least 4 data characters, 10-module quiet zones, and one more agreeing scanline than `min_line_agreement`. Disable it with `symbologies` if you do not expect it.

ITF has no start/stop redundancy strong enough to reject truncated reads, so the default accepts only 14 digits (GTIN-14). Widen `allowed_lengths` only for formats you expect, e.g. `[6, 14]`.

## Effort presets

A scan is a list of passes. A pass is an image variant (original or a preprocessed one), a binarizer, and a polarity. Cheap passes run first.

| Effort | Passes | Intended use |
|---|---|---|
| `Fast` | original image, first binarizer, normal polarity | Video, real time |
| `Balanced` | original and first preprocessor, first two binarizers, both polarities | Default |
| `Exhaustive` | every preprocessor and binarizer, both polarities | Hard stills |

Measured with the default pipeline on an AMD Ryzen 5 5500U (release build, single thread, one image at a time, decode call only), see [BASELINES.md](BASELINES.md):

| Dataset | Effort | Read rate | False positives | Latency p50 / p95 (ms) |
|---|---|---|---|---|
| challenging-images (392 symbols) | Fast | 71.2 % | 0 | 2.1 / 25.4 |
| challenging-images | Balanced | 88.8 % | 0 | 9.4 / 90.9 |
| challenging-images | Exhaustive | 87.8 % | 0 | 22.2 / 246.0 |
| synth (228 symbols) | Fast | 84.2 % | 0 | 0.9 / 6.6 |
| synth | Balanced | 97.8 % | 0 | 3.9 / 25.1 |
| synth | Exhaustive | 98.2 % | 0 | 8.1 / 59.9 |

`Exhaustive` is not always better than `Balanced` on real photos: the extra passes cost time and sometimes read different symbols, so measure on your own images.

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
