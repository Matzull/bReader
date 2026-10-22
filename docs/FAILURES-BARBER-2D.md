# BarBeR and zxing samples: 2D codes that zxing-cpp reads and breader does not

Datasets: [BarBeR](datasets.md#barber) (QR Code, PDF417, Data Matrix) and the [zxing-cpp samples](datasets.md#zxing-cpp-samples) (Data Matrix). Measured on main at `662f3f6` (the tree of [Status 2026-10-13](BASELINES.md#status-2026-10-13)), release build, Balanced. zxing-cpp 3.1.1 (Python, default options) is a black box (outputs only; its source was not read). This is a triage: nothing under `crates/` changed. Stage evidence comes from `eprintln!` instrumentation and environment-variable switches in `breader-qr`, `breader-datamatrix` and `breader-pdf417`, applied to a worktree, measured and reverted (they are not in the repository); a one-file driver scanned one symbology family at a time, and `data/` was only read.

Counting note. The tables count per label (a label is read when any read of its image has its value, matching as in `py_bench.py`, after the harness fix of section 0). On BarBeR that gives breader 1098 / zxing-cpp 1155 for QR (the Rust bench's one-read-per-label count in [BASELINES](BASELINES.md#per-class-comparison-at-balanced) is 1094 / 1153), PDF417 25 / 37 (bench 26 / 36) and Data Matrix 94 / 99 (bench 95 / 99). The Python image decoder (PIL) and the Rust one (`image`) differ on about 1 % of the JPEG files (11 of 1104 images of the subset read differently), which moves these totals by a read or two. Zxing-samples Data Matrix is 87 / 93 in both.

## 0. The Python bench disagreement

The Python binding was not the cause; `py_bench.py` was. Both paths build `ScanOptions::with_effort(effort)` (the binding's defaults, `min_line_agreement` 2, ITF lengths, Code 39 minimum length and the symbology set are the core defaults; no option differs). Three differences of the harness:

| difference | effect |
|---|---|
| The Rust bench (`engine.rs`) offers every read under alternative forms: the charset-decoded `text`, a Shift JIS reading of bytes that are not UTF-8, the Code 39 Full ASCII expansion. `py_bench.py` offered only the last | a correct read of a label written in another charset (Latin-1, Kanji, Cyrillic text in QR and Data Matrix) was counted as a false positive and the label as a miss. This is the whole zxing-samples gap: 662 reads + 19 FP became 681 + 0 in Rust; Python now reads 682 with 0 FP |
| Labels: the Rust matcher repairs UTF-8 that was read as Latin-1 and encoded again (`canonical_label`); Python did not | a few labels never matched |
| Gray conversion: PIL `convert("L")` is Rec. 601 (299/587/114); the `image` crate's `into_luma8` is 2126/7152/722 with truncating division | the pixels differ by a few levels; marginal reads flip |

Fixed in `tools/bench/py_bench.py` (`BreaderEngine.read`, `undo_latin1_utf8`, `to_luma8`). After the fix, Python against Rust at Balanced: challenging-images 412 / 0 FP against 411 / 0; zxing-samples 682 / 0 against 681 / 0; BarBeR subset 807 / 0 against 803 / 0. What remains is the JPEG decoder: scanning the PIL-decoded gray array and `scan_file` (the Rust decoder) on the same 1104 subset files gives different results on 11 (for example `20110817_242.jpg` reads a QR only with PIL, `Excise_stamp_0044.jpg` reads a PDF417 only with the Rust decoder); on challenging-images it is one image (`batch-scanning-1.jpg`, one UPC-A). The binding produces no false positive that the Rust API would not. The zxing-cpp and zbar columns of [BASELINES](BASELINES.md#status-2026-10-13) were measured before the fix; the changes affect their label matching in the same direction (never negative), so they may rise by a few reads and were not re-run.

## 1. Summary

| Class | labels | breader reads | zxing-cpp reads | zxing-cpp only (analysed) | breader only | stage of the first failure |
|---|---|---|---|---|---|---|
| QR Code, BarBeR | 1329 | 1098 | 1155 | 95 (in 95 images) | 38 | detection 38 (finders missing or mis-paired), grid 48, Reed-Solomon 9 |
| PDF417, BarBeR | 67 | 25 | 37 | 18 (17 images) | 6 | row reading / grid, 17 of 18 |
| Data Matrix, BarBeR | 110 | 94 | 99 | 7 | 2 | detection 2, grid fit / sampling 5 |
| Data Matrix, zxing samples | 97 | 87 | 93 | 7 | 1 | grid fit / sampling 7 |

Fix list, ranked by reads gained (details in section 6):

| rank | fix | measured / estimated reads | FP risk |
|---|---|---|---|
| 1 | QR: finder chord aspect limit 0.6..1.7 is too tight for oblique photos | **+59 measured** (BarBeR Dubska + Szentandrasi-Dubska 826 to 885 of 1029, 0 FP) | low |
| 2 | QR: failure budget (`max_failures` 4) is spent on wrong triplets before the right one | +7 measured on top of 1 (with 12) | low |
| 3 | QR: `timing_plausible` gate rejects oblique grids | +3 measured on top of 1 | low to medium |
| 4 | PDF417: row-synchronous line placement and gray averaging per row (excise stamps, 24 rows of 2 modules) | 3 to 8 estimated (+2 of 15 measured with denser lines alone) | low |
| 5 | Data Matrix: sample module gray values instead of binary pixels for 2 to 4 px modules, one hypothesis check per size | 4 to 8 estimated (14 labels) | low (RS-validated) |
| 6 | QR: dense blurred or shadowed v10+ (remaining 33) | 5 to 10 estimated | medium |
| 7 | PDF417 `Special_0012` boundary growth, Data Matrix detection of 2 BarBeR images | 1 to 3 | low |

Fix 1 alone brings breader's BarBeR QR count (1094 in the bench) to about 1153, zxing-cpp's.

## 2. QR Code (BarBeR, 95 zxing-cpp-only labels)

The earlier [QR triage](FAILURES-BARBER-QR.md) (tree `bd844ee`, 240 misses) led to the perspective-tolerant triplet geometry; the gap has shrunk from 240 to 95 labels (the 95 are: Dubska 78 of its 802 QR labels, Szentandrasi-Dubska QR 10, ZVZ-real 5, Deal Kaist 1, InventBar 1). The stage counts below are measured on the current tree (finders, links, triplets and dimension tests matched to the labelled polygon expanded by 20 %).

| stage | labels | evidence |
|---|---|---|
| T2b: 1 or 2 finders found | 3 | `20110817_318`, `IMG_2712` (blurred, gray), `5900084049238` |
| T2: 3 or more finders, no triplet | 35 | pair links score under 0.75 or none (31 labels with a low-score link, 10 with no link); 11 rejected by leg ratio / corner angle |
| T3: a triplet made, no grid sampled | 48 | the dimension candidates rejected by `timing_plausible` (46) or `finders_plausible` (19); the failure budget was spent in 28 |
| T4: grid sampled, Reed-Solomon failed | 9 | |
| read when run alone | 0 | (1 marginal read flips with the decoder) |

The stages overstate how many different problems there are: most of T2 and T3 share one cause.

### C1. The finder chord aspect limit (about 50 labels)

`detect.rs::confirm_finder` accepts a finder only when the vertical chord over the horizontal chord of its outer ring lies in 0.6 to 1.7. In an oblique photo the two top finders of a symbol lie farther away and are foreshortened along one axis more than that. Example `20110817_028.jpg` (a clean, sharp print of version 7 at 2560x1440): the top-left finder is about 140 px wide and 80 px high (ratio 0.57), the top-right 135 by 75; both are rejected in all passes at all scales and only the bottom-left (190 by 120, ratio 0.63) and an alignment pattern are found. The same image cropped to the symbol also fails, so it is not a binarization or scale issue. With the limit at 0.35 to 2.8 the crop reads.

`IMG_2716.JPG` (a clean 12 MP print with 26 px modules, listed as "huge modules" before) is the same family: the three real finders are found, but one only in the inverted polarity (9 scanline hits against 19 and 25 for the others), so no polarity has three; with the wider limit it reads.

Measured effect (switch `BX_ASP`: limit 0.35 to 2.8, nothing else changed):

| set | before | after | false positives |
|---|---|---|---|
| the 95 labels, QR only, Balanced | 1 | **51** | 0 (payload compared) |
| BarBeR Dubska and Szentandrasi-Dubska QR (1029 labels), Rust bench Balanced | 826 | **885** (+59) | 0 |
| BarBeR 1/8 subset, all classes | 803 | 809 | 0 |
| challenging-images / zxing samples / skewed-datamatrix | 411 / 681 / 1012 | same | 0 |

Cost: the switch read the environment inside the finder loop, so its time is overstated; measured Balanced p50 +6 % (BarBeR subset 22.8 to 24.2 ms), +17 % on Dubska (20.2 to 23.7 ms), +11 % on Szentandrasi-Dubska (85 to 95 ms, p95 138 to 251: 12 MP frames, more finders to pair); challenging-images and zxing samples unchanged within noise. More finders also mean more candidate triplets; the failure budget bounds the work.

Proposed fix: widen the limit to about 0.4..2.5 for the chord ratio, but keep the strict 0.6..1.7 first; when a polarity has fewer than three finders (or no triplet decodes), run the relaxed confirmation as a second tier on the finder candidates that failed only this test (keep the failing row hits, as `make_triplet` already has a `relaxed` tier), and rank triplets built from relaxed finders after the strict ones. The finder module used by the later steps should be the per-axis module, not the isotropic mean (the triplet code already uses `module_at`). Expected +55 to +60 labels, FP risk low (Reed-Solomon and the version check validate).

Status 2026-10-14: C1 is implemented as [D-045](decisions.md#d-045) (BarBeR QR Balanced 1094 to 1156). Of C2, a strict failure budget of 12 (+4 reads on the Dubska sets, +2.5 % p50) and skipping `timing_plausible` for wide triplets (+1) were measured and not kept; the wide tier has its own budget (12 at Balanced).

### C2. Failure budget and plausibility gates (about 10 more)

After C1 the residual stage counts (of the 95): read 50, T3 26, T4 11, T2 8. In T3 the failure budget was exhausted in 19 of 26 (`max_failures` 4 at Balanced): wrong triplets built from text-line finder candidates and spurious finders come first and use it up. On the 95 labels, on top of C1: a budget of 12 reads 58 (+7 over C1's 51), skipping `timing_plausible` reads 54 (+3), both 62 (+11); Exhaustive with C1 reads 58 (the extra budget of Exhaustive is the same lever). 1/8 subset with all three: 810 (C1 alone 809). Proposed fix: order the triplets by link evidence (verified timing link score, then geometry) and make the budget a function of the number of triplets with a verified link, so a pass can spend more on triplets with evidence and none on texture; relax `timing_plausible` for triplets with an oblique leg ratio (sample the timing pattern along the perspective-corrected line). Expected +8 to +11, FP risk low (every extra grid is still Reed-Solomon validated), cost to be measured on the negative sets.

### C3. What is left (33 of the 95 after C1 to C3)

Visual classes in the remaining images (crops viewed): heavy defocus or motion blur of dense v10+ symbols (`20110817_023`, `_153`, `_184`), shadow gradients that put one finder in a dark band (`_190`, `_201`, `_263`), dense v13+ at 3 px modules with perspective (`IMG_1740`, `IMG_1749`, `IMG_1795`, `IMG_1894`, the 402x604 / 604x402 frames; the up-scale retry applies only when nothing was found and the side is at most 600), the ZVZ-real photos with 1.4 to 2 px modules (`QR-photo_26_0003`, `_0011`, `_0058`, `_0063`), and two QR in one frame (`IMG_2760`, three labels). The first triage's proposals (local-contrast binarization for the finder scan when fewer than three finders are found, progressive homography from alignment patterns for dimension 45 and above, a 3x up-scale of the polygon for 3 px modules) apply unchanged; estimated 5 to 10 labels, medium complexity, FP risk medium.

## 3. PDF417 (BarBeR, 18 zxing-cpp-only labels in 17 images)

15 of the 18 are `Excise_stamp_*` (photos of Russian excise stamps on glass, mostly 3552x2000; images 0007, 0015, 0016, 0022, 0025, 0028, 0030, 0031, 0034, 0038, 0039, 0041, 0042, 0043, 0045), 2 labels are `Special_0243` and 1 `Special_0012`. All 67 PDF417 labels of BarBeR come from ZVZ-real. The excise stamps are 37 of them: zxing-cpp reads 15, breader 1; on the other 30 labels breader reads 24 and zxing-cpp 22.

Stage (Balanced, PDF417 only, instrumentation of the seed loop and `grid::solve`):

| stage | labels | evidence |
|---|---|---|
| marks, seed and boundary found, lines read, `solve` fails: too many erased cells | 15 stamps | every stamp is a 24 rows x 3 columns symbol, error-correction level 3 (16 check codewords, budget 14 after the guard's reserve of 2); 72 cells of which 15 to 61 are erased by the vote (examples: `0015` 17, `0030` 31, `0016` 18 and 31) |
| low resolution | 1 (`0007`, 22 rows, level 0, module 1.9 px: 36 of 66 cells erased, no budget at level 0) | |
| boundary stops early | 1 (`Special_0012`, 27x22, level 4: the boundary grows over 61 px of a symbol 338 px high, 474 of 594 cells erased) | |
| tiny symbol | 2 (`Special_0243`, 11x2 symbol in a 67x610 box, 15 of 22 cells erased) | |

The stamp images are sharp (modules of 8 to 10 px). The cause is the row geometry: the printed rows are about 2 modules high (the symbol is 24 rows over about 430 to 620 px with modules of 8 to 10 px), under the 3 modules the standard asks for, and the line spacing is 1.25 modules, so a row gets 1.4 lines on average (the debug print shows `lines per row [1, 0, 2, 1, 1, ...]`, with rows that no line falls inside). One line per row, straddling two rows when the sampling phase is wrong, gives wrong or erased codewords. Ink spread joins the elements (cropped view of `Excise_stamp_0030`). Measured: a line spacing of 0.6 modules gives 2 to 4 lines per row and 16 erased cells instead of 31 (budget 14); 0.4 modules reads 2 of the 15 images (`0015`, `0043`); 0.15 modules reaches 8 erased cells and still fails, so some non-erased cells are wrong too.

Proposed fix (PDF417 `reader.rs::read_straight` and `grid.rs`): after the first pass the indicators give the row pitch (rows against `t`, the `fit` that `assign_rows` already computes); read the symbol again with lines at the row centres (and a profile averaged over the central half of each row before edge detection), instead of lines at a fixed module spacing. Optionally allow the budget `ec - reserve` to use the full `ec` when the erasure count is far above the error count (not tried; the misdecode guard is the reason for the reserve). Estimated 3 to 8 of the 15 stamps (2 measured with a denser spacing alone), FP risk low (Reed-Solomon, length descriptor and the cluster check stay). `Special_0012` needs the boundary growth to continue past the first gap; `Special_0243` is a 67 px wide symbol. Both are 1 to 3 labels.

### Triage and outcome (D-054, 2026-10-22, main `0c5773a`)

Per image, Balanced, against zxing-cpp 3.1.1 (black box): 14 labels read by zxing-cpp only and 10 by breader only (33 against 37). zxing-cpp only: 11 stamps (`0007`, `0015`, `0022`, `0025`, `0030`, `0031`, `0034`, `0038`, `0039`, `0041`, `0042`), `Special_0012` and `Special_0243` (2 labels). Stamp ground truth: the text zxing-cpp reads, re-encoded with the test-only rxing encoder at the stamp's 24 x 3, level 3 layout, compared cell by cell with the vote grid of the dense pass (a temporary test; the stamps share their first six rows). Result: the votes were mostly right (`0015` 57 right, 0 wrong, 15 erased; `0031` 55 / 0 / 17; `0042` 52 / 4 / 16; `0039` 50 / 5 / 17; `0025` 57 / 3 / 12) and the erased cells were whole rows at the two ends of the symbol, because the boundary ends 1 to 4 rows short and no pass read lines past it. This corrects the cause named above (rows about two modules high alone): the dense pass fixed the row pitch, the missing rows were the last obstacle for the stamps near the budget. Fix: lines past the boundary ([D-054](decisions.md#d-054)). Outcome: BarBeR Balanced 39 (+6: `0015`, `0031`, `0032`, `0039`, `0042`, `0050`), Exhaustive 38 (from 35), Fast 25 (unchanged), zxing-cpp 37; 0 false positives; the zxing samples gap is closed (58 of 58 at Balanced).

Still unread after it (zxing-cpp only): `0007` (module 1.9 px, 47 of 72 cells erased), `0022` (no start or stop mark), `0025` (rows 4, 13, 23 unreadable on the right half, 12 erased and 3 wrong), `0030`, `0038` (12 erased, 3 wrong), `0041` (5 erased, 13 wrong cells), `0034` (26 wrong cells: geometry), `Special_0012` (27 x 22, level 4: the boundary covers half the symbol, 265 of 594 cells erased after the dense pass), `Special_0243` (11 x 2, rows about one module high, 8 erased against a budget of 6). Tried and dropped: more erasure retries in `decode_cells` (no gain). Next candidates, untried: nearest-codeword reading within the row's cluster, a template match of the row's mean profile against the cluster's 929 patterns.

## 4. Data Matrix, BarBeR (7 labels) and zxing samples (7 labels)

BarBeR: `0381370033875`, `4014400900651`, `4902777015927`, `8711200402779`, `ProductBarcode106`, `20140607_200820`, `Special_0232`. Symbols of 30 to 80 px (box 30x27 to 72x79) at 2 to 4 px per module, 10x10 to 24x24 modules, except `Special_0232` (a 282x125 px symbol in a 2479x3508 frame). zxing samples: `#794`, `144x144_wrong`, `distorted`, `dm-2.jpg`, `dm-2x2-#669-2`, `dm-f.webp`, `issue794-12-1.webp`.

| stage | BarBeR | zxing samples | evidence |
|---|---|---|---|
| detection: no candidate with a layout | 2 (`8711200402779`, `ProductBarcode106`) | 0 | no candidate passes `precheck` / `analyze` |
| candidate and L found, every size hypothesis fails Reed-Solomon | 5 | 7 | `plausible` true on 3 (`0381370033875` 12x12, `distorted` 12x12, `#669-2` 48x48) and the decoder still fails: the sampled grid has bit errors beyond the correction capacity ([D-015](decisions.md#d-015) corrects fewer codewords on small blocks) |

Visual causes: tiny and slightly blurred symbols (`distorted` 60x60 px, `0381370033875`: 12x12 modules at 2.7 px), perspective on a phone screen (`dm-2.jpg`: candidates with the wrong size hypotheses 16, 18, 20) and on a label (`dm-f.webp`: hypotheses 32 and 36 for a symbol of a smaller true size), a curved bottle (`issue794-12-1`, size hypotheses 20 to 26 and 48x22), bold ink-bleed (`#794`, 98x123 px), a multi-region symbol where the clock warp is found and still fails (`#669-2`, reads at Exhaustive only), and a 144x144 symbol whose layout hypotheses fail (`144x144_wrong`). The bench's own [documentation](FAILURES-ZXING-SAMPLES.md#data-matrix) already listed these.

The reader (`sample.rs::sample_grid_warped`) reads each module from the binarized plane at the module centre (with a 5-point vote only above 3 px per module), so on 2 to 4 px modules the grid carries the binarizer's errors without any use of the gray values between. Proposed fix: for symbols with a module under 4 px, sample the gray image (bilinear mean over the module area), threshold per module with the local black and white level of the L pattern and clock tracks (both already measured by `analyze`), and decode with the soft values as erasure candidates (`decode_grid` taking a reliability per module and trying the weakest modules as erasures, like the PDF417 retry). Estimated 4 to 8 of the 14, FP risk low (Reed-Solomon validated, size and pad checks stay), cost only on candidates that already failed. A second item: the frame up-scale retry (side at most 600 px, and only when nothing was found) covers none of the 7 BarBeR images (612x816, 584x1000, 1125x2000, 488x1000, 480x640, 1440x2560, 2479x3508 and so on); a candidate-local up-scale (crop the candidate, enlarge 3x, resample) would cover them and the small zxing samples.

**Update 2026-10-14.** Fix 3 (PDF417 row pitch) is in: [D-047](decisions.md#d-047), ZVZ-real PDF417 Balanced 26 to 33 and Exhaustive 24 to 35 (the density-only variant of this section and the per-row averaged profile were both needed; 11 of the 15 stamps read in a driver run). Fix 4 (Data Matrix gray-level sampling) was built and measured and gave 0 to 1 read: the failing symbols have wrong geometry, not binarizer errors (details in D-047); it is not in the tree.

**Update 2026-10-22 ([D-055](decisions.md#d-055)).** Re-triage on main `0c5773a`: 6 BarBeR labels read by zxing-cpp and not by breader (2 the other way) and 7 on the zxing samples (1 the other way). The stage table above was wrong about the cause: the symbols that fail are not binarizer errors and mostly not "every hypothesis fails Reed-Solomon" either; the true size was often never hypothesised (counted clock runs 10 and 14 for a 16x16, 9 and 12 for a 12x12, 17 and 25 for a 26x26) or the far corner was a module off, or the pitch changed along an axis (`distorted`). Fix: three Balanced/Exhaustive retries (clock-track warp, one-track size hypotheses, corner search over every plausible size). Outcome: read `20140607_200820`, `4014400900651`, `4902777015927` (BarBeR), `distorted`, `dm-2`, `dm-f` (zxing samples) and 59 more of the skewed-datamatrix set; BarBeR Balanced 95 to 98 of 110 (zxing-cpp 99; Exhaustive 99), zxing samples Balanced 87 to 91 of 97 (zxing-cpp 93). Left: `8711200402779` (light print with a thick light L on a black tube, rotated, blurred; no candidate with a layout), `ProductBarcode106` (25 x 20 px, 2 px modules), `Special_0232` (candidate quad swallows the text line above), the curved bottle (`#794`, `issue794-12-1`), `#669-2` (Exhaustive only) and `144x144_wrong` (non-conformant). The gray-level sampling of this section stays dropped; the two small undetected candidates of D-047 are still undetected.

## 5. Where breader leads (for the record)

BarBeR QR: breader-only 38 labels (Dubska 9, Szentandrasi 6, Bodnar 4, Deal Kaist 4, InventBar 3, ZVZ-real 3, ...), PDF417 6, Data Matrix 2. Any fix above must keep them: the checks are the Rust bench per set and `check-regression.sh --tier c2`.

## 6. Ranked fix list

| rank | fix | where | reads | FP risk | complexity | note |
|---|---|---|---|---|---|---|
| 1 | Widen the finder chord aspect limit as a second tier (relaxed finders ranked after strict ones) | `breader-qr/src/detect.rs::confirm_finder` and finder bookkeeping | **+59 on BarBeR (measured), 0 FP** on every set run | low | low | brings BarBeR QR to zxing-cpp's level; do first, then re-measure the rest |
| 2 | QR triplet budget by link evidence, `timing_plausible` relaxed for oblique legs | `breader-qr/src/reader.rs`, `detect.rs` | +8 to +11 (measured with global switches: +7 and +3) | low | medium | re-measure after 1: the order of triplets changes |
| 3 | PDF417 rows at the row pitch, per-row averaged profile | `breader-pdf417/src/reader.rs::read_straight`, `grid.rs` | 3 to 8 (the 15-stamp series; 2 measured with a denser spacing) | low | medium | one series of one source, but 15 of the 18 |
| 4 | Data Matrix gray-level module sampling with erasures for weak modules, below 4 px | `breader-datamatrix/src/sample.rs`, `decode` | 4 to 8 (14 labels on two sets) | low | medium | |
| 5 | QR, remaining blur / shadow / dense: local-contrast finder rescan, progressive homography, polygon up-scale | `breader-qr` | 5 to 10 | medium | high | |
| 6 | PDF417 boundary growth past a gap; Data Matrix detection of 2 small candidates | `breader-pdf417/detect.rs`, `breader-datamatrix/detect.rs` | 1 to 3 | low | low | |
| - | JPEG decoder differences (1 % of images flip a read) | not a reader defect | none | | | the cause of the 1 to 4 remaining Python/Rust differences |

## 7. Method

Labels, matching, overrides, de-duplication: `tools/bench/py_bench.py` (after the fix of section 0; the same files and polygons as the Rust bench, EXIF mapped to the raw frame). One process pool of four, breader (`breader-py` built from this tree, Balanced, all symbologies) and zxing-cpp default, on the images that carry a label of QR, PDF417 or Data Matrix; "zxing-cpp only" means the label is matched for zxing-cpp and not for breader. Stage evidence: instrumented `breader-qr` (finders with polarity and strength, pair links with score, rejected triplets with the reason, dimension candidates with the plausibility results, sampled grids with the Reed-Solomon outcome), `breader-datamatrix` (candidates, layout, hypotheses, plausibility, decode errors, clock warp) and `breader-pdf417` (marks, seeds, boundaries, lines read, layout and erased cells per row), all printed only under an environment variable, and matched to the labelled polygon (expanded by 20 %). Switches (`BX_ASP`, `BX_NOTIM`, `BX_FAIL`, `BX_UNV`, a line-spacing override) were read from the environment in a scratch copy of the code, one at a time and combined. Single-switch results on the 95 QR labels (Balanced, QR only): chord aspect +51 over the 1 read at baseline; timing gate alone +4; budget 24 alone +5; unverified-link threshold 400 alone 0; Exhaustive 8. False-positive and regression checks used the Rust bench (`BENCH_SHOW_FP=1`, `RAYON_NUM_THREADS=4`): challenging-images, zxing samples, BarBeR 1/8 subset, skewed-datamatrix and the Dubska sources, 0 false positives in every run.
