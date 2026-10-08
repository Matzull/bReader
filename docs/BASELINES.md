# Baselines

First measurement, 2026-10-06. AMD Ryzen 5 5500U, release build, single image at a time. Latency = decode call only (image already loaded as luma8), in ms per image.

Reproduce:

```
tools/fetch-datasets.sh
cargo run -r -p synth -- data/synth
cargo run -r -p bench --features baselines -- data/dynamsoft/challenging-images --engine all
crates/breader-py/.venv/bin/python tools/bench/py_bench.py data/dynamsoft/challenging-images --engine zxingcpp
```

Engines: rxing (`TryHarder`, multi), rqrr (QR only), zxing-cpp 3.1.1 (Python package, default options).

## Dynamsoft `challenging-images`

392 expected symbols in v1 symbologies after GT cleanup (56 Data Matrix / PDF417 excluded). See [Ground-truth cleanup](#ground-truth-cleanup).

| Symbology | Expected | rxing | rqrr | zxing-cpp |
|---|---|---|---|---|
| QR | 234 | 120 | 22 | 220 |
| EAN-13 | 18 | 18 | 0 | 17 |
| EAN-8 | 1 | 1 | 0 | 1 |
| UPC-A | 74 | 5 | 0 | 70 |
| Code 128 | 60 | 23 | 0 | 27 |
| Code 39 | 1 | 1 | 0 | 1 |
| ITF | 4 | 2 | 0 | 4 |
| **Read rate** | 392 | 43.4 % | 5.6 % | 86.7 % |
| **False positives** | | 12 | 0 | 2 |
| Latency p50 / p95 | | 105.2 / 2149.0 | 20.5 / 254.3 | 22.1 / 329.0 |

### Ground-truth cleanup

The bench and `py_bench.py` clean the Dynamsoft annotations on load (`--no-overrides` disables only the overrides, not the deduplication):

- Deduplication: within an image, two GT entries are the same symbol when their normalized text matches (UPC-A is folded into EAN-13 with a leading 0) and their quads overlap (centroid of one inside the other's quad, or bbox IoU > 0.5). One entry is kept, the UPC-A one for a UPC-A / EAN-13 pair. This removed 66 entries from `challenging-images` (all in `batch-scanning-1.jpg`, which labeled each UPC-A symbol twice) and none from `synth`. The one same-text pair in `multiple-symbologies-multiple-barcodes-1.jpeg` is two separate symbols at different positions, so it is kept.
- Overrides: `tools/bench/gt-overrides.json`, keyed by dataset directory name and image file, each entry with `from`, `to` and `reason`. One entry: `barcode-with-shadow-1.jpg` is labeled `268290760343`, but the digits printed under the symbol read `888290760343` (verified visually; both strings have check digit 3, so the checksum alone cannot decide).

Totals before this cleanup (same engines, first measurement): 458 expected symbols (EAN-13 84); read rate rxing 37.3 %, rqrr 4.8 %, zxing-cpp 74.2 %. The old rates were deflated by the duplicate labels, so they are not comparable with the current ones. The synthetic dataset is unaffected.

## Synthetic (`tools/synth`, seed 1, 20 images per class)

228 expected symbols; the `negative` class has none.

| Class | rxing | rqrr | zxing-cpp |
|---|---|---|---|
| clean | 100 % | 30 % | 100 % |
| rotated | 55 % | 30 % | 55 % |
| perspective | 80 % | 35 % | 80 % |
| blur | 85 % | 30 % | 90 % |
| low-res | 100 % | 5 % | 100 % |
| illumination | 75 % | 20 % | 75 % |
| inverted | 0 % | 0 % | 35 % |
| noise | 100 % | 30 % | 100 % |
| multi | 88.2 % | 8.8 % | 88.2 % |
| **Total** | 78.5 % | 18.4 % | 82.0 % |
| **False positives** | 49 | 0 | 10 |
| Latency p50 / p95 | 56.4 / 391.9 | 20.8 / 38.7 | 13.4 / 36.2 |

No engine produced false positives on the `negative` class. Most false positives are partial ITF reads (12 of 14 digits) and spurious UPC-E/EAN-8 inside other 1D symbols. breader requires agreement between several scanlines, a strict length list for ITF and a check for overlapping conflicts, which are aimed at these cases.

## breader (Balanced, commit after QR optimization, 2026-10-07)

| Dataset | Read rate | QR | False positives | Latency p50 / p95 / max |
|---|---|---|---|---|
| challenging-images | 348/392 = 88.8 % | 225/234 | 0 | 22.8 / 217.7 / 305.5 |
| synth (seed 1) | 223/228 = 97.8 % | 65/65 | 0 | 14.3 / 39.5 / 84.8 |

Timings on this machine vary up to 2x between runs; compare engines in the same session.

## breader after M7 kernels, QR polarity and 1D work (2026-10-07)

| Dataset | Effort | Read rate | FP | Latency p50 / p95 / max |
|---|---|---|---|---|
| challenging-images | Fast | 279/392 = 71.2 % | 0 | 2.1 / 25.4 / 63.7 |
| challenging-images | Balanced | 348/392 = 88.8 % | 0 | 9.4 / 90.9 / 146.1 |
| challenging-images | Exhaustive | 344/392 = 87.8 % | 0 | 22.2 / 246.0 / 350.0 |
| synth | Fast | 192/228 = 84.2 % | 0 | 0.9 / 6.6 / 19.5 |
| synth | Balanced | 223/228 = 97.8 % | 0 | 3.9 / 25.1 / 65.0 |
| synth | Exhaustive | 224/228 = 98.2 % | 0 | 8.1 / 59.9 / 189.0 |

zxing-cpp in the same session on challenging-images: 341/392 = 87.0 %, 1 FP, 23.3 / 330.2 / 400.4 ms.

## v1 targets (confirmed 2026-10-07)

| Metric | Target |
|---|---|
| False positives | 0 on every dataset, including `negative` |
| Read rate, challenging-images | ≥ 86 % (zxing-cpp level) |
| Read rate, synth | ≥ 82 % overall, ≥ zxing-cpp in every class |
| Latency, Balanced | p50 ≤ zxing-cpp on both datasets |
| Latency, Fast, 1080p frame | p50 ≤ 10 ms |

## Data Matrix (2026-10-08)

Release build, same machine. Balanced unless noted; false positives 0 everywhere. zxing-cpp 3.1.1 with default options.

| Set | Expected | breader Fast | breader Balanced | breader Exhaustive | zxing-cpp |
|---|---|---|---|---|---|
| challenging-images | 50 | 40 | 44 | 44 | 44 |
| skewed-datamatrix | 1323 | 714 | 1000 | 1010 | 861 |
| synth seed 1 (all classes) | 58 | | 57 | | 56 |
| synth seed 2024 (all classes) | 137 | | 136 | | 135 |
| video `batch_datamatrix` | 100 | 66 | 87 | | |
| video `batch_inverted_datamatrix` | 102 | 0 | 96 | | |
| video `wide_to_close_datamatrix` | 2 | 2 | 2 | | |

Fast searches dark-on-light symbols only. Latency (challenging-images p50 / p95 ms): Fast 4.5 / 31, Balanced 16.4 / 118, Exhaustive 37.8 / 272; without the Data Matrix reader Fast is 2.2 / 25 and Balanced 9.5 / 88. skewed-datamatrix p50: Fast 32, Balanced 156, Exhaustive 285, zxing-cpp 282.

Ground truth: `skewed-datamatrix` leaves some clearly visible symbols unlabeled (each read is a valid `case-N` payload next to its printed caption, far from every annotation). Ten of them are added through `tools/bench/gt-overrides.json` (an entry without `from` adds a label), each checked on a crop of the image. The `wide_to_close_datamatrix` video label is wrapped in typographic quotes, which the video-bench CSV reader now strips.

Misses on challenging-images (6 of 50): two cans whose symbols sit inside a dark ring that merges with them (`datamatrix-2`), two dot-peened symbols on rough metal (`datamatrix-3`), one blurred 14x14 with clock tracks that do not alternate cleanly (`healthcare-2`), and a symbol of about 2.7 px modules (`tiny-barcodes-4`).
