# Baselines

First measurement, 2026-10-06. AMD Ryzen 5 5500U, release build, single image at a time. Latency = decode call only (image already loaded as luma8), in ms per image.

Reproduce:

```
tools/fetch-datasets.sh
cargo run -r -p synth -- data/synth
cargo run -r -p bench --features baselines -- data/dynamsoft/challenging-images --engine all
crates/breader-py/.venv/bin/python tools/bench/py_bench.py data/dynamsoft/challenging-images --engine zxingcpp
```

Datasets are described in [datasets.md](datasets.md); the gates that use them in [testing.md](testing.md#gates).

Engines: rxing (`TryHarder`, multi), rqrr (QR only), zxing-cpp 3.1.1 (Python package, default options), zbar 0.23 (through pyzbar 0.1.9, default settings; see [zbar](#zbar-2026-10-13)).

## Status 2026-10-13

Measurement-only refresh on main at `c171fe4` (no reader change), on an idle AMD Ryzen 5 5500U (no other process; `uptime`, `free` and `pgrep` checked before each block), release builds, `RAYON_NUM_THREADS=4`, the library single-threaded, decode call only. This section supersedes the numbers of the older sections below (they stay as history, each marked); the next refresh replaces it.

- **Gate.** `BENCH_SHOW_FP=1 tools/check-regression.sh --tier c2` passed with 0 false positives at every effort on every set (40.3 minutes in all). Stage times in seconds: synth seed 1 (Balanced) 5, synth seed 2024 (F / B / E) 8 / 11 / 32, challenging-images 3 / 5 / 10, skewed-datamatrix 6 / 12 / 18, UPS-Synthetic-Labels 8 / 14 / 30, pdf417-small-module-size 1 / 1 / 3, BarBeR full 297 / 553 / 1093, zxing-cpp samples 8 / 7 / 13, videos (tracker, fast, balanced) 281.
- **Baselines.** The Dynamsoft video, skewed-datamatrix, UPS and pdf417-small-module-size baselines, synth seed 2024 (three efforts) and synth seed 1 (Balanced) were re-written with `--write-baseline` on the same build: byte-identical to the committed files (no read count moved up or down), so nothing was rewritten.
- **Engines.** breader columns are the Rust `bench` (the committed baselines); zxing-cpp 3.1.1 and zbar 0.23 (pyzbar) come from `tools/bench/py_bench.py` at default options, with `breader-py` rebuilt from main. Until 2026-10-13 the Python breader gave different counts and false positives from the Rust bench on the same images (challenging-images Balanced 411 reads, 2 FP, against 411 reads, 0 FP in Rust; zxing-samples 662 reads, 19 FP, against 681 and 0; BarBeR subset 802, 0 FP, against 803, 0). The cause was the harness, not the binding (both build the same `ScanOptions`): `py_bench.py` did not offer the alternative forms of a read that the Rust bench offers (the charset-decoded text of the symbol, a Shift JIS reading of non-UTF-8 bytes), so a correct read of a label written in another charset counted as a false positive and a miss; it also converted to gray with PIL's Rec. 601 weights instead of the `image` crate's 2126/7152/722, and did not repair the double-encoded UTF-8 of some labels. Fixed in `py_bench.py` ([FAILURES-BARBER-2D](FAILURES-BARBER-2D.md#0-the-python-bench-disagreement)): challenging-images 412 / 0 FP, zxing-samples 682 / 0 FP, BarBeR subset 807 / 0 FP; the remaining 1 to 4 reads are marginal reads that flip with the JPEG decoder (PIL against the Rust `image` crate: 11 of 1104 subset images read differently). zxing-cpp and zbar columns above were measured before the fix and shift by at most a few reads. rxing was not re-run.

### Read rate and false positives

Reads / expected symbols, false positives (FP). breader F / B / E = Fast / Balanced / Exhaustive. Default options for zxing-cpp and zbar.

| Dataset | breader F | breader B | breader E | zxing-cpp | zbar |
|---|---|---|---|---|---|
| challenging-images | 333 / 448, 0 FP | 411 / 448, 0 FP | 424 / 448, 0 FP | 388 / 448, 1 FP | 160 / 448, 5 FP |
| zxing-cpp samples | 568 / 794, 0 FP | 681 / 794, 0 FP | 689 / 794, 0 FP | 716 / 794, 69 FP | 450 / 794, 74 FP |
| BarBeR 1/8 subset | 688 / 1093, 0 FP | 803 / 1093, 0 FP | 828 / 1093, 0 FP | 765 / 1093, 15 FP | 713 / 1093, 29 FP |
| BarBeR full | 5400 / 8585, 0 FP | 6358 / 8585, 0 FP | 6566 / 8585, 0 FP | 6123 / 8585, 91 FP | not run |
| synth seed 1 | 426 / 487, 0 FP | 482 / 487, 0 FP | 483 / 487, 0 FP | 381 / 487, 11 FP (2026-10-13 earlier) | 201 / 487, 43 FP (same) |
| synth seed 2024 (60 per class) | 1144 / 1311, 0 FP | 1286 / 1311, 0 FP | 1292 / 1311, 0 FP | not run | not run |
| skewed-datamatrix | 729 / 1324, 0 FP | 1012 / 1324, 0 FP | 1022 / 1324, 0 FP | not run | not run |
| UPS-Synthetic-Labels | 84 / 104, 0 FP | 98 / 104, 0 FP | 99 / 104, 0 FP | not run | not run |
| pdf417-small-module-size | 3 / 22, 0 FP | 4 / 22, 0 FP | 7 / 22, 0 FP | not run | not run |

(Synth seed 1 Fast and Exhaustive come from the latency runs below; the committed gate is Balanced only. The synth seed 1 zxing-cpp and zbar cells were not re-measured.) breader has 0 false positives on every set and effort. On the BarBeR full set, zxing-cpp has 91 (EAN-13 76, UPC-E 9, ...) and 5 scan errors.

### Latency

p50 / p95 in ms per image. breader: the median of 5 interleaved runs of the Rust `bench` (rounds over the nine dataset and effort pairs, so drift hits all alike; spread of the p50 across the 5 runs 0.2 ms or less, except challenging-images Balanced 0.8 ms and Exhaustive 1.4 ms and synth Exhaustive 0.3 ms). zxing-cpp and zbar: one run of `py_bench.py` (includes the Python call; zxing-cpp agrees with the earlier measurement within 6 %). The zxing-cpp samples and BarBeR full rows of breader are single runs, from the c2 gate.

| Dataset | breader Fast | breader Balanced | breader Exhaustive | zxing-cpp | zbar |
|---|---|---|---|---|---|
| challenging-images | 5.43 / 36.8 | 20.24 / 130.5 | 51.78 / 362.7 | 22.2 / 322.7 | 46.3 / 583 |
| BarBeR 1/8 subset | 6.04 / 24.9 | 22.39 / 103.9 | 58.54 / 277.6 | 33.0 / 192.3 | 65.9 / 394 |
| synth seed 1 | 1.72 / 8.3 | 6.11 / 40.0 | 13.96 / 100.3 | 13.3 / 41 (earlier) | 16.4 / 88 (earlier) |
| zxing-cpp samples (single run) | 0.87 / 4.1 | 4.09 / 23.8 | 8.45 / 50.9 | 1.39 / 8.0 | 1.87 / 12.2 |
| BarBeR full (single run) | 6.29 / 25.8 | 23.43 / 110.5 | 59.42 / 287.3 | 32.7 / 198.3 | not run |

Balanced is below zxing-cpp's p50 on the three photo sets and above it on the zxing-cpp samples (small, clean images, where zxing-cpp is fastest of the three). Against Phase D (2026-10-12) the Fast p50 is unchanged within 1 % on challenging-images (5.43 against 5.39), BarBeR subset (6.04 against 6.00) and synth (1.72 against 1.75); Balanced moved +3.5 % (20.24 against 19.55), +2.7 % (22.39 against 21.80) and +0.5 % (6.11 against 6.08).

### Fast at 1080p (video-bench)

`video-bench data/dynamsoft/video-based-testing --engines fast,balanced --filter 1080p`, 3 rounds, per-frame p50 in ms, median over the rounds (the method of [Phase D latency](#phase-d-latency-2026-10-12)). The target is 10 ms at Fast.

| Video | Fast | Balanced |
|---|---|---|
| batch_EAN13 | 5.5 | 17.9 |
| batch_code39 | **17.0** | 81.5 |
| batch_dark_environment_1d | 5.5 | 14.0 |
| batch_datamatrix | **11.4** | 63.2 |
| batch_inverted_datamatrix | 9.5 | 49.6 |
| batch_rotated_small_pdf417 | **11.5** | 50.4 |
| damaged_qrcode | 9.7 | 21.2 |
| reflective_surface_qrcode | 5.8 | 18.2 |
| rotated_vertical_ITF | 5.6 | 15.4 |
| wide_to_close_datamatrix | 4.3 | 16.4 |

**After the Fast latency work ([D-044](decisions.md#d-044)), same method, interleaved old and new binaries, median of 3 rounds** (the machine was shared with another benchmark at the time, so the absolute values may be 2 to 3 % high; old = the table above re-measured):

| Video | Fast before | Fast after |
|---|---|---|
| batch_EAN13 | 6.1 | 5.5 |
| batch_code39 | 17.5 | **12.4** |
| batch_dark_environment_1d | 6.0 | 5.6 |
| batch_datamatrix | 11.8 | **11.4** |
| batch_inverted_datamatrix | 9.9 | 9.6 |
| batch_rotated_small_pdf417 | 11.6 | **11.2** |
| damaged_qrcode | 10.1 | 8.5 |
| reflective_surface_qrcode | 6.5 | 6.0 |
| rotated_vertical_ITF | 6.4 | 5.9 |
| wide_to_close_datamatrix | 5.2 | 4.6 |

Fast on the still-image sets (p50 / p95, old to new): challenging-images 5.44 / 36.8 to 4.79 / 35.2, BarBeR 1/8 subset 6.09 / 24.9 to 5.35 / 21.8, synth seed 1 1.76 / 8.3 to 1.44 / 8.3. Fast reads are unchanged everywhere except skewed-datamatrix Data Matrix 724 to 729 (baseline rewritten); 0 false positives. Three videos are still over 10 ms.

Three of ten videos were over 10 ms before this work (`batch_code39`, `batch_rotated_small_pdf417`, `batch_datamatrix`), the same three as in Phase D, with `batch_code39` 1.1 ms higher than then (15.9 -> 17.0) and the other two about equal (11.3 -> 11.5, 10.9 -> 11.4); `damaged_qrcode` (9.7) and `batch_inverted_datamatrix` (9.5) are within 5 % of the target. Median video: 7.65 ms at Fast, 19.7 ms at Balanced. The tracker gate (`video-bench --check`, c2) passes with 0 false positives; all engines in total: tracker 256 / 277 reads (mean 48.8 ms), balanced 246 / 277, fast 102 / 277.

### Per-class comparison at Balanced

Reads, with false positives after the slash; the gap is breader minus zxing-cpp (reads). Only classes with a gap are listed; BarBeR full is the largest sample and ranks first.

BarBeR full (8585 symbols):

| Class | Expected | breader B | zxing-cpp | Gap |
|---|---|---|---|---|
| QR Code | 1329 | 1094 / 0 | 1153 / 0 | -59 |
| Code 39 | 198 | 130 / 0 | 151 / 1 | -21 |
| ITF | 72 | 48 / 0 | 59 / 1 | -11 |
| PDF417 | 67 | 26 / 0 | 36 / 1 | -10 |
| Data Matrix | 110 | 95 / 0 | 99 / 0 | -4 |
| EAN-8 | 70 | 67 / 0 | 64 / 1 | +3 |
| Aztec | 14 | 14 / 0 | 11 / 0 | +3 |
| UPC-A | 327 | 272 / 0 | 259 / 0 | +13 |
| Code 128 | 1428 | 597 / 0 | 450 / 2 | +147 |
| EAN-13 | 4970 | 4015 / 0 | 3841 / 76 | +174 |

BarBeR 1/8 subset: QR Code 128 against 132 (-4), Code 39 12 against 18 (-6), ITF 11 against 13 (-2); leads on EAN-13 524 / 0 against 496 / 13 (+28), Code 128 67 against 51 (+16), UPC-A 39 against 35 (+4), EAN-8 +1, PDF417 +1.

zxing-cpp samples (794 symbols; here the gaps are largest, mostly in classes with few BarBeR labels): ITF 7 against 20 (-13; the default ITF length list, [D-008](decisions.md#d-008)), GS1 DataBar 10 against 17 (-7), Data Matrix 87 against 93 (-6), EAN-13 39 against 45 (-6), Codabar 12 against 15 (-3), PDF417 56 against 58 (-2), and -1 each for Code 128, Code 39, EAN-8 and Micro QR; leads on QR Code 128 against 125 (+3) and UPC-A 53 against 50 (+3); ties Aztec, GS1 DataBar Expanded and Limited, UPC-E, rMQR. zxing-cpp reads the stacked Expanded labels, which are unsupported, so its 64 Expanded false positives are not read errors.

challenging-images (448): Code 128 41 against 28 (+13), QR Code 229 / 0 against 220 / 1 (+9), Data Matrix 45 against 44 (+1), UPC-A 71 against 70 (+1); ITF 3 against 4 (-1); ties EAN-13 (17), EAN-8, Code 39 (1) and PDF417 (3).

Ranked gaps where breader trails zxing-cpp (reads, Balanced): QR Code on BarBeR (-59 of 1329, mostly Dubska and Muenster), Code 39 on BarBeR (-21), ITF on zxing-samples (-13), ITF on BarBeR (-11), PDF417 on BarBeR (-10), GS1 DataBar on zxing-samples (-7), Data Matrix (-6 on zxing-samples, -4 on BarBeR), EAN-13 on zxing-samples (-6), Code 39 on the BarBeR subset (-6). The v1 target "read rate >= zxing-cpp per class" is therefore still not met on real photos for QR Code, Code 39, ITF, PDF417 and Data Matrix; breader leads on Code 128, EAN-13, UPC-A and Aztec on BarBeR and on Code 128 and QR Code on challenging-images, with 0 false positives against zxing-cpp's 91 on BarBeR.

## Dynamsoft `challenging-images`

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

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

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

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

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

| Dataset | Read rate | QR | False positives | Latency p50 / p95 / max |
|---|---|---|---|---|
| challenging-images | 348/392 = 88.8 % | 225/234 | 0 | 22.8 / 217.7 / 305.5 |
| synth (seed 1) | 223/228 = 97.8 % | 65/65 | 0 | 14.3 / 39.5 / 84.8 |

Timings on this machine vary up to 2x between runs; compare engines in the same session.

## breader after M7 kernels, QR polarity and 1D work (2026-10-07)

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

| Dataset | Effort | Read rate | FP | Latency p50 / p95 / max |
|---|---|---|---|---|
| challenging-images | Fast | 279/392 = 71.2 % | 0 | 2.1 / 25.4 / 63.7 |
| challenging-images | Balanced | 348/392 = 88.8 % | 0 | 9.4 / 90.9 / 146.1 |
| challenging-images | Exhaustive | 344/392 = 87.8 % | 0 | 22.2 / 246.0 / 350.0 |
| synth | Fast | 192/228 = 84.2 % | 0 | 0.9 / 6.6 / 19.5 |
| synth | Balanced | 223/228 = 97.8 % | 0 | 3.9 / 25.1 / 65.0 |
| synth | Exhaustive | 224/228 = 98.2 % | 0 | 8.1 / 59.9 / 189.0 |

zxing-cpp in the same session on challenging-images: 341/392 = 87.0 %, 1 FP, 23.3 / 330.2 / 400.4 ms.

## Linear reader fixes (2026-10-08)

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

Four changes after the failure triage in [FAILURES.md](FAILURES.md): Code 39 quiet zone of 4 modules (symbols below 7 modules and without a check character need one more agreeing line), geometric grouping of equal-text hits in the aggregator, a weak Code 128 start character, and no change for ITF (default length list). Read counts, false positives 0 everywhere; latency not re-measured.

| Set | Fast | Balanced | Exhaustive |
|---|---|---|---|
| challenging-images, all symbologies (442) | 319 -> 322 | 392 -> 395 | 388 -> 406 |
| challenging-images, Code 128 (60) | 26 -> 29 | 34 -> 37 | 29 -> 47 |
| zxing-samples, Code 39 (14) | 5 -> 7 | 8 -> 9 | 8 -> 9 |
| video `batch_code39` (32) | 9 -> 25 | 11 -> 28 | not run (tracker 13 -> 30) |
| video `batch_EAN13` (28) | 3 -> 3 | 21 -> 22 | not run (tracker 21) |

Synthetic sets and skewed-datamatrix are unchanged.

## Edge relocation and border quiet zone (2026-10-08)

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

Two linear-reader changes after the zxing-cpp samples triage ([FAILURES-ZXING-SAMPLES.md](FAILURES-ZXING-SAMPLES.md)): the half-level edge reference is blended with the extremes of a window scaled by the local run width, and a quiet zone that ends at the profile border is open for EAN/UPC, Code 128 and Code 39 (not ITF). Read counts, 0 false positives everywhere:

| Set | Fast | Balanced | Exhaustive |
|---|---|---|---|
| zxing-samples (470) | 245 -> 248 -> 256 | 292 -> 313 -> 321 | 297 -> 319 -> 327 |
| challenging-images (442) | 322 -> 322 -> 322 | 395 -> 400 -> 400 | 406 -> 413 -> 413 |
| synth seed 1 (286) | 242 -> 244 -> 244 | 280 -> 283 -> 283 | 281 -> 284 -> 284 |
| synth seed 2024 (831) | 718 -> 725 -> 725 | 808 -> 816 -> 816 | 811 -> 819 -> 819 |
| skewed-datamatrix (1323) | 714 | 1000 | 1010 |
| video (277 symbols): fast / balanced / tracker | 100 -> 101 | 240 -> 244 | 246 -> 253 |

Each cell is before -> after edge relocation -> after both changes (video: before -> after both). The window levels of the edge relocation may move a reference level at most 0.25 of the local level difference; without that limit the plain blend gained a little more in total but lost the `upca-3` group of zxing-samples, synth-2024 `illumination` ITF and the video `EAN_13-distance` (see FAILURES-ZXING-SAMPLES.md). The latency of the decode call was compared with interleaved runs of the base, edge-relocation-only and final builds (3 rounds, median p50): challenging-images Balanced 16.85 / 16.89 / 17.00 ms, Exhaustive 38.7 / 40.4 / 40.6 ms, synth-2024 Balanced 5.38 / 5.44 / 5.44 ms, zxing-samples Exhaustive 4.56 / 5.04 / 5.11 ms (more symbols read, so more work per image).

## v1 targets (confirmed 2026-10-07)

| Metric | Target |
|---|---|
| False positives | 0 on every dataset, including `negative` |
| Read rate, challenging-images | ≥ 86 % (zxing-cpp level) |
| Read rate, synth | ≥ 82 % overall, ≥ zxing-cpp in every class |
| Latency, Balanced | p50 ≤ zxing-cpp on both datasets |
| Latency, Fast, 1080p frame | p50 ≤ 10 ms |

## Data Matrix (2026-10-08)

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

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

## zxing-cpp samples (2026-10-08)

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

The `test/samples` images and expected-result files of zxing-cpp, tag `v3.1.1`, fetched by `tools/fetch-datasets.sh zxing-samples` (shallow sparse checkout into `data/zxing-samples`). Only the images and the expected values are used, as test data. The binary `zxing-samples-gt` (`tools/bench/src/bin/zxing_samples_gt.rs`) writes an `annotations.json` into each sample directory:

- The directory prefix gives the symbology (`qrcode-2` is QR Code); `falsepositives-*` images get an empty label list, so any read there is a false positive. A `format=` line in `<stem>.result.txt` (Code 32, Pharmazentralnummer) overrides the directory's symbology.
- The expected value is `<stem>.txt` or `<stem>.bin`; non-UTF-8 values are stored as hex (`bytes_hex`) and compared as bytes. The other `.result.txt` keys (symbology identifier, EC level, ...) are not used.
- In `*-extension-*` directories the add-on digits that zxing-cpp appends to the text are dropped: the default reader ignores add-ons (`EanAddonPolicy::Ignore`), so the labeled symbol is the main code.
- Left out, with a note printed by the converter: 9 images with no expected file (`multi-1`, `maxicode-2`; their expectations are hard-coded in zxing-cpp's test code and are not invented here) and 8 parts of structured-append symbols (`qrcode-7/01-0N`, `pdf417-4`), whose only expected text is the merged one. No image failed to load (WebP is decoded by the `image` crate's `webp` feature).

62 directories, 932 images: 46 negative images (`falsepositives-*`) and 886 labeled images with one label each, plus 3 labels added by overrides. Of the 889 labels, 470 are in the nine v1 symbologies and 419 are unsupported (Aztec, PDF417, Micro PDF417, MaxiCode, Micro QR, rMQR, DataBar family, Codabar, Code 93, Telepen, DX film edge, Code 32, PZN) and excluded from the read rate.

Matching changes made for this dataset (all general, with unit tests): UPC-E is expanded to UPC-A and UPC-A / expanded UPC-E in 13-digit EAN notation are folded into EAN-13 like 12-digit UPC-A; a read also matches a label that holds its charset-decoded text (the reader's `text`, or its Shift JIS reading when the bytes are not UTF-8), because zxing-cpp labels hold UTF-8 text. The Dynamsoft results are unchanged.

Read count / false positives per symbology (release build, no GT overrides besides the three below):

| Symbology | Expected | breader Fast | breader Balanced | breader Exhaustive | rxing | zxing-cpp 3.1.1 |
|---|---|---|---|---|---|---|
| QR Code | 138 | 107 / 0 | 125 / 0 | 125 / 0 | 119 / 0 | 125 / 1 |
| EAN-13 | 65 | 24 / 0 | 39 / 0 | 40 / 0 | 43 / 4 | 45 / 4 |
| EAN-8 | 9 | 7 / 0 | 8 / 0 | 8 / 0 | 8 / 6 | 9 / 0 |
| UPC-A | 76 | 21 / 0 | 53 / 0 | 56 / 0 | 50 / 0 | 50 / 0 |
| UPC-E | 29 | 14 / 0 | 20 / 0 | 22 / 0 | 18 / 17 | 20 / 0 |
| Code 128 | 22 | 15 / 0 | 21 / 0 | 21 / 0 | 21 / 0 | 22 / 0 |
| Code 39 | 14 | 7 / 0 | 11 / 0 | 11 / 0 | 11 / 3 | 12 / 0 |
| ITF | 20 | 1 / 0 | 2 / 0 | 2 / 0 | 17 / 0 | 20 / 0 |
| Data Matrix | 97 | 79 / 0 | 86 / 0 | 89 / 0 | 78 / 2 | 93 / 0 |
| **Total** | 470 | 256 / 0 | 348 / 0 | 355 / 0 | 365 / 32 | 396 / 5 |

Cells are reads / false positives. Read rate: breader 54.5 % / 74.0 % / 75.5 %, rxing 77.7 %, zxing-cpp 84.3 %. breader has zero false positives at every effort. The other engines' false positives were not triaged. rxing (`TryHarder`, multi) panics on one image (`aztec-1/gs1-figure-4.15.1-2-31x31.png`) and loops on another (`itf-1/#220.png`, abandoned after 30 s); both count as no read. zxing-cpp is the Python package with default options (a UPC-A read can come back as EAN-13). Counts include the linear fixes, the up-scale retry and the edge/border-quiet-zone changes (2026-10-09). A Code 39 read also matches its Full ASCII expansion (`code39-2` labels the expanded text while breader defaults to plain Code 39).

Balanced and Exhaustive include the up-scale retry (3x Catmull-Rom copy of frames up to 600 px that yielded nothing); with the linear fixes and before it they read 292 and 297 (QR 102/102, UPC-A 39/40, UPC-E 16/18, Code 128 14/14, ITF 1/1, Data Matrix 72/74, the rest unchanged). The retry also adds one QR read on challenging-images (225 -> 226 of 234 QR at Balanced and Exhaustive) and nothing on synth. Fast has the retry off; enabled it would read 308 here and nothing more on challenging-images. Latency on this set, p50 / p95 ms, same session, interleaved runs: Balanced 1.8 / 7.3 before, 4.7 / 21.0 after; Exhaustive 4.5 / 15.8 before, 11.7 / 44.5 after (most of the 932 images are small, and the negative and unsupported-symbology ones all take the retry); challenging-images Balanced 16.9 / 119 before, 17.0 / 119 after, Exhaustive 38.0 / 277 before, 41.0 / 338 after. A blank 600x600 frame costs +15 ms at Balanced and +26 ms at Exhaustive, a noise-filled one +142 and +340 ms.

Latency: to be measured on an idle machine.

Sampling retries (2026-10-09, `breader-qr`, `breader-datamatrix`; zxing-samples reads Fast / Balanced / Exhaustive, 0 false positives at every effort and dataset): QR 95 / 115 / 115 -> 107 / 125 / 125 (pitch correction from the finder and timing patterns, local-threshold sampling from the luma image, nested symbols) and Data Matrix 72 / 79 / 80 -> 79 / 86 / 89 (pitch correction from the clock tracks, pad check waived for uncorrected large blocks). The retries run only after Reed-Solomon failed on a plausible grid; interleaved before / after runs show no latency change (zxing-samples Exhaustive p50 12.7 -> 12.3 ms, challenging-images Balanced 17.1 -> 16.7 ms, Fast 4.7 -> 4.6 ms, synth Balanced 5.6 -> 5.5 ms). challenging-images: QR 186 / 226 / 226 -> 191 / 229 / 229 (the 1862-character v40 symbol of `custom-scan-parameters-4` and `A-0040-Z` now decode), Data Matrix 40 / 44 / 44 -> 40 / 45 / 45. skewed-datamatrix: 714 / 1000 / 1010 -> 724 / 1012 / 1022 of 1324. The video baselines gain 1 to 3 Data Matrix reads in `batch_datamatrix_1080p`; synth is unchanged.

Bench matching and labels changed with this: a label that is UTF-8 read as Latin-1 and encoded again (`way` + `\u00e2\u0080\u0094` + `in` for an em dash, the `custom-scan-parameters-4` text) is compared in repaired form (a normalization, not a misdecode: the decoded text is the correct one); and `skewed-datamatrix/IMG20260320082633.jpg` gains the unlabeled but clearly visible, Reed-Solomon-valid `case-138` through `gt-overrides.json`.

Ground truth additions (`tools/bench/gt-overrides.json`, each checked on the image):

- `qrcode-2/16.webp`: a nested double QR; only the inner symbol is labeled, the outer one is a second readable symbol (both are read since the nested-symbol rule).
- `code39-1/code32-1.png`, `code39-1/pzn-1.png`: Code 32 and PZN are carried in plain Code 39 symbols and labeled with their own (unsupported) symbology; the Code 39 content of the same symbol (`3PRM8N`, the base-32 form of the Code 32 value A123456788, and `-12345678`) is added.

Where breader reads less than zxing-cpp:

- ITF: by design, the default accepts 14 digits only (`ItfOptions::allowed_lengths`); 17 of the 20 labels are 4 to 10 digits long (the 3 of 14 digits are read in one case).
- EAN/UPC (`ean13-4`, `upca-2`, `upca-4`, `upce-3`) and QR Code (`qrcode-4`): not investigated; Fast misses most of them (fewer passes).
- Code 128 and Code 39 (`code128-3`, `code39-2`, `code128-1`): not investigated.
- Data Matrix: `datamatrix-5` (0 of 5) and a few in `datamatrix-3`.
- A kana-only Shift JIS QR payload (`qrcode-2`) is reported with `text` as Latin-1, since the heuristic requires a double-byte character; `data` is the exact payload.

## BarBeR (2026-10-11)

First measurement on [BarBeR](datasets.md#barber): 8748 real photos of 12 source datasets, 8585 supported expected symbols (227 more labels are postal codes, IATA 2 of 5 and EAN-2, excluded; about 1000 regions are undecodable or of unknown value and are not expected reads). Internal use only (no license). Loader rules, ground-truth findings and overrides: [datasets](datasets.md#barber); the gate: [testing](testing.md#the-barber-gate). Measured on the tree with the Aztec, Codabar, Micro QR, rMQR and DataBar readers (none of the last four has a BarBeR label). Release build, zxing-cpp 3.1.1 Python with default options, rxing with `--features baselines` (`TryHarder`, multi). Read count / false positives. The tables below are refreshed on 2026-10-13 (breader and zxing-cpp; see [Status 2026-10-13](#status-2026-10-13)); the prose of the triage is from 2026-10-11.

| Symbology | Expected | breader Fast | breader Balanced | breader Exhaustive | rxing | zxing-cpp |
|---|---|---|---|---|---|---|
| EAN-13 | 4970 | 3462 / 0 | 4015 / 0 | 4121 / 0 | 3487 / 77 | 3841 / 76 |
| UPC-A | 327 | 227 / 0 | 272 / 0 | 280 / 0 | 237 / 14 | 259 / 0 |
| EAN-8 | 70 | 63 / 0 | 67 / 0 | 69 / 0 | 60 / 126 | 64 / 1 |
| Code 128 | 1428 | 425 / 0 | 597 / 0 | 671 / 0 | 431 / 6 | 450 / 2 |
| Code 39 | 198 | 73 / 0 | 130 / 0 | 138 / 0 | 114 / 3 | 151 / 1 |
| ITF | 72 | 46 / 0 | 48 / 0 | 52 / 0 | 50 / 6 | 59 / 1 |
| QR Code | 1329 | 975 / 0 | 1094 / 0 | 1101 / 0 | 1030 / 3 | 1153 / 0 |
| Data Matrix | 110 | 93 / 0 | 95 / 0 | 96 / 0 | 67 / 0 | 99 / 0 |
| PDF417 | 67 | 25 / 0 | 26 / 0 | 24 / 0 | 0 / 0 | 36 / 1 |
| Aztec | 14 | 11 / 0 | 14 / 0 | 14 / 0 | 0 / 0 | 11 / 0 |
| **Total** | 8585 | 5400 / 0 (62.9 %) | 6358 / 0 (74.1 %) | 6566 / 0 (76.5 %) | 5476 / 828 (63.8 %) | 6123 / 91 (71.3 %) |

Latency per image, p50 / p95 ms (2026-10-13, idle machine, single runs of the c2 gate and of `py_bench.py`): breader Fast 6.3 / 25.8, Balanced 23.4 / 110.5, Exhaustive 59.4 / 287.3; zxing-cpp 32.7 / 198.3; rxing 157 / 1033 (it is run on a worker thread with a 30 s timeout and took 4.6 hours for the set; measured on the shared machine of 2026-10-11, not re-run). Scan errors: breader 0, rxing 0, zxing-cpp 5 (images it fails on). zxing-cpp also reports 9 UPC-E reads for labels that are never UPC-E (counted in its false positives; no label is UPC-E). **The breader and zxing-cpp columns were re-measured on 2026-10-13** (breader at `c171fe4`: the ITF, QR and Code 39 rows rose since 2026-10-11, e.g. Balanced 6187 -> 6358 reads, QR Code 945 -> 1094, ITF 34 -> 48); **the rxing column predates the last label overrides** (it was run with 8585 expected symbols but before the overrides for `Special_0329`, `8886467105357` and `Special_0177` and before the Aztec and newline rules; it has no Aztec reader in any case) and was not re-run.

Read rate (all symbologies) per source dataset, read rate / false positives, Balanced latency p50 / p95 ms:

| Source | Images | Expected | Fast | Balanced | Exhaustive | rxing | zxing-cpp | Balanced ms |
|---|---|---|---|---|---|---|---|---|
| Artelab | 430 | 436 | 50.9 % / 0 | 68.3 % / 0 | 72.9 % / 0 | 48.4 % / 20 | 53.0 % / 6 | 21.9 / 63 |
| Artelab Extended | 155 | 162 | 63.6 % / 0 | 75.3 % / 0 | 76.5 % / 0 | 64.2 % / 8 | 63.6 % / 0 | 7.0 / 16 |
| Bodnar | 98 | 94 | 88.3 % / 0 | 94.7 % / 0 | 94.7 % / 0 | 89.4 % / 1 | 91.5 % / 0 | 18.0 / 32 |
| Deal Kaist | 3308 | 2895 | 69.1 % / 0 | 83.6 % / 0 | 85.6 % / 0 | 72.1 % / 385 | 78.2 % / 38 | 19.4 / 108 |
| Dubska | 810 | 802 | 70.0 % / 0 | 78.6 % / 0 | 79.1 % / 0 | 77.4 % / 77 | 86.9 % / 0 | 20.0 / 53 |
| InventBar | 527 | 536 | 61.2 % / 0 | 78.4 % / 0 | 81.9 % / 0 | 69.6 % / 19 | 82.6 % / 0 | 7.5 / 14 |
| Muenster | 1055 | 1064 | 79.4 % / 0 | 80.6 % / 0 | 82.0 % / 0 | 79.0 % / 95 | 88.0 % / 27 | 45.1 / 106 |
| Open Food Facts | 185 | 149 | 69.1 % / 0 | 76.5 % / 0 | 80.5 % / 0 | 65.8 % / 29 | 73.8 % / 3 | 26.0 / 137 |
| ParcelBar | 844 | 1136 | 21.0 % / 0 | 32.2 % / 0 | 37.3 % / 0 | 21.3 % / 59 | 20.4 % / 2 | 21.1 / 29 |
| SKUU DSP Lab | 325 | 345 | 56.5 % / 0 | 68.7 % / 0 | 71.6 % / 0 | 56.5 % / 28 | 62.9 % / 9 | 34.7 / 54 |
| Szentandrasi-Dubska QR | 90 | 227 | 82.4 % / 0 | 86.3 % / 0 | 87.7 % / 0 | 81.5 % / 6 | 86.3 % / 0 | 88.6 / 145 |
| ZVZ-real | 921 | 739 | 72.4 % / 0 | 82.1 % / 0 | 83.9 % / 0 | 58.9 % / 101 | 82.4 % / 6 | 54.2 / 202 |

(Dubska counts 23 images shared with Szentandrasi-Dubska QR, which therefore has 90 instead of 115. The rxing column is from 2026-10-11 and was not re-run.) breader trails zxing-cpp on QR Code (1094 against 1153 at Balanced; the gap is now concentrated in Dubska, 78.6 % against 86.9 %, and Muenster, 80.6 % against 88.0 %), Code 39, ITF and PDF417, leads on Aztec, and is ahead on Code 128, EAN-13, UPC-A and EAN-8 from Balanced on (at Fast it trails zxing-cpp on EAN-13, UPC-A and Code 128). By source, at Balanced it leads zxing-cpp in 7 of 12 sources, ties Szentandrasi-Dubska QR (86.3 %) and trails in Dubska, InventBar, Muenster and ZVZ-real (by 0.3 points there). ParcelBar (Code 128 on postal parcels) is hard for all engines. At Balanced breader decodes a value on about 300 regions the dataset calls undecodable (masked reads, not false positives).

### False-positive triage (breader)

Zero false positives at every effort, full set and subset. The first Balanced run had 42 false positives, all ground-truth or normalization problems; the Aztec reader added one more of the same kind, and the Exhaustive-only UPC-E misread of `EAN13_09_0093.jpg` (a clean EAN-13 `5909990748518` read as UPC-E `0190000009995`) was fixed in the reader on main before the final measurement. Nothing was changed in the readers for this triage.

(a) Ground truth, fixed in the loader or by an override (each checked on the image):

- ZVZ-real payload encoding (23: 22 PDF417, among them `Special_0008`, and 1 Data Matrix): binary and 8-bit payloads of PDF417 / Data Matrix are labelled as IBM437 text or as hex of UTF-16 units, with `^` for both NUL and caret. Fixed in the loader (`legacy_payload`); ambiguous (`^`) payloads are treated as unknown (32 regions).
- Line endings (`Special_0001`, 3 x `Special_0253`, `Special_0036`): QR payloads with CR or CR LF, labelled with LF. Fixed in `matching::canonical` (general rule, unit test).
- Label typos and wrong labels (overrides): `20140607_203354` (12 digits, invalid check digit; the symbol is 8809073220054), `20140607_203927` (4971381329321 for 4971381320321), `[P]ISBN_18_0067` (...824 for ...827), `[S]ISBN_02_0074` (truncated, 9785699632657), `Special_0331` (garbled Code 128 for the printed AU170360RZTCASA221C03Z0), `IMG_20120226_165417` and `IMG_3029` (`http//` for `http://`), `PostBarcode362_0` and `PostBarcode831` (literal `\000001` prefix), `QR-photo_26_0021` (the label is the URL of another plant), `Special_0329` (a 13-bar Code 128 holding `1`, labelled `Q1` as an EAN-2), `Special_0177` (the Aztec of a Trenitalia ticket is labelled with the printed ticket number; its real payload is binary and unknown, so the region is masked as undecodable).
- Unannotated symbols (overrides): `4305399070402-01_N95` (no annotation at all), `20140607_203543` (a QR next to the labelled EAN-13), `8886467105357` (the EAN-13 carries only an undecodable polygon outside the image).

(b) Genuine breader misreads: none left at this commit.

Other engines (summary): rxing 828 false positives (EAN-8 126, EAN-13 77, QR 3, ...; mostly wrong digits of real symbols, e.g. 385 in Deal Kaist), zxing-cpp 95 (EAN-13 76, UPC-E 9, ...): mostly EAN-13 values with a few wrong digits read from a real symbol; some are the same label problems (for example 3 `jdi.na.vutbr.cz` QR in `IMG_20120226_1644xx`, which the dataset does not annotate). They were not triaged individually.

### Gate

`tools/check-regression.sh --barber` (1/8 subset, about 1100 images: fast 51 s, balanced 67 s, exhaustive 133 s, 4.2 minutes in total on the shared machine) and `--barber-full` / `--all` (fast 297 s, balanced 549 s, exhaustive 1071 s). Baselines `barber-subset-*.json` and `barber-*.json`, written from real runs with 0 false positives: groups are the 12 sources, items the symbologies. The full gate is over the 3 minute budget per effort (Balanced and Exhaustive), so the quick check is the subset; the trade-off is a 1/8 sample (about 12 images from the smallest source) against exact coverage, and a regression below about 8 reads of one source and symbology can hide in the sample. Run tier b before merging a reader change and `--tier c1` (the full set at Balanced and Exhaustive) only for changes that raise false-positive exposure; the list is in [testing](testing.md#when-to-run-c1).

## Dynamsoft `UPS-Synthetic-Labels` and `pdf417-small-module-size` (2026-10-09)

Two more Dynamsoft sets, gated by `dynamsoft-ups-*.json` and `dynamsoft-pdf417-*.json` (`tools/check-regression.sh --dynamsoft`). Their ground truth is already in the `barcode-benchmark/1.0` format but lives in `annotation/annotations_min.json` and `annotation/test-results.json`, with image names the bench resolves by stem (UPS names have no extension; PDF417 names end in `_annotated` while the files are `Images/1-1-4.jpeg`). Labels whose image is missing are skipped with a notice. All coordinates in both files are zero (no quads).

- `UPS-Synthetic-Labels` (CC BY 4.0, "UPS Synthetic Shipping Label Dataset, Dynamsoft, 2025."): 58 annotated images, but only 52 are in the checkout (`IMG_8757`, `IMG_9565`, `IMG_9570`, `IMG_9577`, `IMG_9579`, `IMG_9582` are labelled and absent). Per image two Code 128 (tracking `1Z...`, reference `CA 859 2-NN`) and one MaxiCode: 104 Code 128 labels supported, 52 MaxiCode unsupported.
- `pdf417-small-module-size`: 11 images, 15 PDF417 labels (all unsupported). The dataset has no Code 128 labels; 7 are added through `gt-overrides.json` (below), so 7 supported labels.
- `challenging-images` carries 6 PDF417 labels, already counted as unsupported (excluded) there.
- Video: `batch_rotated_small_pdf417_1080p` is in the video baseline, but its CSV holds only the header line (no PDF417 values were published), so it has 0 expected labels. Neither breader nor zxing-cpp reads a PDF417 in its 404 frames (see [PDF417 reader](#pdf417-reader-2026-10-09)), so no label could be verified and the gate has no false positive.

MaxiCode maps to no `Symbology`, so its labels are excluded; PDF417 labels count since the reader landed. `map_symbology` (`tools/bench/src/groundtruth.rs`) is the only mapping, shared by the bench and video-bench.

Read count / false positives, release build (zxing-cpp 3.1.1 Python with default options; rxing with `--features baselines`, default options):

| Set / symbology | Expected | breader Fast | breader Balanced | breader Exhaustive | rxing | zxing-cpp |
|---|---|---|---|---|---|---|
| UPS, Code 128 | 104 | 84 / 0 | 98 / 0 | 99 / 0 | 88 / 0 (+4 UPC-E FP) | 91 / 0 |
| pdf417-small, Code 128 | 7 | 2 / 0 | 3 / 0 | 6 / 0 | 5 / 4 (+2 EAN-8 / UPC-E FP) | 7 / 0 |

Triage (BENCH_SHOW_FP=1, all efforts):

- UPS: no false positives; the misses are real (5 to 20 of 104, e.g. `CA 859 2-41` in `IMG_9592` / `IMG_9593` and `CA 859 2-34` in `IMG_9576` at every effort), not investigated.
- pdf417-small: the first run had 3 / 2 / 6 Code 128 false positives (Fast / Balanced / Exhaustive), all `Q8` or `NUTWR`. These are genuine, unlabelled Code 128 symbols of the AIAG shipping labels (the dataset only labels the PDF417): the QTY(Q) barcode next to the printed quantity `8` (Q is the data identifier) and the SUPP(V) barcode of the supplier whose code is printed large as `NUTWR`. Checked on crops, and zxing-cpp reads exactly the same seven (`Q8` in 10-9-11, 16-6-15 x2, 4-3-11, 6-5-14; `NUTWR` in 16-6-15 x2). Added to `gt-overrides.json` with `count` (a new field for several identical symbols in one image). The labels carry other Code 128 symbols (part number `ML35-7B6xx-GB`, serial) that breader and zxing-cpp do not read; they stay unlabelled. rxing reads them, wrongly in two cases (`PML35-7B615-TB` and `PML35-7B606-TB` for `...-GB`), which count as its false positives.
- No normalization differences were found.

Open items: the 5 to 20 missed UPS Code 128 reads.

## PDF417 reader (2026-10-09)

First measurement of `breader-pdf417` (`pdf417` is a default feature). Baselines rewritten for the synthetic sets, the zxing-cpp samples, `challenging-images` and `pdf417-small-module-size` at all three efforts; the diff against the old files is additions only (new PDF417 entries, plus the 33 / 80 new synthetic PDF417 labels), every other symbology unchanged, 0 false positives on every set.

PDF417 reads, release build (zxing-cpp 3.1.1 Python, default options; rxing with `--features baselines`, default options):

| Set | Expected | Fast | Balanced | Exhaustive | zxing-cpp | rxing |
|---|---|---|---|---|---|---|
| zxing-cpp samples `pdf417-1..3` | 58 | 51 | 55 | 54 | 58 | 0 |
| `challenging-images` | 6 | 3 | 3 | 3 | 3 | 0 |
| `pdf417-small-module-size` | 15 | 1 | 1 | 1 | 0 | 0 |
| synthetic seed 1 | 33 | 29 | 32 | 32 | 13 (1 FP) | 0 |
| synthetic seed 2024 | 80 | 73 | 75 | 79 | 25 | 0 |

(rxing reads no PDF417 symbol anywhere with the bench's default hints; it is listed for completeness.) The synthetic classes `clean`, `rotated`, `blur`, `low-res`, `illumination`, `inverted`, `noise` read 100 % at Balanced on seed 1; the misses are compact or perspective symbols (`perspective` 2 of 3 on seed 1, 4 of 7 on seed 2024). The dataset sizes changed with the new images: seed 1 286 to 319 symbols, seed 2024 831 to 911.

False-positive triage: none occurred on any set. The zxing-cpp samples label `13-readerinit` ("$I"): the symbol starts with the reader initialisation codeword, which breader rejected by design at the time (superseded by [D-035](decisions.md#d-035): it now reads, see [Reader initialisation symbols](#reader-initialisation-symbols-2026-10-09)). `pdf417-4` (Macro PDF417 split over several images) has no per-image labels and is not scored; `03-01.png` there reads.

Latency, `challenging-images`, interleaved before/after runs (three each, medians of the p50): Fast 4.61 to 4.73 ms, Balanced 16.75 to 17.0 ms (+0.25), Exhaustive 40.42 to 41.13 ms. The matcher on the linear scan's runs is the only cost of frames without PDF417. Frames with a symbol cost more: the 3 MP `air-travel-ticket` takes a few ms extra for the row lines.

Video: `batch_rotated_small_pdf417_1080p` shows a label of about 60 px width on a test tube, with modules well under a pixel. All 404 frames were read with breader (Balanced, Fast, tracker) and zxing-cpp: no PDF417 comes out of either, so there is nothing to label and nothing to count as a false positive. `video-bench` has an `ADDITIONS` list for verified labels, empty.

Misses worth knowing: 1 px modules (zxing sample `19`: the shared edge detector merges single-pixel elements), a pen stroke across the symbol (`10.webp`), compact symbols under strong perspective, and everything where modules are below one pixel (`pdf417-small-module-size`, the video).

## Aztec reader (2026-10-09)

First measurement of the `breader-aztec` reader (`aztec` is a default feature). Baselines rewritten for the synthetic sets and the zxing-cpp samples at all three efforts; the diff against the old files is additions only (new Aztec entries and the 36 / 81 new synthetic Aztec labels, 48 zxing sample labels that now count), every other symbology unchanged, 0 false positives on every set (Dynamsoft sets checked unchanged).

Aztec reads (release build; zxing-cpp 3.1.1 Python and rxing `--features baselines`, default options; the bench got an rxing Aztec mapping):

| Set | Expected | Fast | Balanced | Exhaustive | zxing-cpp | rxing |
|---|---|---|---|---|---|---|
| zxing-cpp samples `aztec-1` | 32 | 25 | 32 | 32 | 32 | 20 |
| zxing-cpp samples `aztec-2` | 16 | 16 | 16 | 16 | 16 | 2 |
| synthetic seed 1 | 36 | 33 | 36 | 36 | 36 | 1 |
| synthetic seed 2024 | 81 | 71 | 81 | 81 | 76 | 4 |

Synthetic images: compact (1 to 4 layers) and full-range (2 to 15 layers, up to 99 modules) symbols from rxing's encoder in the classes clean, rotated, perspective, blur, low-res, illumination, inverted, noise and multi, every third one mirrored; Fast reads no inverted symbol by design (dark-on-light only), which accounts for its misses on the synthetic sets. The whole zxing-cpp samples set at Balanced: Aztec 48 of 48 against zxing-cpp 48 and rxing 22; the total read count moved with the 48 new labels (2026-10-08: 348 of 470, now 468 of 576 with 0 false positives).

False-positive triage (`BENCH_SHOW_FP=1`):

- `aztec-1/Z1-sequence4of7`: the reader returned ` Z1.txt DG3456` for the label `3456`. That is the structured append header of the symbol (message ID `Z1.txt`, sequence `DG` = 4 of 7) followed by the payload: normalization, not a misdecode. The bench strips such a header from Aztec reads (`matching.rs`, with a test; `py_bench.py` the same), see [D-034](decisions.md#d-034). The decode is Reed-Solomon valid, and the file name (`sequence4of7`) and the label (`3456`) agree with the header's `DG`.
- `aztec-1/rune-*`: the labels are three digits (`001`, `064`); the reader first returned `1`, `64`. Fixed in the decoder (three-digit text) rather than in the matching, so users get zxing-cpp's spelling.
- No other false positive on any set, at any effort.

Misses found and fixed while measuring (all were decoder or sampler causes, none was relaxed): reader initialisation symbols (`readerinit-*`, a flag in the codeword count field), Binary Shift after a U/S shift (`dlusbs`), runes flush with the image border (core sampling outside the image), and the fit assuming rays that start at the true center (a quarter of a pixel off skewed the corners of the core by a module). The photo `aztec-2/04.webp` (strong perspective) needs the hill climb of the core corners on the bullseye and marks. Remaining misses: none on the zxing samples at Balanced or Exhaustive; Fast misses the small or tilted ones.

Latency, `challenging-images`, Balanced, interleaved before/after runs of one binary with the reader switched off by a temporary environment check (five each, medians of the p50): 17.08 ms to 17.36 ms (+0.28 ms), p95 about +3 ms. Scanning every pass instead of the first costs about +7 ms ([D-033](decisions.md#d-033)). Fast and Exhaustive latency were not measured.

Known limits seen in the unit and synthetic runs: modules of about 2.5 px or less are not detected reliably (2.75 px reads); 151-module symbols (32 layers) need about 3.5 px per module when rotated; strongly tilted large full-range symbols rely on the reference-grid anchors.

## Reader initialisation symbols (2026-10-09)

Data Matrix (Reader Programming, codeword 234 first) and PDF417 (codeword 921 first) now return such symbols with `reader_init` set instead of dropping them ([D-035](decisions.md#d-035)), as Aztec already did. zxing-cpp samples, reads / false positives: Data Matrix 79 / 86 / 89 -> 79 / 87 / 90 and PDF417 51 / 55 / 54 -> 51 / 56 / 55 at Fast / Balanced / Exhaustive (the two samples `datamatrix-1/readerinit.png` and `pdf417-1/13-readerinit.png` read from Balanced, matching their labels; Fast still does not find them), 0 false positives at every effort. Baselines `zxing-samples-{fast,balanced,exhaustive}.json` rewritten; the diff is two additions each at Balanced and Exhaustive and nothing at Fast. Other datasets unchanged.

## Codabar (2026-10-11)

First measurement of the Codabar decoder ([design](symbologies/linear.md#codabar), [D-036](decisions.md#d-036)). Release build, Ryzen 5 5500U, `tools/bench` (breader; rxing with `--features baselines`). Synthetic sets were generated into a worktree-local directory (`synth-local/seed1`, `synth-local/seed2024`) with the new Codabar images appended to every class but `negative` and `datamatrix`; earlier synthetic images were compared byte for byte with `data/synth` and `data/synth-2024` and are identical. The shared synthetic baselines are rewritten when the sets are regenerated.

| Set | Expected | breader Fast | breader Balanced | breader Exhaustive | rxing (TryHarder) | zxing-cpp |
|---|---|---|---|---|---|---|
| zxing samples `codabar-1` | 11 | 8 | 10 | 10 | 11 | not measured |
| zxing samples `codabar-2` | 4 | 2 | 2 | 2 | 2 | not measured |
| synthetic seed 1 | 66 | 62 | 66 | 66 | 38 (Balanced) | not measured |
| synthetic seed 2024 | 162 | 124 | 154 | 154 | 84 (Balanced) | not measured |

zxing-cpp could not be run (the Python package is not installed in this environment). Dynamsoft sets have no Codabar labels.

- **False positives: 0** on every dataset and effort, including the 46 zxing `falsepositives-*` images, the negative synthetic class and the Code 39 / ITF / EAN / Code 128 images (checked with `BENCH_SHOW_FP=1`). `tools/bench` `--check` passes at all three efforts on the seven gated sets with the existing baselines: no read count dropped for any symbology, so no Code 39 or ITF read turned into a Codabar read or the reverse. zxing-samples baselines were rewritten (Codabar additions only: `codabar-1` and `codabar-2`).
- **Misses.** `codabar-1/01`: no quiet zone, the bars touch the image border. `codabar-1/14` and `15` (Fast only): 1 px narrow elements, read after the up-scale retry from Balanced. `codabar-2/03` and `04`: quiet zones of about 3 modules, below the 10-module guard.
- **Latency** (`challenging-images`, p50 of 6 interleaved runs with and without the Codabar decoder doing any work, indicative on a shared machine): Fast 4.97 to 5.06 ms, Balanced 19.4 to 17.9 ms (noise); synthetic seed 1: Fast 1.76 to 1.76 ms, Balanced 5.94 to 6.13 ms. No measurable cost, so Codabar runs at every effort including Fast.

## Micro QR reader (2026-10-11)

First measurement of Micro QR in `breader-qr` (no new feature, [D-038](decisions.md#d-038)). Baselines rewritten for the zxing-cpp samples at all three efforts, additions only (the `microqrcode-1` entry, 16 labels that now count). The synthetic baselines were not rewritten: the shared sets are regenerated by the architect; the numbers below are from worktree-local sets (`synth-local/seed1`, `seed2024`, existing images byte-identical to the shared ones up to the Aztec block). 0 false positives on every set at every effort.

Micro QR reads (release build; zxing-cpp 3.1.1 Python and rxing `--features baselines`, default options):

| Set | Expected | Fast | Balanced | Exhaustive | zxing-cpp | rxing |
|---|---|---|---|---|---|---|
| zxing-cpp samples `microqrcode-1` | 16 | 12 | 14 | 14 | 15 | 14 |
| synthetic seed 1 (local) | 36 | 33 | 35 | 35 | 32 | 30 |
| synthetic seed 2024 (local) | 82 | 71 | 79 | 79 | 74 | not measured |

Format distance: accepting 3 instead of 2 added no read on any of these sets at any effort (measured by rebuilding with the limit at 3), so the single copy is held to 2.

Latency, indicative (other agents were running; `challenging-images`, p50 ms, three interleaved runs with the Micro QR path switched off by a temporary switch and on): Balanced 17.5 to 18.7 without, 18.2 to 18.5 with; Fast 4.9 to 5.0 without, 5.2 to 5.3 with (about +0.3 ms).

Misses: `7.webp` and `9.webp` (photographs, 4 to 5 px per module, the grid is off by up to a module in the data area), at Fast also `MQR-needs-br-update.jpg` and `NoQuietZone.png`; synthetic: 2.3 px low-res symbols, a few perspective and multi-symbol images. See [known gaps](symbologies/microqr.md#known-gaps).


## rMQR reader (2026-10-11)

First measurement of rMQR in `breader-qr` (no new feature, [D-039](decisions.md#d-039)). Baselines rewritten for the zxing-cpp samples at all three efforts, additions only (the `rmqrcode-1` entry, 3 labels that now count). The synthetic baselines were not rewritten: the shared sets are regenerated by the architect; the numbers below are from worktree-local sets (`synth-local/seed1`, `seed2024`; every pre-existing image byte-identical to the shared sets, checked with `cmp`; the rMQR images are the 27 and 63 added ones). 0 false positives on every set at every effort, and none on the shared synthetic sets, Dynamsoft sets and zxing samples gates.

rMQR reads (release build; zxing-cpp 3.1.1 Python and rxing `--features baselines`, default options):

| Set | Expected | Fast | Balanced | Exhaustive | zxing-cpp | rxing |
|---|---|---|---|---|---|---|
| zxing-cpp samples `rmqrcode-1` | 3 | 1 | 3 | 3 | 3 | 2 |
| synthetic seed 1 (local) | 32 | 31 | 32 | 32 | 7 | 5 |
| synthetic seed 2024 (local) | 80 | 76 | 80 | 80 | 18 | 11 |

The synthetic images are encoded by `qrcode-generator` (all 32 sizes at M and H, three payload kinds, mirrored every third image, all classes); the other readers read few of them (small modules, perspective, noise), which says more about the images than about the readers.

Misses: at Fast the two `R7x43-H` samples (2 px modules, 88x16 and 86x14 images: the up-scale retry does not run at Fast), one noise image of seed 1, two noise and two multi-symbol images of seed 2024 (candidate cap of 4, jpeg noise).

Latency, indicative (other agents were running; `challenging-images`, p50 ms, three interleaved runs with the rMQR path switched off by a temporary environment switch and on): Fast 5.30, 5.30, 5.42 without and 5.40, 5.33, 5.36 with (0.0 to 0.1 ms, under the 0.3 ms line, so rMQR is not gated to Balanced); Balanced 18.99, 19.14, 18.77 without and 19.82, 19.86, 19.18 with (+0.4 to +0.8 ms).

Allocations (`cargo test --release -p breader --test allocation -- --nocapture`, QR reader only, allocations/KiB; Fast, Balanced, Exhaustive): texture 800x600 1/6K, 6/45K, 6/44K; texture 320x240 0/0K, 3/19K, 12/70K; one QR 17/3K, 20/3K, 20/3K; blank frames 0. Bounds unchanged. `Homography::fit` no longer collects its solution into a `Vec`, which a debug build kept as an allocation per fit (the test also passes in debug builds now).

Format distance: the sampled grid accepts 3 per copy (two copies agreeing) and 2 for a single copy. A limit of 2 for both copies was not measured separately; 3 is kept because two copies that agree cross-check each other.

## GS1 DataBar (2026-10-11)

First measurement of the DataBar decoders ([design](symbologies/linear.md#gs1-databar), [D-037](decisions.md#d-037)). Release build, Ryzen 5 5500U, `tools/bench` (breader; rxing with `--features baselines`; zxing-cpp 3.1.1 through `tools/bench/py_bench.py` in a virtualenv). Synthetic sets were generated into worktree-local directories (`synth-local/seed1`, `synth-local/seed2024`) with 3 DataBar images (Omnidirectional, Limited and Expanded in turn, from the independent `anyd` encoder) appended to every class but `negative` and `datamatrix` after the Micro QR ones; every earlier image is byte-identical to `data/synth` and `data/synth-2024`. Reads of the labels, breader Fast / Balanced / Exhaustive:

| Set | Expected | breader | rxing (TryHarder) | zxing-cpp |
|---|---|---|---|---|
| zxing `rss14-1` and `rss14-2` (Omnidirectional; the 7 of `rss14-2` are stacked) | 20 | 10 / 10 / 10 | 13 (Balanced) | 17 |
| zxing `databarltd-1` (Limited) | 2 | 2 / 2 / 2 | 0 (no Limited reader) | 2 |
| zxing `rssexpanded-1`, `-2`, `-3` | 162 | 160 / 162 / 162 | 158 (Balanced) | 162 |
| synthetic seed 1, Omnidirectional | 10 | 8 / 10 / 10 | 5 | 8 |
| synthetic seed 1, Limited | 10 | 10 / 10 / 10 | 0 | 8 |
| synthetic seed 1, Expanded | 11 | 7 / 11 / 11 | 6 | 9 |
| synthetic seed 2024, Omnidirectional | 29 | 23 / 29 / 29 | 18 (2 false) | 21 |
| synthetic seed 2024, Limited | 22 | 20 / 22 / 22 | 0 | 13 |
| synthetic seed 2024, Expanded | 25 | 20 / 24 / 24 | 15 | 21 (2 false) |

- **False positives: 0** (breader) on every dataset and effort, including the 46 zxing `falsepositives-*` images, the negative synthetic class, all Dynamsoft sets (`--check` passes at three efforts on `challenging-images`, `skewed-datamatrix`, `UPS-Synthetic-Labels` and `pdf417-small-module-size`; the video gate was not run here) and the EAN / Code 128 / Code 39 / ITF / Codabar images (checked with `BENCH_SHOW_FP=1`). No read count dropped for any other symbology (`--check` against the existing synthetic and zxing baselines). Two of the first runs read wrong strings from valid symbols (the 3203 weight offset and a date field of 38400 meaning "no date"); both are fixed and covered by tests. rxing reads the expanded stacked rows as Expanded (63 "false positives" there are labels still counted as unsupported).
- **Misses.** Stacked symbols (the 7 of `rss14-2`, which rxing and zxing-cpp read; `rssexpandedstacked-*` are unsupported labels). `rss14-1` images 8, 11 and 13 to 15 are blurred photographs: 1 to 2 px modules whose narrow bars merge; Balanced reads 10 of 13, zxing-cpp 11. Images that touch the symbol with the border (`rss14-1/4`, `/5`, `rssexpanded-1/*`) are read through the cut-guard rule. Synthetic misses are rotated, perspective and blurred images at Fast.
- **Latency** (`challenging-images`, p50 of 5 interleaved runs, two binaries with and without the decoder doing any work, indicative on a shared machine): Fast 5.82 / 5.80 / 6.01 / 6.18 / 5.91 ms with against 5.83 / 5.98 / 5.88 / 5.92 / 5.68 without; Balanced 20.71 / 20.90 / 23.25 / 20.88 / 19.57 with against 19.77 / 20.33 / 22.78 / 20.88 / 20.09 without. Mean cost +0.05 ms at Fast (limit 0.3 ms) and +0.2 ms at Balanced, so the decoders run at every effort. An earlier version without the screens (finder A for Expanded, narrow elements for Omnidirectional) cost +10 ms p95 at Balanced.
- **Baselines.** `zxing-samples-{fast,balanced,exhaustive}.json` rewritten, additions only (the six DataBar sets); the synthetic baselines are regenerated after the merge.

## Phase D latency (2026-10-12)

*Superseded by [Status 2026-10-13](#status-2026-10-13).*

What the Phase D readers (PDF417, Aztec, Codabar, Micro QR, rMQR, GS1 DataBar) and the UPC-E guard fix cost, against the [v1 targets](#v1-targets-confirmed-2026-10-07). Machine: AMD Ryzen 5 5500U (6 cores / 12 threads), idle (`uptime` and `ps` checked before each block: no cargo, rustc or bench process), release builds, the library single-threaded, decode call only.

Method:

- **post-D** is main at `bc63f72`. **pre-D** is the same tree with `breader`, `breader-qr`, `breader-linear` and `breader-datamatrix` taken from `10c37d7` (the last commit before the PDF417 work; it keeps the QR / Data Matrix sampling retries and the edge relocation, so only Phase D is measured). Four struct literals and one `match` arm were patched in the pre-D copy so that it builds against the newer `breader-core` and the current bench; this was done in a scratch worktree, removed afterwards. Each binary has its own target dir.
- Interleaved runs, old then new, 5 rounds, for each dataset and effort: `bench <set> --effort E` (`data/dynamsoft/challenging-images`, `data/barber --subset 8` as in `tools/check-regression.sh --barber`, `data/synth` seed 1). The tables give the median over the 5 runs of the whole-set p50 / p95. Run-to-run spread of the p50 is 0.05 ms or less.
- zxing-cpp 3.1.1, default options, `tools/bench/py_bench.py` (challenging-images 3 runs, BarBeR subset 3 runs).
- Fast at 1080p: `video-bench data/dynamsoft/video-based-testing --engines fast,balanced --filter 1080p`, 3 interleaved rounds, ten 1080p videos, per-frame latency (the method of the target). Median over rounds of the per-video p50.
- Profile: a scratch program (outside the repository) scans challenging-images at Fast / Balanced with one symbology set per run, and with "all minus X". No `perf` here (`perf_event_paranoid` is 4).

Latency, p50 / p95 in ms:

| Dataset | Effort | pre-D | post-D | change of p50 | zxing-cpp |
|---|---|---|---|---|---|
| challenging-images | Fast | 4.68 / 32.4 | 5.39 / 36.3 | +15 % | |
| challenging-images | Balanced | 16.93 / 110.5 | 19.55 / 125.9 | +15 % | 22.7 / 326 |
| BarBeR 1/8 subset | Fast | 5.49 / 21.7 | 6.00 / 24.2 | +9 % | |
| BarBeR 1/8 subset | Balanced | 19.79 / 86.5 | 21.80 / 100.4 | +10 % | 32.9 / 198 |
| synth seed 1 | Fast | 1.48 / 5.9 | 1.75 / 8.2 | +18 % | |
| synth seed 1 | Balanced | 5.44 / 27.3 | 6.08 / 37.0 | +12 % | |

On these sets post-D reads more (it has the new classes): challenging-images Fast 327 -> 330 and Balanced 405 -> 408 of 448; BarBeR subset Fast 671 -> 674 and Balanced 782 -> 785 of 1093; synth Fast 244 -> 426 and Balanced 283 -> 482 of 487. False positives of breader: 0 in every run.

Fast at 1080p (video-bench, per-frame p50 ms; the median over the ten videos is 6.8 pre-D, 7.3 post-D):

| Video | pre-D Fast | post-D Fast | pre-D Balanced | post-D Balanced |
|---|---|---|---|---|
| batch_EAN13 | 5.0 | 5.3 | 13.0 | 13.9 |
| batch_code39 | 11.0 | 15.9 | 51.6 | 75.4 |
| batch_dark_environment_1d | 5.3 | 5.4 | 13.6 | 13.9 |
| batch_datamatrix | 9.7 | 10.9 | 53.7 | 60.7 |
| batch_inverted_datamatrix | 8.3 | 9.1 | 43.4 | 47.4 |
| batch_rotated_small_pdf417 | 8.4 | 11.3 | 32.0 | 48.7 |
| damaged_qrcode | 8.8 | 9.8 | 19.1 | 21.4 |
| reflective_surface_qrcode | 4.3 | 5.4 | 13.7 | 17.3 |
| rotated_vertical_ITF | 5.2 | 5.5 | 13.6 | 14.2 |
| wide_to_close_datamatrix | 4.3 | 4.3 | 16.1 | 16.3 |

Cost by reader (challenging-images, mean ms per image over 65 images of 3 MP, single run of 7 repetitions, minimum; "all minus X" against all post-D readers, so the shared preprocessing is not charged to X):

| Reader | Fast | Balanced |
|---|---|---|
| Aztec | +0.70 | +2.4 |
| GS1 DataBar (three classes) | +0.22 | +2.1 |
| Codabar | +0.15 | +1.6 |
| PDF417 | +0.14 | +1.5 |
| rMQR | +0.14 | +0.6 |
| Micro QR | +0.01 | +0.05 |
| all six together | +1.55 (9.63 -> 11.18) | +8.8 (35.8 -> 44.5) |

The Fast numbers of the scratch program are the stages-style means (9.63 / 11.18), not the p50 of the table above. The six readers cost +16 % of the mean at Fast and +25 % at Balanced on 3 MP photos; a 1080p video frame pays about +1 ms at Fast on most videos and +3 to +5 ms on the three that already do work (code39, rotated pdf417, datamatrix).

Targets:

| Target | Result |
|---|---|
| Balanced p50 <= zxing-cpp | Met. challenging-images 19.55 against 22.7 ms, BarBeR subset 21.80 against 32.9 ms. The margin on challenging-images fell from about 5.8 ms to 3.1 ms. |
| Fast <= 10 ms at 1080p | Not clean. 7 of 10 videos stay under 10 ms (median video 7.3 ms), but `batch_code39` (15.9), `batch_rotated_small_pdf417` (11.3) and `batch_datamatrix` (10.9) exceed it; before Phase D only `batch_code39` (11.0) did. The 3 MP stills are at 5.4 ms (challenging-images Fast p50). |
| 0 false positives | Met for breader (0 on challenging-images, BarBeR subset, synth at Fast and Balanced). zxing-cpp has 17 on the BarBeR subset and 1 on challenging-images. |
| Read rate >= zxing-cpp per class | Not met on real photos, unchanged by Phase D: challenging-images ITF 0 of 4 against 4 (Balanced; QR 229 against 220, UPC-A 71 against 70, Code 128 41 against 28, Data Matrix 45 against 44, PDF417 3 against 3); BarBeR subset QR 113 against 130, Code 39 11 against 18, ITF 9 against 13 (EAN-13 524 against 496, Code 128 67 against 51; total 785 against 763). Synth classes were not compared here. |

The Fast regression is above 10 % on challenging-images (+15 %) and synth (+18 %), BarBeR is +9 %. Aztec is the largest single cost at Fast; the Fast pass still runs all six readers (Codabar, DataBar, PDF417, rMQR cost 0.14 to 0.22 ms each). Nothing was changed in the reader.

Note: the Python package in the shared `pyvenv` is an older build of `breader-py`; its numbers (2 false positives on challenging-images) are not those of the Rust bench and were not used.

## zbar (2026-10-13)

zbar is a comparison engine in `tools/bench/py_bench.py` (`--engine zbar`, or `--engine all` for breader, zxing-cpp and zbar). It is used as a black box through its public C API via pyzbar 0.1.9 (`pip install pyzbar`; needs the system library, `apt install libzbar0`; libzbar 0.23). Default settings; `--zbar-all` passes every symbology explicitly, which changed nothing on challenging-images (160 reads, 5 false positives both ways). zbar's own sources were not read. It reads Code 93, EAN-2/5, SQ code and composite codes too; those have no breader counterpart, so labels in them are unsupported and zbar's reads of them are dropped. zbar has no Data Matrix, Aztec, Micro QR, rMQR or DataBar Limited reader, so those labels count as misses for it, as for any engine.

Quirks normalised, so that the comparison is on values and not on spelling (all in `ZbarEngine` and the shared `canonical`):

- UPC-A comes back as UPCA or as EAN-13 with a leading 0, UPC-E as UPCE: folded into EAN-13 on both sides, as for the other engines. ISBN-13 is an EAN-13.
- ITF is reported at any length (zbar has its own minimum of 6 digits, no list like breader's [D-008](decisions.md#d-008)); no length filter is applied, so its false positives include fragments of real ITF symbols.
- Payloads are bytes: a non-UTF-8 payload is also offered as Shift JIS and Latin-1 text, because labels hold UTF-8.
- Codabar comes with its start/stop characters (A to D): dropped from both sides, as for the other engines.
- DataBar has no AIs: a bare 14-digit DataBar result gets the label's `01` prefix; parentheses of expanded output are dropped on both sides.

Subsets only (not the full BarBeR set). Reads / expected, false positives, latency p50 / p95 ms. breader Balanced is the committed baseline (`tools/bench/baselines/*-balanced.json`):

| Set | breader Balanced | zxing-cpp | zbar |
|---|---|---|---|
| challenging-images | 411 / 448, 0 FP, 20.2 / 131 | 388 / 448, 1 FP, 22.2 / 323 | 160 / 448, 5 FP, 46.3 / 583 |
| zxing-samples | 681 / 794, 0 FP, 4.1 / 23.8 | 716 / 794, 69 FP, 1.4 / 8.0 | 450 / 794, 74 FP, 1.9 / 12.2 |
| BarBeR 1/8 subset | 803 / 1093, 0 FP, 22.4 / 104 | 765 / 1093, 15 FP, 33.0 / 192 | 713 / 1093, 29 FP, 65.9 / 394 |
| synth seed 1 | 482 / 487, 0 FP, 6.1 / 40.0 | 381 / 487, 11 FP, 13.3 / 41 | 201 / 487, 43 FP, 16.4 / 88 |

(Re-measured 2026-10-13 on an idle machine, see [Status 2026-10-13](#status-2026-10-13): breader from the Rust bench, zxing-cpp and zbar from `py_bench.py`; the synth seed 1 zxing-cpp and zbar cells are from the first run of this section, on the shared machine.)

zbar is far behind on the Dynamsoft photos (QR 41 of 234, Code 128 30 of 60; UPC-A 68 of 74 and EAN-13 16 of 18 are close to the others) and on the synthetic sets (it has no Aztec, Data Matrix or Micro QR). On the BarBeR subset it is closer (65 % against 70 % for zxing-cpp and 73 % for breader) and does well on linear codes and the large-QR sources; its latency is the highest of the three on every set but zxing-samples.

False positives of zbar:

- challenging-images: 5 ITF, all pieces of the four real ITF symbols (`28056306` and `03280563` for `0328056306`, `698860`, `188010`, ...): partial reads that the length rule of breader would reject.
- BarBeR subset: 25 EAN-13 (wrong digits of real symbols, e.g. `8083882032262` against `8083822038262`), 3 ITF, 1 Code 128.
- zxing-samples: 50 DataBar Expanded (the stacked `rssexpandedstacked-*` labels are unsupported, so any read counts as a false positive, as for zxing-cpp's 64), 9 QR (Japanese samples such as `d-qr.net` whose bytes differ from the label), 11 EAN-13, 3 Code 39 (e.g. `12+A+B`), 1 Code 128.
- synth: 38 ITF (fragments of ITF symbols on the rotated, perspective, noisy and low-resolution classes), 4 QR, 1 DataBar Expanded.
