# zxing-cpp samples: failure analysis (images zxing-cpp reads, breader does not)

Dataset: `data/zxing-samples/test/samples` (zxing-cpp tag v3.1.1 test images), 470 labels in the nine v1 symbologies. Measured with the current tree (22ae624), release build. zxing-cpp 3.1.1 (Python, default options) is used as a black box only.

Status of this file: the triage itself changed nothing under `crates/` or `tools/`; ranked fixes 1 (U, section 2) and 2 (E) and 3 (Q) ("Implemented" below the table of section 2) have since been implemented. The tables and counts in the rest of the file describe the state before them. All experiments ran on a scratch copy of the workspace with environment-variable switches (`/tmp/claude-1000/-home-matzul-Documents-Breader/2d145433-4560-4365-862d-8a30122dc9df/scratchpad/triage-zx/`, `prototype_edges_runs.diff` holds the two core patches). No GT overrides were edited.

## 1. Summary

114 labelled images are read by zxing-cpp and by neither breader Balanced nor Exhaustive (Balanced 291, Exhaustive 296, zxing-cpp 396 including its false positives; breader reads 13 images zxing does not: the two GT-override Code 39 images, 3 EAN-13, 4 UPC-A, 1 UPC-E and 3 QR).

| symbology | both-miss | of which: policy/option only | single-line/edge/QZ (1D algorithm) | 2D / tiny / geometry |
|---|---|---|---|---|
| QR Code | 26 | 0 | | 26 |
| Data Matrix | 19 | 3 (pad check, Reader Programming, 144x144 layout) | | 16 |
| EAN-13 | 14 | 0 | 14 | |
| UPC-A | 14 | 0 | 14 | |
| UPC-E | 4 | 0 | 4 | |
| EAN-8 | 4 | 0 | 4 | |
| Code 128 | 8 | 0 | 8 | |
| Code 39 | 6 | 3 (Full ASCII off by default) | 3 | |
| ITF | 19 | 16 (lengths [14]; min length 6) | 3 | |
| total | 114 | 22 | 50 | 42 |

Policy versus algorithm for ITF (asked explicitly): with `allowed_lengths = []` (any even length >= 6) breader Exhaustive reads 9/20 instead of 1/20 (8 images are pure policy). 7 more become readable only when the quiet-zone defect (class Q) is also fixed (they have 10 digits or 6 digits and image-border margins under 10 modules), 1 (`#853`, 4 digits) needs `min_length <= 4`, 3 are algorithmic (1 px modules, shear, 4 px high image). So 16 of 19 are policy-gated, 3 algorithm.

Combined prototype (all switches below on, scratch build, Rust bench scoring, 0 false positives on every set):

| set (Balanced) | baseline | prototype |
|---|---|---|
| zxing-samples (470) | 291 | 375 (Exhaustive 296 -> 378, Fast 243 -> 278 without the up-scale retry) |
| dynamsoft challenging-images (442) | 392 | 406 |
| dynamsoft skewed-datamatrix (1323) | 1000 | 1000 |
| synth seed 1 / seed 2024 | 280 / 808 | 283 / 811 |

Of the 114 both-miss images the prototype recovers 70 (Exhaustive); no image that baseline reads is lost (checked per image on zxing-samples). 44 remain (QR 13, Data Matrix 13, EAN-13 8, UPC-A 3, UPC-E 2, Code 39 3, ITF 2), listed in section 5.

## 2. Root-cause classes and ranked fixes

Counts are images of the 114 (each image has one primary class). "Measured" gains are Balanced, zxing-samples, one switch at a time on the baseline unless stated.

