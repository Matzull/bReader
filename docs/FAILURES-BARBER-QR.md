# BarBeR: QR codes that zxing-cpp reads and breader does not

Dataset: [BarBeR](datasets.md#barber), 1329 QR labels (after the loader's cleanup). Measured on tree `bd844ee`, release build. zxing-cpp 3.1.1 (Python, default options) is used as a black box (outputs only). This is a triage: nothing under `crates/` or `tools/` changed. All experiments ran on a scratch copy of the workspace outside the repository (instrumented `breader-qr` with environment-variable switches, a one-file driver that scans QR only at Balanced), and the data directory was only read.

Counting note: this triage matches per label (a label is read when any read of its image has its value), which gives breader Balanced 953 and zxing-cpp 1162; the bench's one-read-per-label matching gives 945 and 1150 ([BASELINES](BASELINES.md#barber-2026-10-11)). The difference is duplicate labels and does not change the picture.

## 1. Summary

| | labels |
|---|---|
| read by both | 922 |
| zxing-cpp only (the misses analysed here, 236 images) | 240 |
| breader only (count only) | 31 |
| neither | 136 |

Misses by source: Dubska 218 (of its 802 QR labels), Szentandrasi-Dubska QR 12, ZVZ-real 5, Deal Kaist 2, Bodnar 1, InventBar 1, SKUU DSP Lab 1. Open Food Facts and ParcelBar: 0. Breader-only reads: Dubska 9, Szentandrasi 6, Bodnar 4, Deal Kaist 4, InventBar 3, ZVZ-real 3, Open Food Facts 1, SKUU 1.

Misses by pixels per module (annotation `PPE`): under 2: 5, 2-3: 1, 3-4: 9, 4-6: 94, 6-10: 49, 10-15: 15, 15-25: 58, 25 and over: 9, none: 1. So this is not a small-module problem: 82 of the 240 have modules of 10 px or more.

The set is dominated by one source: Dubska is the "cite_NN_L/M/Q/H.png" series, a QR of a mid or high version (v7 to v13, all four error-correction levels) printed on a page of text and photographed obliquely, usually 2560x1440, sometimes with a shadow or glare band across it, plus an older series of 402x604 images (`IMG_1xxx`) with modules of 4 to 6 px.

Pipeline stage of the first failure (Balanced, QR only, trace of the finder search, triplet selection and `try_dims`; counted for the finders inside the labelled polygon):

| stage | labels | evidence |
|---|---|---|
| T1 no finder candidate | 4 | 0 finders of plausible size in the polygon |
| T2b only 1-2 finders found | 26 | the other finders are lost (shadow, glare, blur, low contrast) |
| T2 3+ finders, no triplet | 87 | triplet rejected by the geometry gates: dimension consistency (`dimdiff`) 61, leg ratio / angle / module ratio 14, no rejection recorded (finders of other polarity or other symbols) 12 |
| T3 triplet made, no plausible grid | 107 | the real triplet was rejected (as in T2) while other triplets were tried; or its timing / finder-block checks failed |
| T4 grid sampled, Reed-Solomon failed | 13 | grid wrong (curved sheet, shadow band, drift) |
| T5 read when run alone | 3 | the QR-only run reads them, the bench's JPEG decode of the same file does not: marginal reads that flip with the decoder (Rust `image` against PIL) |

Hardly any miss is dropped by a guard after a successful decode (no stage reads and then discards).

## 2. Root causes, ranked by reads recovered

Gains are measured on the 240 misses with the QR-only scratch driver at Balanced, one switch at a time on the unchanged tree. Reads are Reed-Solomon validated, so each recovered label is a correct payload.

| rank | cause | labels | measured gain | fix complexity | FP risk |
|---|---|---|---|---|---|
| 1 | **Perspective: the triplet gates assume an isotropic module** (`dimdiff`, leg ratio, corner angle, module ratio) | about 140 (T2 87 + most of T3) | relaxing only the `d1`/`d2` dimension consistency (0.2 to 0.5 of the dimension): +129; plus leg ratio 3.0, angle cos 0.8, module ratio 3.2: +139 (+136 net of the 3 decoder-noise reads) | low | low (see section 4) |
| 2 | **Finder detection fails under uneven light, blur or low contrast** (1-2 finders found, or none) | 30 (T1 + T2b); 5 recovered by 1, 18 left | none measured | medium | low to medium |
| 3 | **Dense mid-size modules with a grid that does not follow the print** (T3 and T4 left after 1: 62, mostly 4-6 px modules of v10-v13 on curved or oblique pages) | 62 | skipping the timing / finder-block plausibility tests: +5; up-scale retry for 604 px frames: +4 | high | medium |
| 4 | Huge modules (58 px, 12 MP) and two codes in one frame | about 5 | none measured | low to medium | low |
| 5 | JPEG decoder sensitivity of marginal reads | 3 | not a reader defect | none | none |

### Cause 1: perspective and the dimension estimate

The finder module size is an isotropic mean of the horizontal and vertical chords. In an oblique photo the modules are foreshortened along one axis, so the dimension estimated from leg length over module size differs between the two legs. `make_triplet` rejects the triplet when `|d1-d2| > 0.2 dim`. The same shot also fails the timing-pattern link check that would have supplied a verified dimension (`dp = dq = 0`), so the relaxed path (`hinted`) is not available either.

Example, `20110817_025.jpg` (clean v7 print, text right at the edges): three real finders (modules 16.9, 17.0, 23.7 px), 4 finders, 0 triplets; the only candidate is rejected with `d1=34.5 d2=26.6` and all three timing links score 0. With the gate relaxed it reads. Further examples of the same stage evidence: `20110817_022`, `_151`, `_365`, `_064`, `_154`, `_170`, `_202`, `_369` (strong tilt), `IMG_1556`, `IMG_1662`, `IMG_1699` (mild tilt, dense), `IMG_20120226_151917` (clean sticker at a steep angle), `5010284100032` (Deal Kaist, ink smear, tilted).

Of the 240: the dimension gate alone accounts for 129 recovered (T2dim 57 of 61, T3 60 of 107, T2b 2, T4 5, T1 1, T5 3, T2none 1); the other three gates add 10 (T2leg 4 of 14, T2b 3).

### Cause 2: finders not found

`20110817_160` (shadow gradient across the sheet): the first pass finds one real finder (module 14.8) and one at 19.6 of the other polarity; the second pass finds none. Same family: `_251`, `_328` (diagonal shadow, steep tilt), `_303` (defocus plus perspective), `_318`, `_028`, `IMG_2712` (blurred, gray, low contrast), `5900084049238` (103 px, green background, blur). Binarization with a global or coarse adaptive threshold loses the finder ring in the shadowed corner. `IMG_2715` (a clean 12 MP print with 58 px modules, T1) finds 33 spurious finders at module 11 inside the real finder rings and none at the real size: there is no coarse scale for modules above about 25 px.

### Cause 3: dense, mid-size modules; wrong grids

`IMG_1562`, `IMG_1596`, `IMG_1646`, `IMG_1749`, `IMG_1836`, `IMG_1783`, `IMG_1825`, `IMG_1859`, `IMG_1879`, `IMG_1894` (402x604 frames, QR of 270-420 px, v10-v13, about 4-6 px per module, mild tilt): estimated dimensions are around 40-57 and the true triplet fails `finders_plausible` and `timing_plausible` under the affine fourth corner; with both tests skipped only 5 more are read, so the geometry (fourth-point guess, alignment) is the limit. T4 examples: `20110817_220`, `_243`, `_333` (shadow stripe on a curved page), `IMG_1518`, `IMG_1580`, `IMG_1668` (sampling drift), `065` (rotated sticker), `20140607_204118` (133 px code in a frame). The 604 px frames are 4 px over the up-scale retry limit (`DEFAULT_UPSCALE_MAX_SIDE` 600); raising the limit to 700 recovers 4 of the 240 on top of cause 1.

### Other visual causes seen

Moire of an LCD photographed (`QR-photo_26_0011`, T3), blurred dense codes (`QR-photo_26_0063`, `_0058`), two QR in one frame with a noisy finder set (`IMG_2760`, `IMG_2769`), glare bands (`20110817_154`, `_202`, `_380`, `_392`). No miss was inverted, no miss lacks a quiet zone beyond the text printed up to the code's edge (breader reads those once the triplet is made), and none was dropped by a guard.

## 3. Visual causes of the stratified sample

43 misses, stratified by stage (T1 1, T2b 2, T2 19, T3 12, T4 6, T5 3; non-Dubska sources over-sampled), plus 32 of the residual after cause 1 viewed on crops of the labelled polygon. Counts are of the sample, not of the 240.

| visual cause | sample |
|---|---|
| oblique perspective on a clean print (mid or high version) | 22 |
| perspective plus a shadow or glare band | 15 |
| blur or low contrast | 6 |
| small modules (4-6 px, v10+) | 10 (overlaps with perspective) |
| curvature of the sheet | 4 |
| damage / ink smear | 1 |
| multiple codes in frame | 2 |
| huge modules | 1 |
| moire of a screen | 1 |

## 4. False-positive and regression check of the experiment

Switch "cause 1" (leg ratio 3.0, angle 0.8, module ratio 3.2, dimension difference 0.5, all as in the existing `hinted` branch for the leg and module limits), scratch copy, Balanced, QR reads and false positives:

| set | QR reads before | after | false positives |
|---|---|---|---|
| BarBeR 1/8 subset | 113 | 126 | 0 true (1 listed: a correct read of `http://jdi.na.vutbr.cz/uvod`, whose label has the typo `http//`, and which the baseline does not read at all) |
| zxing-samples | 125 | 128 | 0 |
| Dynamsoft challenging-images | 229 | 229 | 0 |
| Dynamsoft skewed-datamatrix | 0 | 0 | 0 |
| synth seed 1 (incl. negatives) | 65 | 65 | 0 |

Latency of the whole set at Balanced did not change measurably (p50 22.4 against 22.9 ms, p95 103 against 110 ms on the BarBeR subset; the machine was shared, so indicative only). Extrapolated to the full set: 945 / 1329 would become about 1080 (zxing-cpp 1150). The trade-off is more triplets per image, each costing the plausibility tests; the existing failure budgets (`max_failures`) bound it.

## 5. Recommended fixes

1. **Triplet selection, `detect.rs::make_triplet` and `select_triplets`: perspective-tolerant geometry (about +130 labels).** Measure the module along each leg (the finder chord in the direction of that leg, not the isotropic mean) to estimate `d1` and `d2`, or keep the isotropic estimate and widen the `d1`/`d2` tolerance; let the angle, leg-ratio and module-ratio limits of the hinted case apply when the number of finders is small; rank such triplets after the strict ones, so clean images keep their order. Reed-Solomon and the version check stay the validators. Risk low; 0 false positives on every set tried. Do this first and re-measure before the rest: part of cause 3 may disappear because the right triplet is then tried first.
2. **Finder search: robustness to uneven light and blur (about 15 of the 30 labels with missing finders, estimate).** Options: a QR-specific local-contrast binarization for the finder scan only (when fewer than three finders of the plausible size are found, rescan a locally normalized copy), a wider ratio tolerance on the second binarizer, and a downscaled copy for frames with modules above about 25 px (IMG_2715). Medium complexity; each rescan is conditional on a failed search, so the happy path does not change.
3. **Grid fit for dense v10+ symbols under tilt (`reader.rs::try_dims`, `sample.rs`), about +10 (estimate).** Fit the homography progressively from the alignment patterns (the `Refined` path is only used for dimension 45 and above), and apply `timing_warp` and the local-threshold retry to the first plausible grid even when the plausibility tests fail on the affine map. Also raise the up-scale retry limit to cover 604 px frames (+4 measured). High complexity, medium risk (more grids sampled), validated by Reed-Solomon.
4. Low priority: a second pass for frames with two codes (finder pairing across symbols), and the 3 marginal reads that depend on the JPEG decoder.

## 6. Method

Labels: `tools/bench/py_bench.py` loader and matching (`load_barber_sources`, overrides, de-duplication), one process pool for both engines (breader-py rebuilt from this tree at Balanced, zxing-cpp default), QR-labelled images only. PPE is read from the VIA annotation. Stage evidence: scratch copy of `breader-qr` printing, per scan call, the finders, links, rejected triplets (with reason) and the dimension candidates with their plausibility results, matched to the labelled polygon (EXIF-mapped, expanded by 20 %). The scratch copy and its switches are not in the repository.
