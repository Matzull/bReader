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