| rank | class | images | fix hypothesis (file / function) | measured gain | FP risk |
|---|---|---|---|---|---|
| 1 | **U**: small modules / jagged or smooth-needed frames; nothing decodes at native scale, everything decodes after a 3x resample | QR 13, DM 7, UPC-A 4, C128 2, UPC-E 1, ITF 1 (= 28 recovered; classes Q1/Q3/Q4/D2/D3/M below overlap) | pipeline: when a frame yields no symbol and `max(w,h) <= ~600`, rescan a 3x Catmull-Rom up-scaled copy (2x was worse: +22 and one EAN-13 FP; 4x: +23). Implemented as `imgproc::Upscale` plus a final conditional scan in `Pipeline::scan` (`PipelineBuilder::upscale`, `ScanOptions::upscale_retry`); the copy is scanned as a frame of its own with the regular pass list and the quads are mapped back. Running it also when something was found gives +1 (320) and is not done. Cost: Balanced p50 / p95 1.8 / 7.3 -> 4.7 / 21.0 ms on this set. | **Fixed.** +28 Balanced (291 -> 319: QR +13, DM +7, UPC-A +4, C128 +2, UPC-E +1, ITF +1) and +29 Exhaustive (296 -> 325); dynamsoft challenging +1 at both; synth 0; Fast unchanged (retry off by default; with it on Fast would read 243 -> 308) | none seen (0 FP on zxing-samples incl. falsepositives-1/2, synth seeds 1 and 2024 incl. negative, challenging-images, at every effort) |
| 2 | **E**: edge relocation reference level; blurred or thin-bar EAN/UPC reads all digits but with >= 2 weak digits or 1 line | 17 (EAN-13 5, UPC-A 10, UPC-E 2) | `breader-linear/src/edges.rs`, `EdgeDetector::refine_edges`: the half-level reference comes from the extremes of the two neighbouring runs only (`low/high(profile[p..=c])`). Narrow bars never reach full contrast, so their crossing sits at a different level than wide bars. Using the mean of that level and the min/max of a +-24 sample window around the edge removes the width-dependent bias. Window 12..64 all give +18..+21; pure window levels (no blend) lose reads (EAN-13 -1). | +21 (291 -> 312: EAN-13 +5, UPC-A +12, UPC-E +4); dynamsoft +8 (392 -> 400), synth +3 / +3 | none seen (0 FP on all sets) |
| 3 | **Q**: quiet zone is limited by the image border, not by content (tight crops, 1D) | 11 (EAN-8 3, C128 5, C39 2, EAN-13 1) + 7 ITF | `breader-linear/src/runs.rs`, `Runs::space_before` / `space_after` for the first / last bar (profile border), and the QUIET checks in `ean.rs::read_layout`, `code128.rs::start_of/read_symbol`, `code39.rs::start_of`, `itf.rs::start_of/stop_at`: a margin that ends at the profile border is only a lower bound, so accept it when it is >= ~1 px (prototype: treat as infinite). zxing-cpp black box: reads with 0 margin at an image border, but needs >= 3 modules on the right and >= 6 on the left of EAN-13 when a real neighbour bar exists, so content-limited quiet zones should keep a (smaller) minimum. | +9 alone (EAN-8 +3, C128 +5, C39 +1); with class L +16; 0 change on dynamsoft/synth | low: checksum + agreement still apply; keep full QZ when the margin ends at a bar |
| 4 | **L**: option defaults. ITF `allowed_lengths = [14]`; Code 39 `full_ascii = false` | ITF 16, C39 3 | `breader-core/src/options.rs` `ItfOptions::default` (empty list, `min_length` 6, keep the 3-line gate) and `Code39Options` (decode both; prefer Full ASCII when every `$%/+` pair is valid and at least one is present). `min_length 4` produced one FP (`code128-2/37.webp`: a Code 128 read as ITF "9004"), `min_length 6` none. | ITF 1 -> 8 alone (+7 with C39 full ASCII, 291 -> 298); with Q: ITF 16 | ITF is the weakest 1D code; 0 FP measured at min 6 with 3 lines |
| 5 | **A**: only one scanline reads the symbol | 3 (+ 2 tiny-height images) | `breader-linear/src/aggregate.rs`: accept a single line when `checksum_verified`, strong digit margins and no conflicting group (`min_line_agreement` 1 on every set: 0 FP) | +5 (291 -> 296), and +5 again on top of E+Q+L | moderate: zxing-cpp itself accepts one line |
| 6 | **G**: guard calibration / digit match fails on heavily blurred photos (rise 2-3 modules) | 13 | no cheap lever found: relaxing `GUARD_TOLERANCE` 0.55 -> 1.0 and `MODULE_DRIFT` 1.8 -> 3 gives +1 read and +1 FP. Candidates: sharpen/deconvolve the profile before `calibrate`, or fit the guards on the sharpened profile only for lines that find a plausible start | at most ~4 | high |
| 7 | **Q3**: QR sampled grid has 0.5-3.7 % wrong modules (diagonal bands, crumpled/curved sheets): RS fails | QR 9 (5 recovered by the 3x retry: #709, qrcode-3/07, 14, qrcode-4/04, 12) | `breader-qr/src/reader.rs::try_dims`: the `Refined` alignment fit is only used for `dim >= 45`; for v2-v6 use timing-pattern based local correction (module phase along row/col 6) or a piecewise warp | **Fixed in part (2026-10-09).** `timing_warp` (pitch of the columns and rows measured on the finder edges and timing patterns, smoothed, applied twice) as a retry after RS fails, at every dimension below and above 45. Recovered at Balanced: qrcode-3/03, qrcode-4/05, 06, 07, 09, 11, 31, 41 (plus 01, 04, 12, 14, qrcode-3/07, 14 at Fast, which the 3x retry already read at Balanced); qrcode-4/08, 13, 30, 33 and qrcode-3/23 stay unread (the drift is not separable per axis there). QR 115 -> 125 Balanced together with the items below | none seen (RS validated, 0 FP) |
| 8 | **Q4**: projective skew / curvature, no plausible grid | QR 7 | `breader-qr/src/detect.rs::select_triplets` (module size ratio <= 1.6, leg ratio) and `finders_plausible`: relax per triplet when finder modules differ by a smooth gradient (perspective) | up to 4 | low |
| 9 | **D4**: Data Matrix sampling drift on multi-region symbols (errors confined to the left columns) | DM 8 | `breader-datamatrix/src/sample.rs::refine`: per-region fit / +-1 phase shifts of the left regions validated by RS | **Fixed (2026-10-09)** with `clock_warp`: per-boundary offsets from the clock transitions of the top rows and right columns of every region, as a retry after RS fails or the plain grid is implausible. Recovered at Balanced: #1063, #1072-2, #1088, dm-2x2-a, issue1063, issue1072-left (6; the last two need the retry on implausible grids too); #669-2 only at Exhaustive; dm-2x2-b stays unread | none seen |
| 10 | **D1**: non-conformant encoders | DM 3 | `decode/encodation.rs` case 129: skip the 253-state pad check when RS corrected 0 errors (`abcd-36x16.png`); `decode/mod.rs` de-interleave: add the "ECC at 10x155" layout fallback for 144x144 (`144x144_wrong.png`); Reader Programming (234): decode and flag | **Pad check fixed** (`abcd-36x16`): the 253-state pad check is waived when RS corrected 0 errors and the block has more than 12 check codewords; `144x144_wrong` (layout) and `readerinit` (policy) unchanged | none seen |
| 11 | **Q2/Q5**: nested QR, QR Model 1, styled QR, truncated inverted QR | QR 4 | `reader.rs::scan`: the `known`/`inside` containment filter drops the inner symbol (BR_NOSKIP: both read); Model 1 format info (grid already perfect); gates for round modules | **Nested QR fixed**: a finder inside a decoded quad is kept when its module is below 0.5 of that symbol's (`qrcode-2/16.webp` now reads both); Model 1, styled and truncated inverted QR unchanged. Local-threshold sampling of the luma image (retry) reads `qrcode-2/qr-inv-1.webp` | none seen |

Implemented (2026-10-08, `breader-linear`, release build, 0 false positives on every dataset at every effort, including `falsepositives-1/2`):

- **E** (rank 2): `EdgeDetector::refine_edges` averages the neighbouring-run reference levels with the extremes of a window around the edge. The window is 8 mean run widths (mean over the 4 edges on each side) on each side, limited to 8..64 samples, instead of the fixed +-24 samples of the prototype, and each blended level may move at most 0.25 of the local level difference away from its local value. The limit was added because the plain blend lost whole groups that read before: `upca-3` (crinkled bags with plastic glare: a bar weakened by glare, not by blur, was narrowed because the window levels assumed blur), synth-2024 `illumination` ITF and the video `EAN_13-distance`; caps of 0.15 to 0.4 were tried (0.25 is the only one without a group regression in the gate sets; one-sided windows, a weight of 0.25 to 0.4 and window factors 4 to 12 did not remove them). Without the limit the Balanced gains on synth / synth-2024 / challenging-images / zxing-samples were +3 / +10 / +7 / +19 (fixed 24-sample window: +3 / +3 / +6 / +19). With it, relative to the base tree: zxing-samples Fast 245 -> 248, Balanced 292 -> 313, Exhaustive 297 -> 319 (mostly EAN-13, UPC-A and UPC-E; per-symbology counts of the final tree are in BASELINES.md); challenging-images Balanced 395 -> 400, Exhaustive 406 -> 413; synth Balanced 280 -> 283, Exhaustive 281 -> 284; synth-2024 Balanced 808 -> 816, Exhaustive 811 -> 819. The prototype measured +21 on zxing-samples at Balanced (291 -> 312).
- **Q** (rank 3): a quiet zone that reaches the profile border by at least 1 sample is open for EAN/UPC, Code 128 and Code 39 (`Runs::open_before` / `open_after`, `space_before_open`, `common::quiet_before_open`). Code 39 border reads keep their measured margin for `short_quiet`, so a border margin under 7 narrow widths still needs one more agreeing line. EAN/UPC and Code 128 reads carry a check character (`checksum_verified`), from which the aggregator already waives the extra line, so no extra-line rule applies to them (the add-on's end margin stays closed). ITF is deliberately left alone: its start pattern also occurs inside a longer symbol and a truncated symbol with a stop pattern is a valid shorter code (unit test `a_symbol_cut_by_the_border_is_not_read_shorter` fails for ITF when the border is opened). zxing-samples Fast 248 -> 256, Balanced 313 -> 321, Exhaustive 319 -> 327 (EAN-8 +3, Code 128 +5 at every effort); no change on synth, synth-2024, challenging-images or skewed-datamatrix; video Fast 100 -> 101, tracker +1. Code 39 gains nothing (the prototype had +1): `code39-1/1.png` starts with a bar at pixel 0, which leaves no margin to measure, and the other candidate stays out of reach for other reasons (not investigated).

Combined (rows 1-5): 375/470 Balanced, 378 Exhaustive, 0 FP, against 396 for zxing-cpp.

## 3. What was measured, per class

**U (up-scale).** Black-box transforms (2x/3x/4x Catmull-Rom, nearest 2x, +/- padding) were applied to the 114 images and read with Exhaustive. Some up-scale transform recovers QR 14 (`#709`, `16`, `32`, qrcode-3/04, 07, 14, 40, qrcode-4/04, 12, 41, qrcode-5/16-19) and DM 6 (`mod-size-1`, `16.webp`, `dm-3`, `dm-7`, `abcd-144x8`, `ultrakain`); `qrcode-2/#940.png` (v5 in 41 px, 2 px margin) is not recovered by any transform. Two groups: (a) modules <= 1 px or <= 2.3 px with no quiet zone: `qrcode-5/16-19` (v40, 1.0 px/module, 177x177), `qrcode-2/32.png` (v12, 65 px), `datamatrix-1/mod-size-1.png` (0.9 px), `abcd-144x8.png` (2.0 px), `code128-1/6.png`, `itf-1/1.webp` (1.0 px); (b) 3-6 px modules on blurred, pre-binarised or photographed sheets (qrcode-3, qrcode-4), where the cause is not module size. For (b) the `LocalBinarizer` tile/window was swept (block 4-16, window 1-4): QR changed by at most 2, so the binarizer is not the lever; interpolation smoothing and finer sub-pixel sampling are the remaining explanations (not isolated).

**E (edge relocation).** For 17 images every digit matches but >= 2 are "weak" (margin < 0.45) or only one line reads. `upca-4/7.webp` is clean but its narrow bars never exceed ~25 % darkness; a global threshold (Otsu transform) reads it, as it does 9 more images; the blended reference gives the same effect inside the decoder.

**Q (quiet zone).** The decoders compare the margin up to the profile start with 4.5/5.5/6.5 (EAN-13, EAN-8, UPC-E) / 5 (Code 128) / 7 (Code 39) / 10 (ITF) modules. Images such as `code39-1/1.png` start and end with bars at pixel 0, `ean8-1/5.webp` has 3.9 modules, `itf-1/3.webp` 8-11 modules. zxing-cpp: reads every one of these when cropped to the bare symbol (0 margin) and needs real clearance only for a neighbouring object.

**G.** 13 images (EAN-13 7, UPC-A 4, UPC-E 2) have edge rise of 2-3 modules (3.5 px bars with 7 px rise), glare, or an add-on 1-2.5 modules away. `ean13-1/37.webp`: 357 plausible starts, 427 calibration rejections (312 guard drift, 115 guard tolerance), 153 reach digit matching and none passes. Of the 13, the 3x retry recovers `upca-2/39` and `upca-extension-1/18`; the other 11 stay unread.

**Q3 (QR sampling drift).** Sampled grids compared with zxing's re-rendered symbol: qrcode-3/03 0.5 %, 14 1.2 %, 07 1.6 %; qrcode-4/12 1.3 %, 41 1.6 %, 04 2.0 %, 11 2.0 %, 07 3.7 %; `#709` 1.5 %. The wrong modules form diagonal chains (local drift), not noise.

**D4.** 2x2-region DM grids vs ideal: #1063 3.6 %, #1072-2 6.4 %, issue1072-left 7.3 %, #1088 9.7 %, dm-f 12 %, #669-2 16 %, dm-2x2-a 20 %. In #1063 and #1088 the errors sit in the 6-8 left columns and in the left regions only.

**D1.** `abcd-36x16.png`: codewords 98..123, 129, 0, 150, 45, 195, 90: the first pad is 0 instead of the 253-state value, RS passes with 0 corrections, grid has 0/576 module errors. `144x144_wrong.png`: grid 0/20736 errors; RS blocks 8 and 9 validate only when the ECC starts at codeword 1550 (10 x 155 data first), the standard 156/155 layout validates nothing. `readerinit.png` carries Reader Programming (rejected by design) and is 1.9 px/module.

## 4. Per-symbology tables

Columns: px/mod is the estimate from zxing's quad (QR/DM: side / dimension; EAN/UPC: length / 95 / 67 / 51; Code 128/39 and ITF: percentile of run widths, rough). "blur" is the 20-80 % edge rise divided by px/mod (>= 3 is unreliable for tiny symbols). QZ is the margin to the image border along the symbol axis in modules of that estimate; the cause text quotes the decoder's own module estimate. Rotation is zxing's orientation in degrees. "reads after" lists option switches (E edge relocation, Q border quiet zone, L length/Full ASCII policy, A single-line) that make Balanced read it alone, then the pre-transforms that make Exhaustive read it. The last column says whether the combined prototype reads it in Exhaustive.


#### QR Code

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | dim | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| qrcode-2/#709.webp | 234x343 | 4.4 | 1.4 | 169-251 | 0 | 37 | RS (20/1369 modules wrong) | light symbol on colour gradient, low contrast /  grid is nearly right but RS fails /  binarisation/sampling noise | up-scale,otsu,stretch,smooth/down | Y |
| qrcode-2/#940.png | 41x41 | 1.0 | 5.0 | 0-255 | 0 | 37 | finder triplets (none) | 1.0 px/module, v5 in a 41 px image (2 px margin): finder module 1.0 px, no triplet passes | - | n |
| qrcode-2/16.webp | 260x260 | 1.9 | 1.0 | 0-255 | 0 | 33 | containment filter | nested QR: the labelled inner symbol (v4, 1.9 px/mod) is inside the decoded outer quad, its finders are dropped (BR_NOSKIP experiment decodes both) | up-scale,pad | Y |
| qrcode-2/30a.webp | 350x350 | 7.4 | 0.2 | 1-255 | -26 | 25 | triplet gate | strong projective skew: finder modules 2.8-7.3 px (ratio 2.6 > 1.6 limit), 0 plausible grids | - | n |
| qrcode-2/32.png | 65x65 | 1.0 | 4.0 | 0-254 | 0 | 65 | finder triplets (none) | 1.0 px/module (v12 in 65 px), no quiet zone | up-scale,pad | Y |
| qrcode-2/fix-finderpattern-order.webp | 280x216 | 6.3 | 0.2 | 0-255 | -60 | 25 | triplet gate | extreme perspective: finder modules 4.0/6.5/9.6 px, no triplet | - | n |
| qrcode-2/fix-traceline.webp | 336x268 | 7.4 | 0.2 | 28-227 | -43 | 25 | finder detection | extreme perspective on photo: 2-4 finders found (module 6.5 vs 11.5 px), no triplet | - | n |
| qrcode-2/high-res-1.jpg | 350x375 | 2.1 | 2.7 | 0-160 | -1 | 153 | grid plausibility | v34 (153 modules) at 2.1 px/mod on curved paper: 34 finders, 0 plausible grids | - | n |
| qrcode-2/qr-circles-1.png | 258x260 | 8.0 | 0.1 | 0-255 | 0 | 29 | plausibility gates | styled QR (round dots, bull-eye finders, 8 px modules): dim 29 tried but finder-block/timing gates reject it | pad,smooth/down | n |
| qrcode-2/qr-inv-1.webp | 419x403 | 4.2 | 1.0 | 41-254 | -13 | 29 | RS (103/841 wrong) | light-on-dark, top rows clipped by the circular display /  2 finders | - | n |
| qrcode-2/qr-model-1.png | 66x66 | 2.0 | 3.2 | 0-255 | 0 | 25 | format information | QR Model 1: grid sampled with 0/625 module errors, format info (Model 2 BCH) rejected | - | n |
| qrcode-3/03.webp | 159x148 | 3.0 | 0.7 | 0-255 | 1 | 41 | RS (8/1681 wrong) | pre-binarised, jagged, 3.0 px/mod /  sampled grid 0.5% wrong, RS fails | flip/rot | n |
| qrcode-3/04.webp | 190x199 | 3.2 | 0.6 | 0-255 | -16 | 41 | plausibility (0 grids) | blurred skewed, 2.7-3.9 px/mod, black border /  no plausible grid | up-scale,flip/rot | Y |
| qrcode-3/07.webp | 240x240 | 4.4 | 0.6 | 0-255 | -4 | 41 | RS (27/1681 wrong) | pre-binarised, perspective, 3.3-4.1 px/mod /  errors along a diagonal band | up-scale,flip/rot | Y |
| qrcode-3/14.webp | 205x185 | 4.9 | 0.4 | 0-255 | 4 | 29 | RS (10/841 wrong) | pre-binarised, 4.4-6.3 px/mod, perspective | up-scale,pad,flip/rot | Y |
| qrcode-3/23.webp | 240x240 | 2.4 | 1.4 | 29-222 | 1 | 45 | plausibility (0 grids) | v7 photo of a sheet in perspective, 2.1-4.0 px/mod, 8 finders (3 false) | - | n |
| qrcode-3/40.webp | 200x186 | 3.9 | 1.9 | 23-158 | 17 | 33 | plausibility (0 grids) | hand-held curved sheet, blur, 3.6-4.4 px/mod | up-scale,pad,flip/rot | Y |
| qrcode-4/04.webp | 240x240 | 4.9 | 1.5 | 43-243 | -2 | 29 | RS (17/841 wrong) | crumpled paper, blur (rise 7 px), errors in a diagonal band: local geometric drift | up-scale,pad | Y |
| qrcode-4/07.webp | 240x240 | 4.9 | 0.8 | 41-255 | 0 | 29 | RS (31/841 wrong) | same series, 3.7% wrong along diagonal chain | - | n |
| qrcode-4/11.webp | 240x240 | 5.3 | 1.1 | 62-252 | -4 | 29 | RS (17/841 wrong) | same series | flip/rot | n |
| qrcode-4/12.webp | 240x240 | 5.1 | 1.6 | 63-251 | -3 | 29 | RS (11/841 wrong) | same series | up-scale,pad,sharpen | Y |
| qrcode-4/41.webp | 173x152 | 3.4 | 2.1 | 81-252 | 6 | 25 | RS (10/625 wrong) | same series, v2 | pad | n |
| qrcode-5/16.png | 177x177 | 1.0 | 7.0 | 0-255 | 0 | 177 | finder triplets | v40 at 1.0 px/module, no quiet zone (177x177 image) | up-scale,pad | Y |
| qrcode-5/17.png | 177x177 | 1.0 | 7.5 | 0-255 | 0 | 177 | finder triplets | v40 at 1.0 px/module, no quiet zone | up-scale,pad | Y |
| qrcode-5/18.png | 177x177 | 1.0 | 7.0 | 0-255 | 0 | 177 | finder triplets | v40 at 1.0 px/module, no quiet zone | up-scale,pad | Y |
| qrcode-5/19.png | 177x177 | 1.0 | 6.0 | 0-255 | 0 | 177 | finder triplets | v40 at 1.0 px/module, no quiet zone | up-scale,pad | Y |

#### Data Matrix

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | dim | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| datamatrix-1/144x144_wrong.png | 292x292 | 2.0 | 1.0 | 0-255 | 0 | 144x144 | RS (block layout) | grid sampled with 0/20736 errors /  RS fails on the standard 144x144 layout (blocks 8,9 validate only with EC starting at cw 1550): non-conformant interleave | - | n |
| datamatrix-1/abcd-36x16.png | 199x97 | 5.0 | 0.3 | 0-255 | 0 | 16x36 | encodation (pad check) | grid perfect, RS ok (0 corrections) /  rejected by the strict 253-state pad check: pads after cw 129 are 0,150,45,195,90 | - | n |
| datamatrix-1/distorted.png | 60x60 | 3.0 | 0.8 | 0-255 | 0 | 16x16 | plausibility (36% wrong) | 16x16 at 3 px/mod, strong shear: sampled grid 92/256 wrong, true size gated out as implausible | - | n |
| datamatrix-1/mod-size-1.png | 12x12 | 0.9 | 6.7 | 0-255 | 0 | 10x10 | detection (no candidate) | 10x10 at 0.9 px/mod in a 12x12 image | up-scale | Y |
| datamatrix-1/readerinit.png | 22x22 | 1.9 | 1.8 | 0-255 | 0 | 10x10 | precheck + policy | 10x10 at 1.9 px/mod in 22x22 /  no candidate passes the L precheck /  also carries Reader Programming (234), rejected by design | - | n |
| datamatrix-2/16.webp | 240x240 | 2.3 | 2.4 | 51-156 | -2 | 40x40 | size hypothesis | 40x40 at 2.3 px/mod, low contrast photo /  true size never hypothesised | up-scale,pad,sharpen,stretch,flip/rot | Y |
| datamatrix-3/#794.png | 98x123 | 3.2 | 1.0 | 0-255 | 4 | 26x26 | size hypothesis | 26x26 at 3.2 px/mod, eroded/jagged print, fragmented L and clock /  true size never hypothesised | - | n |
| datamatrix-3/dm-2x2-#1063.webp | 145x148 | 2.9 | 0.4 | 0-255 | 1 | 44x44 | RS (69/1936 wrong) | 44x44 2x2 regions, 2.9 px/mod, binarised /  errors confined to the left 6-8 columns (sampling drift) | - | n |
| datamatrix-3/dm-2x2-#1072-2.png | 261x256 | 5.0 | 0.2 | 0-255 | 0 | 48x48 | RS (147/2304 wrong) | 48x48, 5 px/mod /  errors in left half | - | n |
| datamatrix-3/dm-2x2-#1088.webp | 376x380 | 6.8 | 0.1 | 0-255 | -3 | 48x48 | RS (224/2304 wrong) | 48x48, 6.8 px/mod, rotated -3 deg /  errors mostly left half | - | n |
| datamatrix-3/dm-2x2-#669-2.png | 223x224 | 3.9 | 0.4 | 14-252 | -6 | 48x48 | RS (370/2304 wrong) | 48x48, 3.9 px/mod, rotated -6 deg /  16% wrong | - | n |
| datamatrix-3/dm-2x2-a.webp | 499x499 | 9.1 | 0.1 | 0-255 | 5 | 48x48 | RS (472/2304 wrong) | 48x48, 9 px/mod /  thick strokes (ink spread), 20% wrong /  plausibility rejects 4 of its true-size grids | - | n |
| datamatrix-3/dm-3.webp | 133x76 | 3.4 | 1.5 | 10-252 | -10 | 12x26 | L/clock analysis | 12x26 photo, 3.4 px/mod, blurred, rotated -10 deg /  true size never reached | up-scale,pad | Y |
| datamatrix-3/dm-7.webp | 212x163 | 5.6 | 0.7 | 33-192 | 21 | 10x10 | L/clock analysis | 10x10 on dotted metal, 5.6 px/mod, rot 21 deg /  all 11 candidates fail layout analysis | up-scale,pad | Y |
| datamatrix-3/dm-f.webp | 151x147 | 3.0 | 2.7 | 6-213 | -16 | 26x26 | RS (81/676 wrong) | 26x26 photo in perspective, 3 px/mod, blur /  12% wrong | sharpen | n |
| datamatrix-4/abcd-144x8.png | 292x20 | 2.0 | 1.8 | 0-255 | 0 | 8x144 | RS (106/1152 wrong) | 8x144 at 2.0 px/mod, 20 px high image /  9.2% wrong when sampled at the module centre | up-scale,pad,flip/rot | Y |
| datamatrix-4/dm-e-d-c-ultrakain.webp | 484x80 | 3.3 | 1.5 | 0-236 | 0 | 8x64 | L/clock analysis | 8x64 on a syringe photo (80 px high), 3.3 px/mod /  6 candidates fail layout | up-scale | Y |
| datamatrix-5/issue1072-left.webp | 540x553 | 10.0 | 0.1 | 0-255 | 0 | 48x48 | RS (168/2304 wrong) | same symbol as #1072-2 at 10 px/mod /  7% wrong, errors in left half | smooth/down | n |
| datamatrix-5/issue794-12-1.webp | 190x210 | 3.1 | 2.6 | 28-190 | 4 | 26x26 | size hypothesis | 26x26 photo (same as #794), 3.1 px/mod /  true size never hypothesised | - | n |

#### EAN-13

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | QZ L/R (mod, to image edge) | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| ean13-1/#526.webp | 341x11 | 3.0 | 1.4 | 0-255 | 0 | 12.0/6.6 | right quiet zone | 341x11 strip /  a black block follows the end guard at ~2.6 modules (needs 4.5 /  zxing needs ~3 at a real neighbour) /  all 12 digits read, 0 weak | - | n |
| ean13-1/37.webp | 384x225 | 2.8 | 4.2 | 121-232 | 0 | 27.2/17.8 | guard calibration | blurred photo (edge rise ~2 modules) /  357 plausible starts, 312 rejected by guard drift, 115 by guard tolerance | - | n |
| ean13-2/04.webp | 337x238 | 2.3 | 5.2 | 10-126 | 0 | 24.9/25.3 | guard calibration | blurred ISBN photo, 2 px/mod, edge rise 3 modules /  guard tolerance/drift reject nearly all starts | up-scale,pad,sharpen | n |
| ean13-2/05.webp | 381x227 | 2.6 | 4.5 | 19-153 | 0 | 23.6/29.0 | digit match (weak) | all 12 digits matched but 2 are weak (margin<0.45) /  the rule allows 1 | opts E / pad,sharpen,stretch | Y |
| ean13-2/09.webp | 347x251 | 2.3 | 5.2 | 20-119 | 0 | 18.8/35.0 | guard calibration | blurred, shaded /  add-on bars touch the right end (qa 0.7) /  only 4/12 digits read | - | n |
| ean13-4/02.webp | 240x240 | 1.5 | 7.3 | 22-151 | 0 | 33.2/28.6 | digit match (weak) | 12/12 digits, 2 weak, blurred 2.5 px/mod | opts E / pad,otsu,sharpen,stretch,flip/rot | Y |
| ean13-4/04.webp | 240x240 | 1.7 | 6.8 | 26-142 | 0 | 24.7/25.3 | digit match (weak) | 12/12 digits, 2 weak /  glare patch /  Exhaustive reads it once edge relocation is fixed | sharpen | Y |
| ean13-4/05.webp | 237x103 | 1.5 | 8.1 | 31-134 | 0 | 28.0/31.9 | guard calibration | 237x103 crop, 2.5 px/mod, heavy blur/glare /  add-on bars adjacent on the right /  2/12 digits | - | n |
| ean13-4/06.webp | 240x240 | 1.6 | 6.8 | 24-137 | 0 | 23.4/30.2 | line agreement | 12/12 digits, 1 weak: one line reads it, 2 needed | opts E / up-scale,pad,sharpen,stretch,flip/rot | Y |
| ean13-4/07.webp | 217x43 | 1.6 | 8.2 | 35-136 | 0 | 23.1/21.8 | guard calibration | 217x43 strip, 2.5 px/mod, blur, glare | - | n |
| ean13-4/12.webp | 240x240 | 1.9 | 7.2 | 10-109 | 0 | 19.6/13.3 | line agreement | one reading line only (OK once under proto), others fail guard | otsu | Y |
| ean13-4/13.webp | 240x240 | 1.8 | 7.7 | 26-122 | 0 | 17.7/25.1 | guard calibration | rotated ~10 deg, perspective, 3.5 px/mod, blur | - | n |
| ean13-4/15.webp | 240x101 | 1.6 | 8.2 | 1-34 | 0 | 31.7/26.0 | guard calibration / digits | very dark crop (p5-p95 = 1-34 levels), 10/12 digits, right quiet 0.5 modules (add-on) | - | n |
| ean13-extension-1/38.webp | 542x226 | 2.8 | 3.2 | 58-195 | 0 | 36.2/65.5 | digit match | misread digit (7 read as 1: checksum fail 9180201752847) at 1.8 px/mod with add-on /  17 parity failures | opts E / up-scale,pad,stretch | Y |

#### UPC-A

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | QZ L/R (mod, to image edge) | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| upca-2/07.webp | 198x118 | 1.3 | 7.3 | 15-187 | 0 | 27.7/26.2 | digit match (weak) | 12/12 digits, 2 weak, blur | opts E / up-scale,pad,sharpen,stretch,flip/rot | Y |
| upca-2/16.webp | 172x111 | 1.5 | 6.3 | 36-193 | 0 | 13.0/10.3 | line agreement | digits fine (weak 0), only 1 line decodes /  curved bars on a jar | opts EA / up-scale,pad,sharpen,stretch,flip/rot | Y |
| upca-2/25.webp | 164x102 | 1.4 | 8.0 | 69-199 | 0 | 17.4/7.3 | digit match (weak) | 12/12 digits, 2 weak /  rotated bars | opts E / up-scale,pad,otsu,sharpen,stretch | Y |
| upca-2/31.webp | 240x74 | 1.5 | 7.4 | 28-185 | 0 | 48.7/21.9 | guard calibration | 240x74 crop, add-on/edge at 1.3 modules on the right, blur | - | n |
| upca-2/35.webp | 203x78 | 1.4 | 6.6 | 26-212 | 0 | 24.3/30.9 | digit match (weak) | 12/12 digits, 3 weak | opts E / up-scale,pad,sharpen,flip/rot | Y |
| upca-2/39.webp | 240x54 | 1.9 | 6.1 | 66-235 | 0 | 9.9/20.2 | guard calibration | 240x54 strip with text touching the right end /  3.5 px/mod | up-scale | Y |
| upca-4/13.webp | 352x219 | 2.3 | 5.4 | 85-191 | 0 | 30.1/28.8 | digit match (weak) | 12/12 digits, 2 weak /  glare | opts E / up-scale,otsu | Y |
| upca-4/14.webp | 270x198 | 2.0 | 4.2 | 47-182 | 0 | 19.7/18.7 | digit match (weak) | 12/12 digits matched in 23 lines but 2-8 weak each /  needs relocation fix and 1-line acceptance | - | n |
| upca-4/16.webp | 365x107 | 3.1 | 3.7 | 24-145 | 0 | 12.6/10.3 | guard calibration | 365x107 brownish low-contrast, bars 3 px, blur | - | n |
| upca-4/18.webp | 430x226 | 2.6 | 4.0 | 49-205 | 0 | 34.3/36.6 | digit match (weak) | 12/12 digits, 2 weak /  11 of 14 lines | opts E / up-scale,otsu,stretch | Y |
| upca-4/7.webp | 349x180 | 2.1 | 3.3 | 2-253 | 0 | 33.6/41.9 | digit match | clean image but thin bars (bias -0.5 px = -0.23 mod) /  11/12 digits, last digit cost 1.3-1.56 > 1.3 limit | opts E / otsu | Y |
| upca-5/09.webp | 310x178 | 2.6 | 4.5 | 41-158 | 0 | 13.3/12.9 | digit match | blur+perspective: 5/12 digits | opts E / stretch | Y |
| upca-5/31.webp | 384x193 | 3.3 | 3.7 | 30-153 | 0 | 11.8/9.7 | line agreement | weak 1, only 1 line decodes /  curved label | opts E / up-scale,pad,otsu,sharpen,stretch,flip/rot | Y |
| upca-extension-1/18.webp | 239x152 | 1.6 | 8.0 | 57-173 | 0 | 16.6/42.2 | guard calibration | add-on "00" 2.5 modules after the end guard (qa 0.5-2.5), 7/12 digits | otsu | Y |

#### UPC-E

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | QZ L/R (mod, to image edge) | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| upce-2/03.webp | 201x62 | 1.9 | 6.1 | 21-166 | 0 | 34.7/22.4 | digit match (weak) | 6/6 digits, 2 weak | opts E / otsu,sharpen,flip/rot | Y |
| upce-2/39.webp | 220x85 | 2.2 | 3.4 | 32-199 | 0 | 28.2/19.7 | digit match (weak) | 6/6 digits, 1 weak: reads once | opts E / up-scale,otsu,sharpen,stretch | Y |
| upce-3/05.webp | 240x240 | 2.2 | 4.4 | 120-159 | 0 | 20.8/37.0 | digit match | low contrast (levels 120-159), heavy blur, add-on at ~1.5 modules /  2/6 digits | - | n |
| upce-3/09.webp | 240x240 | 2.3 | 3.6 | 136-166 | 0 | 21.6/31.6 | digit match | same series, 3/6 digits | up-scale,pad | n |

#### EAN-8

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | QZ L/R (mod, to image edge) | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| ean8-1/3.png | 156x119 | 2.0 | 2.0 | 4-255 | 0 | 5.5/6.5 | quiet zone | 156x119 tight crop: left quiet 4.8 modules (<5.5) | opts Q / pad | Y |
| ean8-1/5.webp | 292x261 | 3.7 | 0.6 | 0-255 | 0 | 4.6/8.2 | quiet zone | 292x261 crop: left quiet 3.9 modules (<5.5) | opts Q / pad | Y |
| ean8-1/8.png | 194x120 | 2.6 | 1.2 | 0-128 | 0 | 3.5/5.9 | quiet zone | colour-banded background, right quiet 3.0 modules (<5.5) | opts Q / pad | Y |
| ean8-1/single-line.png | 69x1 | 1.0 | 7.1 | 3-253 | 0 | 2.0/2.0 | line agreement | 69x1 px image: a single scanline, module 1 px | pad | Y |

#### Code 128

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | QZ L/R (mod, to image edge) | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| code128-1/3.webp | 229x79 | 1.8 | 4.1 | 7-255 | 0 | 4.6/3.4 | quiet zone | 229x79 tight crop: quiet 3.9-4.0 modules (<5) /  blurred | opts Q / pad | Y |
| code128-1/5.png | 1061x251 | 6.0 | 0.2 | 0-255 | 0 | 5.2/5.0 | quiet zone | 1061x251 clean render, quiet 4.8 modules (<5) | opts Q / pad | Y |
| code128-1/6.png | 288x62 | 1.0 | 6.0 | 0-255 | 0 | 12.0/12.0 | min module | 1.0 px/module, 288x62 | up-scale,pad | Y |
| code128-2/#444.webp | 393x67 | 3.8 | 1.9 | 9-120 | 0 | 2.9/3.7 | quiet zone | 393x67 crop, grey low contrast (9-120), quiet < 5 | opts Q / pad | Y |
| code128-2/06.webp | 363x145 | 3.0 | 4.0 | 37-176 | 0 | 27.0/13.0 | line agreement | 5 px modules, blur /  1 of the lines decodes | otsu | Y |
| code128-2/28.webp | 580x47 | 2.8 | 3.1 | 45-171 | 0 | 12.0/9.5 | line agreement | rotated ~15 deg, 47 px high strip: 1 line decodes (checksum ok) , 2 needed | opts A / up-scale,pad,sharpen,stretch,flip/rot | Y |
| code128-3/1.png | 390x206 | 4.8 | 0.5 | 67-255 | 0 | 3.9/2.5 | quiet zone | scan-like bars, quiet 4.0-4.2 modules (<5) | opts Q / pad | Y |
| code128-3/2.webp | 410x226 | 4.2 | 0.6 | 100-255 | 0 | 8.7/5.2 | quiet zone | right quiet 4.3-4.5 modules (<5), 89 stop patterns rejected | opts Q / pad | Y |

#### Code 39

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | QZ L/R (mod, to image edge) | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| code39-1/1.png | 1470x347 | 9.0 | 0.2 | 0-255 | 0 | 0.1/0.2 | quiet zone / border | 1470x347 render cropped tight: first and last bars touch the image border (0-1 px margin) | pad | n |
| code39-1/2.webp | 364x78 | 1.0 | 7.0 | 0-255 | 0 | 7.0/8.0 | min module | 364x78, ~1.3 px/module, blur | pad | Y |
| code39-1/4.png | 300x300 | 3.0 | 1.2 | 0-255 | 0 | 4.7/6.3 | quiet zone | 300x300 render, quiet 4.3-5.3 modules (<7) | opts Q / pad | Y |
| code39-2/1.png | 420x87 | 1.0 | 7.0 | 0-255 | 0 | 4.0/3.0 | Full ASCII + min module | 420x87, ~1.0-1.3 px/mod, Extended text needs full ASCII | pad | Y |
| code39-2/2.png | 377x181 | 3.0 | 0.8 | 0-248 | 0 | 1.3/0.3 | Full ASCII + border | "12ab" needs Full ASCII /  stop bar clipped by the right image border (no closing edge) | pad | n |
| code39-2/wide.png | 418x31 | 2.0 | 1.5 | 0-255 | 0 | 6.0/5.5 | wide:narrow ratio + Full ASCII | 418x31 /  wide/narrow ~3.5-4 (limit 3.6) /  with ratio<=5 and Full ASCII it reads | - | n |

#### ITF

| file | WxH | px/mod | blur (rise/mod) | contrast p5-p95 | rot | QZ L/R (mod, to image edge) | fails at | cause | reads after (option toggles /  pre-transforms) | combined prototype (Exh) |
|---|---|---|---|---|---|---|---|---|---|---|
| itf-1/#220.png | 8x518 | 3.0 | 1.3 | 0-255 | -90 | 60.3/15.0 | length policy | 10 digits, vertical 8x518 strip /  default lengths [14] | opts L / up-scale,pad,sharpen,stretch,smooth/down,flip/rot | Y |
| itf-1/#853.png | 143x53 | 3.0 | 0.7 | 0-255 | 0 | 1.3/1.7 | length policy (4 digits) | "1515" /  default min_length 6 and lengths [14] /  quiet zone ~4 modules | - | n |
| itf-1/1-slanted.png | 330x13 | 2.0 | 3.1 | 0-255 | 0 | 7.5/7.5 | pair read | ITF-14 on a sheared 330x13 strip /  pair reads fail after 2 groups | - | n |
| itf-1/1.webp | 165x109 | 1.0 | 6.5 | 0-255 | 0 | 17.0/15.0 | min module | ITF-14 at 1.0 px/module /  up-scale x2 reads it | up-scale,pad | Y |
| itf-1/13.webp | 238x120 | 2.0 | 4.5 | 81-170 | 0 | 17.5/22.0 | length policy | 10 digits | opts L / up-scale,pad,sharpen,stretch,flip/rot | Y |
| itf-1/17.webp | 305x113 | 1.5 | 5.2 | 30-255 | 0 | 52.7/52.7 | length policy | 10 digits | up-scale,pad,sharpen,stretch,flip/rot | Y |
| itf-1/3.webp | 250x85 | 2.3 | 3.3 | 50-157 | 0 | 10.9/14.3 | length policy + quiet zone | 10 digits /  quiet zone < 10 modules (image border) | pad | Y |
| itf-1/5.webp | 250x93 | 2.5 | 3.5 | 60-134 | 0 | 8.8/12.0 | length policy + quiet zone | 10 digits /  quiet zone | pad | Y |
| itf-1/6.webp | 250x66 | 2.5 | 5.1 | 75-164 | 0 | 8.2/10.2 | length policy + quiet zone | 10 digits /  quiet zone | pad | Y |
| itf-1/7.webp | 250x89 | 2.5 | 3.7 | 72-143 | 0 | 10.8/10.0 | length policy + quiet zone | 10 digits /  white scan lines through bars /  quiet zone | up-scale,pad | Y |
| itf-1/8.webp | 239x130 | 2.2 | 3.1 | 82-173 | 0 | 12.4/16.9 | length policy | 10 digits | opts L / up-scale,pad,otsu,sharpen,stretch,flip/rot | Y |
| itf-1/9.webp | 239x102 | 2.2 | 3.6 | 79-163 | 0 | 15.1/12.9 | length policy | 10 digits | opts L / up-scale,pad,otsu,sharpen,stretch,flip/rot | Y |
| itf-1/with-tail.png | 256x4 | 2.0 | 3.7 | 1-255 | 0 | 8.0/46.5 | line agreement + quiet zone | 8 digits, 256x4 px image: 4 scanlines, ITF needs 3 agreeing /  quiet zone | pad | Y |
| itf-2/01.webp | 240x240 | 2.5 | 3.3 | 15-156 | 0 | 16.8/27.2 | length policy | 6 digits | opts L / up-scale,pad,otsu,sharpen,stretch,flip/rot | Y |
| itf-2/04.webp | 240x240 | 3.0 | 2.9 | 24-186 | 0 | 10.5/17.0 | length policy | 6 digits, rotated | opts L / up-scale,pad,otsu,sharpen,stretch,smooth/down,flip/rot | Y |
| itf-2/06.webp | 240x240 | 3.2 | 2.2 | 16-170 | 0 | 18.5/7.7 | length policy + quiet zone | 6 digits /  quiet zone | pad | Y |
| itf-2/07.webp | 240x240 | 2.2 | 4.3 | 29-171 | 0 | 25.8/32.0 | length policy | 6 digits | opts L / up-scale,pad,otsu,sharpen,stretch,flip/rot | Y |
| itf-2/08.webp | 240x240 | 3.0 | 2.4 | 43-169 | 0 | 9.0/20.7 | length policy + quiet zone | 6 digits /  quiet zone | pad | Y |
| itf-2/13.webp | 240x240 | 3.0 | 2.6 | 43-170 | 0 | 18.7/11.0 | length policy + quiet zone | 6 digits /  quiet zone | up-scale,pad,otsu,sharpen | Y |

## 5. Remaining after the combined prototype (44)

QR (13): `qrcode-2/#940.png` (41 px, 1 px/module, 2 px margin: no transform reads it), `30a`, `fix-finderpattern-order`, `fix-traceline`, `high-res-1.jpg` (projective/curved), `qr-circles-1.png`, `qr-inv-1.webp`, `qr-model-1.png`, qrcode-3/03, qrcode-3/23, qrcode-4/07, 11, 41 (drift).
Data Matrix (13): `144x144_wrong`, `abcd-36x16`, `distorted`, `readerinit`, `#794`, `issue794-12-1`, dm-2x2-#1063, #1072-2, #1088, #669-2, dm-2x2-a, `dm-f`, `issue1072-left`.
EAN-13 (8): ean13-1/#526, 1/37, 2/04, 2/09, 4/05, 4/07, 4/13, 4/15. UPC-A (3): upca-2/31, 4/14, 4/16. UPC-E (2): upce-3/05, 3/09. Code 39 (3): code39-1/1 (bars touch both image borders), code39-2/2 (stop bar clipped by the border), code39-2/wide (wide:narrow > 3.6; reads with ratio <= 5 and Full ASCII). ITF (2): `#853.png` (4 digits), `1-slanted.png`.
The Code 39 border cases need a virtual edge at the profile border (padding the profile with 24 background samples reads 2 more but produced one FP in one configuration, so it is not recommended without more evidence).

## 6. Suspected GT errors

None found. Checks done against printed text: EAN/UPC human-readable digits match the labels in ean13-1/37, ean13-2/04/05/09, ean13-4/*, upca-2/07/25/31/35, upca-4/7/13/14/18, upca-5/09/31, upce-3/05 (UPC-E 0 496580 2 expands to the labelled 0049000006582); ITF itf-1/5 vs 6 differ only in the printed date digit as labelled; ISBN check digits of ean13-2 (9780804816632) and ean13-4 (9780441014989) are valid. `itf-2/*` prints "07042" and the bars encode 070429 (zxing and breader Exhaustive with lengths open agree), consistent with a check digit not printed. Not verifiable by eye: upca-4/16, upca-2/39 (no digits visible). Images zxing-cpp does not read but breader does (13) are out of scope here.

## 7. Notes on zxing-cpp behaviour (black box)

- Quiet zone: reads EAN-13, EAN-8, Code 128/39 and ITF cropped to the bare symbol; with a real black neighbour it needs >= 3 modules on the right and >= 6 on the left of an EAN-13 (synthetic render, 2 and 3 px modules).
- Its 1D positions are thin lines (a few px high) and it recovers images where only one scanline decodes, so it appears to have no multi-line agreement gate.
- Reads Code 39 Full ASCII and ITF of any length >= 4 by default; reads Model 1 QR and Reader Programming DM.
- Reads nested QR symbols (both).

## 8. Reproduction

Scratch tools (not in the repo): `dump` (reads a tree, JSON lines), `dump2` (transform grid: up/pad/flip/rot/blur/otsu/sharpen), `dump3` (single image with `BR_DBG=1` stage events), `dump4`/`dump5` (whole set with switches), `ws/` (workspace copy with the switches: `BR_EDGE=2 BR_WIN=24` edge blend, `BR_QZ=1` open border quiet zone, `BR_AGREE=1`, `BR_ITFANY`, `BR_C39FULL`, `BR_NOSKIP`, `BR_C39R`, `BR_GT`/`BR_DRIFT`, `UPFB=3 UPMAX=600` retry in the bench engine). Authoritative numbers come from the workspace copy of `tools/bench` (`--effort balanced`), which scores like the repo bench.
