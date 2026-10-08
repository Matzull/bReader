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

## Linear reader fixes (2026-10-08)

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
| QR Code | 138 | 95 / 0 | 115 / 0 | 115 / 0 | 119 / 0 | 125 / 1 |
| EAN-13 | 65 | 24 / 0 | 39 / 0 | 40 / 0 | 43 / 4 | 45 / 4 |
| EAN-8 | 9 | 7 / 0 | 8 / 0 | 8 / 0 | 8 / 6 | 9 / 0 |
| UPC-A | 76 | 21 / 0 | 53 / 0 | 56 / 0 | 50 / 0 | 50 / 0 |
| UPC-E | 29 | 14 / 0 | 20 / 0 | 22 / 0 | 18 / 17 | 20 / 0 |
| Code 128 | 22 | 15 / 0 | 21 / 0 | 21 / 0 | 21 / 0 | 22 / 0 |
| Code 39 | 14 | 7 / 0 | 11 / 0 | 11 / 0 | 11 / 3 | 12 / 0 |
| ITF | 20 | 1 / 0 | 2 / 0 | 2 / 0 | 17 / 0 | 20 / 0 |
| Data Matrix | 97 | 72 / 0 | 79 / 0 | 80 / 0 | 78 / 2 | 93 / 0 |
| **Total** | 470 | 256 / 0 | 348 / 0 | 355 / 0 | 365 / 32 | 396 / 5 |

Cells are reads / false positives. Read rate: breader 54.5 % / 74.0 % / 75.5 %, rxing 77.7 %, zxing-cpp 84.3 %. breader has zero false positives at every effort. The other engines' false positives were not triaged. rxing (`TryHarder`, multi) panics on one image (`aztec-1/gs1-figure-4.15.1-2-31x31.png`) and loops on another (`itf-1/#220.png`, abandoned after 30 s); both count as no read. zxing-cpp is the Python package with default options (a UPC-A read can come back as EAN-13). Counts include the linear fixes, the up-scale retry and the edge/border-quiet-zone changes (2026-10-09). A Code 39 read also matches its Full ASCII expansion (`code39-2` labels the expanded text while breader defaults to plain Code 39).

Balanced and Exhaustive include the up-scale retry (3x Catmull-Rom copy of frames up to 600 px that yielded nothing); with the linear fixes and before it they read 292 and 297 (QR 102/102, UPC-A 39/40, UPC-E 16/18, Code 128 14/14, ITF 1/1, Data Matrix 72/74, the rest unchanged). The retry also adds one QR read on challenging-images (225 -> 226 of 234 QR at Balanced and Exhaustive) and nothing on synth. Fast has the retry off; enabled it would read 308 here and nothing more on challenging-images. Latency on this set, p50 / p95 ms, same session, interleaved runs: Balanced 1.8 / 7.3 before, 4.7 / 21.0 after; Exhaustive 4.5 / 15.8 before, 11.7 / 44.5 after (most of the 932 images are small, and the negative and unsupported-symbology ones all take the retry); challenging-images Balanced 16.9 / 119 before, 17.0 / 119 after, Exhaustive 38.0 / 277 before, 41.0 / 338 after. A blank 600x600 frame costs +15 ms at Balanced and +26 ms at Exhaustive, a noise-filled one +142 and +340 ms.

Latency: to be measured on an idle machine.

Ground truth additions (`tools/bench/gt-overrides.json`, each checked on the image):

- `qrcode-2/16.webp`: a nested double QR; only the inner symbol is labeled, the outer one is a second readable symbol.
- `code39-1/code32-1.png`, `code39-1/pzn-1.png`: Code 32 and PZN are carried in plain Code 39 symbols and labeled with their own (unsupported) symbology; the Code 39 content of the same symbol (`3PRM8N`, the base-32 form of the Code 32 value A123456788, and `-12345678`) is added.

Where breader reads less than zxing-cpp:

- ITF: by design, the default accepts 14 digits only (`ItfOptions::allowed_lengths`); 17 of the 20 labels are 4 to 10 digits long (the 3 of 14 digits are read in one case).
- EAN/UPC (`ean13-4`, `upca-2`, `upca-4`, `upce-3`) and QR Code (`qrcode-4`): not investigated; Fast misses most of them (fewer passes).
- Code 128 and Code 39 (`code128-3`, `code39-2`, `code128-1`): not investigated.
- Data Matrix: `datamatrix-5` (0 of 5) and a few in `datamatrix-3`.
- A kana-only Shift JIS QR payload (`qrcode-2`) is reported with `text` as Latin-1, since the heuristic requires a double-byte character; `data` is the exact payload.
