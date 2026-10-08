# Failure triage

Date 2026-10-08, commit 32c3ec0 (Balanced unless noted). Investigation only: measurements were taken on a scratch copy of the workspace with temporary debug output and env-var switches; nothing in `crates/` or `tools/` was changed. "zxing-cpp" = 3.1.1 Python package, default options, used as a black-box reader.

Coordinates of ground-truth quads (`annotations.json`) are in EXIF-corrected orientation; 10 of the 68 images carry an EXIF orientation (`barcode-with-shadow-1`, `batch-scanning-1/2`, `custom-scan-parameters-4`, `multiple-symbologies-multiple-barcodes-9`, `retail-1/2/3`, `single-symbology-multiple-barcodes-1`, `tiny-barcodes-4`), but the bench decodes the raw pixels. Matching is by text only, so this is harmless today; any position-based check must apply the orientation.

## 1. Summary

| Area | Misses | Root-cause classes (symbols) |
|---|---|---|
| Code 39, `batch_code39` video (32 symbols) | 19 (tracker), 21 (Balanced), 23 (Fast) | quiet zone cut by the label-cell separator and rejected by the 7-module rule (18 of 19); 1 quiet zone of 2.1 modules |
| QR, challenging-images (234) | 9 | curved surface (1), damaged or cropped finders (3), blur or low contrast, no finder found (1), 2.5 px modules (2), v40 at 3.1 px (1), embossed surface (1) |
| Code 128, challenging-images (60) | 26 | symbols at 30/60 degrees missed by the 22.5 degree plan (8), thin bars at 2 px modules (8), identical-text merge in the aggregator (1 now, 8 more in Exhaustive), <= 1.5 px/module (6), weak start character (2), glare (1) |

Experiments on the scratch copy (each a one-constant change, 0 false positives on challenging-images, synth, synth-2024 and zxing-samples):

| Change | Result |
|---|---|
| Code 39 `QUIET` 7 -> 4 modules | `batch_code39`: Fast 9 -> 26, Balanced 11 -> 29, tracker 13 -> 30 of 32. challenging-images unchanged, zxing-samples Code 39 1 -> 2 |
| Code 39 `MIN_SEPARATION` 1.3 -> 1.1 | no change (not a cause) |
| Aggregator link distance `< max(len)` -> `< 0.2 * max(len)` | challenging-images Exhaustive 388 -> 404, Balanced 392 -> 393 |
| Code 128 allow a weak start character (counts as the one allowed weak character) | challenging-images Code 128 34 -> 36, total 392 -> 394 |

## 2. Code 39 in `batch_code39_1080p` (1080x1920 portrait, 244 frames, 32 labels on one sheet)

The sheet is a 2x16 grid of cells; every label is visible in every frame. The ground truth has no positions, so slots were located per frame by edge-density projection (16 bands per column, top to bottom) and mapped to the printed text. zxing-cpp reads 28 of 32 over the clip (not 003, 004, 005, 006, 021).

Measured on 5 frames (median): narrow 2.1 px, wide 4.1-4.2 px (wide:narrow 2.0:1, which is the minimum the standard allows and is why every character has a small wide/narrow gap), symbol width 445-500 px (about 16 characters, 202 modules), bar height 37-44 px (rows 004 and 056 at the bottom: 28 px), edge rise (10-90 %) 1.2-1.5 px, contrast 160-195 of 255, tilt 0 to 2 deg (bottom rows 3.6 deg), inter-character gap 2.0-2.4 px (1 narrow).

Stage where each frame-level attempt dies, counted on 41 sampled frames with horizontal Exhaustive lines (`stop_quiet` / `start_quiet` = quiet zone shorter than 7 narrow widths). Scanlines do cross every symbol; start pattern and all 14 characters decode on clean lines; the quiet check is the first and systematic rejection:

| Group | Symbols | Quiet zone on the separator side (narrow widths, median over rejections) | Failing stage |
|---|---|---|---|
| Left column, right end next to the cell separator (about 5-8 px gap) | 001 (6.5), 002 (4.6), 004 (4.1), 005 (5.2), 006 (4.3-6.7), 007 (5.8), 008 (5.5), 009 (4.1), 010 (4.6), 011 (4.6), 012 (5.6), 013 (6.4), 014 (5.4), 100 (5.4), 056 (4.1-6.7) | 4.1-6.7 | `QUIET = 7.0` in `code39.rs::read_symbol` (stop side) / `start_of` (when read from the right) |
| Right column, left end next to the separator | 017 (5.1), 018 (5.1), 020, 025, 003, 057 | 5.1 where measured; 003, 020, 025, 057 stop earlier at `QUIET_SCREEN` (0.33 of a character width, about 8 px) | `start_of` quiet screen / quiet check |
| Separator closer than the rest | 021 | 2.1 | quiet check, still fails at `QUIET = 4` |
| Read already | 015, 016, 019, 022-028, 030 | wider gap (separator 10+ px away) | none |

Fast, Balanced and the tracker differ only by which of the borderline symbols a given frame happens to hit (Fast 9, Balanced 11, tracker 13 before the change). No edge-detection, run-quantization or checksum failure is involved: Code 39 here has no check digit, and the character table and gap checks passed on all rows that reached them. A `MIN_SEPARATION` of 1.1 changes nothing.

After `QUIET = 4` the remaining misses are 021 (quiet 2.1 modules) and 057 (bottom-right label, narrow 1.9 px, start quiet screen fails), plus 003 in Balanced.

Spec note: ISO/IEC 16388 asks for 10 modules, which these labels violate (4.1-6.7). zxing-cpp tolerates them. This is a printing/layout defect of the dataset sheet, not a capture problem.

## 3. QR misses in challenging-images (9 of 234)

"Finders" lists what `find_finders` reported near the symbol in Balanced. Module size = GT side / dimension. All numbers measured on the original pixels.

| File | Symbol | Version, px/module | Finder / triplet | Decode result | Cause class | zxing-cpp |
|---|---|---|---|---|---|---|
| barcode-with-shadow-2 | `HTTPS://NUTS.COM/QR/retail_piece/4507...` | v4 (33), about 9 px | 3 finders (module 8.9-9.2), triplet and dim 33 found | only the affine variant ran (alignment search fails); `ErrorCorrection` | Curved or crumpled surface: rows and columns bend nonlinearly; right edge is cut by the image border (GT x reaches 586 in a 585 px image); shadow gradient | yes |
| batch-scanning-2 | `A-41` | v1 (21), 7.5 px | 2 of 3 finders found, no triplet | not reached | Third finder destroyed by a peeled label chip (white patch inside the finder); some modules faded | no |
| batch-scanning-2 | `A-47` | v1, 7.6 px | no finder candidate at all in the area | not reached | Engraved metal disc: blurred, low-contrast finders, glare | yes |
| custom-scan-parameters-4 (JPEG data in a .png) | 1862-char text | v40 (177), 3.1 px | correct triplet; the timing-pattern hint says 161 (wrong); dozens of false finders from alignment patterns | at dim 177 the alignment fit (`Refined`) returns no mapper, `Corner` none, plain affine fails; wrong dims 161/169/173 also tried (`ntried < 4`) | Very high version at about 3 px/module: alignment-pattern fit fails, drift over 177 modules, budget of 4 dims spent on the wrong hint | yes |
| custom-scan-parameters-9 | `http://cokeurl.com/q/2017-00776` | v3 (29), about 20 px | correct triplet, module 18-21 px | `Format` and `ErrorCorrection` at dim 29 for affine and corner variants | Embossed, engraved plastic with strong perspective: thick dark outlines around raised modules, grid does not follow the surface | no |
| incomplete-2 | `dynamsoft.com` | v3 (29), about 6 px | only the top-right finder found | not reached | Deliberately incomplete: top-left finder has a 2 px left ring instead of 7 px (cropped), bottom-left finder missing; symbol touches the image border (no quiet zone) | no |
| single-symbology-multiple-barcodes-2 | `A-41` | v1, 2.9 px | one finder with count 3 | not reached | Low resolution plus the same peeled corner as in batch-scanning-2 | no |
| single-symbology-multiple-barcodes-2 | `A-0040-Z` | v1, 2.5 px | triplet found, dim 21 | `ErrorCorrection` | 2.5 px modules: sampling from the binarized grid misreads modules; JPEG blur | yes |
| single-symbology-multiple-barcodes-2 | `R-0060-B` | v1, 2.5 px | triplet found, dim 21 | `ErrorCorrection` | same | no |

## 4. Code 128 misses in challenging-images (26 of 60)

px/module = GT length divided by the module count implied by the text (subset C for even digit strings). Edge rise = 10-90 % width on a profile along the GT axis.

| File | Symbols missed | Measured | Failing stage | Cause class | zxing-cpp |
|---|---|---|---|---|---|
| multiple-symbologies-multiple-barcodes-2 | 8 of 16 `Dynamsoft` (the 30/60/120/150 deg ones) | 291 x 30 px, 2.2 px/module, clean synthetic render | no start pattern found: scanlines never cross the full length | Short symbol (height/length 0.10) tolerates about 5.9 deg of misalignment; Balanced directions are 22.5 deg apart (30 and 60 deg are 7.5 deg off). Balanced reads the 4 axis and 4 diagonal ones; Exhaustive (15 deg step) reads 15 once the aggregator merge below is removed | 4 of 16 |
| same file, Exhaustive | 15 of 16 lost, one giant symbol reported | | aggregator | Identical texts: `group` links hits whose centers are closer than the longer segment; neighbours in the pinwheel are about 215 px apart, so all 16 merge into one group with one huge quad (Exhaustive loses these 15 symbols; this is part of why Exhaustive reads fewer than Balanced in BASELINES.md) | |
| single-symbology-multiple-barcodes-3 | 8 of 13 (`132323956030`, `132325313030`, `132325430030`, `132325661030`, `132325903030`, `132327524030`, `132327853030`, `132328983030`) | vertical, 50 x 200 px, 2.0 px/module, bars on a cylinder | `start_pattern` rarely satisfied; edges of 1 px thin bars merged or missing | Extreme negative print gain: 1-module bars render about 1 px and gray, spaces swell; curved tube. Exhaustive adds 2 | 1 of the 8 (`132325661030`) |
| single-symbology-multiple-barcodes-1 | `D6OKCT001674`, `90NK0011-M01780` | 1.44 and 1.52 px/module, contrast 126-135 | start or character match | Resolution below 1.6 px/module with low contrast | no |
| multiple-symbologies-multiple-barcodes-1 | `1S82B8002UUXPF2G3TFT`, `PF2G3TFTPF9XB0C13005`, `SPF2G3TFT` | 1.38, 1.39, 1.40 px/module, edge rise 1.4 px | start or character match | Resolution about 1.4 px/module (edge rise equals the module width) | only `SPF2G3TFT` |
| healthcare-1 | `Jane-F-2023-0304-23` | 244 modules in 245 px = 1.0 px/module, 17 deg tilt | not decodable | Below the sampling limit | no |
| single-symbology-multiple-barcodes-7 | `99HCE030-00`, `HT631F228585` | vertical, 2.5 and 2.2 px/module, contrast 183 | `start_of`: start character found with margin below 0.7 (`weak`) on most lines and rejected (`start.weak`) | Strict start character margin, the rest of the symbol is checksum and stop protected | yes, both |
| single-symbology-multiple-barcodes-6 | one of two identical `6RA70826FV620Z+X01` | 2.66 px/module, 620 px long, copies 170 px apart | aggregator | Identical texts merged (same link rule), centers 170 px apart < length 620 | one copy |
| barcode-with-shadow-1 | `566577510700002229` | 8.4 px/module, inverted, 77 deg | edges destroyed | Large glare streak across a curved glossy light-on-dark label | no |

## 5. Root-cause classes and fix hypotheses (ranked by gain / risk)

| # | Class | Symbols | Fix hypothesis | Expected gain | FP risk |
|---|---|---|---|---|---|
| 1 | Code 39 quiet zone required 7 modules, real labels have 4-6.7 | 18 video symbols (Balanced 11 -> 29, tracker 13 -> 30 of 32); 1 in zxing-samples | `crates/breader-linear/src/code39.rs`: `QUIET` 7.0 -> 4.0 (start and stop). Keep `QUIET_SCREEN` (measured: no effect) | +18 (video), +1 samples | Low-moderate. Measured 0 FP on every dataset at 4.0; at 5.0 only 24/32 are read, so do not stop at 5. Code 39 has no mandatory check digit, so keep `min_line_agreement` and the length list unchanged. A tracker confirmation could carry the risk instead if desired |
| 2 | Identical-text symbols merged by the aggregator | 8 in Exhaustive (challenging-images 388 -> 404 incl. all effects), 1 in Balanced (ss-6 copy) | `aggregate.rs::group`: require geometric consistency, not only center distance < longer segment: e.g. link when the segments intersect or are within a fraction (0.2-0.3) of the shorter length across and along, or when the angle difference is small and the perpendicular offset is below the bar height. The scan-line group should stay one symbol, two parallel copies side by side must split | +9 now; fixes Exhaustive being worse than Balanced | Low (more splitting can only drop below `min_line_agreement`, never create text). 0 FP measured on four datasets with factor 0.2 |
| 3 | Weak start character rejected | 2 (ss-7) | `code128.rs::start_of`: accept `start.weak` and count it against the single weak character allowed in `read_symbol` | +2 (36/60) | Low: checksum, stop and quiet zone still apply; 0 FP measured |
| 4 | Plan directions too coarse for short symbols | 8 (ms-2) | `plan.rs`: when a start pattern was seen on a line (the `retry` signal already exists), rescan with lines at the neighbouring +/-7.5 deg offsets; or estimate local orientation (gradient structure tensor) and aim the lines | +8 (synthetic pinwheel, real-world gain small) | Low |
| 5 | Resolution 1.4-1.5 px/module Code 128 | 5 (ms-1 x3, ss-1 x2) | Upsample 2x (cubic) the profile or the image region around start hits before edge detection; relax `MIN_MODULE`/margin rules only with checksum-protected symbols | Up to 5, zxing-cpp reads only 1 of them | Moderate |
| 6 | QR at 2.5 px/module, sampling from the binary grid | 2 (ss-2), 1 readable by zxing | `breader-qr/src/sample.rs`: sample the gray image (bilinear) with a local threshold per module and retry decoding with 2x upsampling; RS arbitrates | +1 to +2 | Low (RS validates) |
| 7 | QR v40 at 3 px/module: wrong dimension hint, alignment fit fails | 1 (csp-4) | `reader.rs::try_dims`: do not let a timing hint that disagrees with the finder-based estimate (161 vs 175) take the first slot and spend the 4-dimension budget; try the estimated dimension first and widen the alignment search radius | +1 | Low |
| 8 | QR curved surface | 1 (shadow-2) | Local grid refinement: fit a piecewise or thin-plate warp from the timing patterns and found alignment patterns, or per-module re-centering by module-edge tracking | +1 (zxing-cpp reads it) | Low |
| 9 | QR finder not found on blurred, low-contrast engraved metal | 1 (A-47) | Finder search on a locally normalized gray image (or try a second binarizer); the existing Balanced second binarizer did not produce a candidate | +1 | Low |
| 10 | Damaged finders (peeled chip, cropped finder), 2 finders + data | 3 (A-41 x2, incomplete-2) | Use two finders plus timing patterns / alignment patterns to place the third corner (the code has synthetic triplets from timing patterns; extend to the case where one finder exists with 2 of 3 timing patterns) | up to +3, zxing-cpp reads none | Moderate (more candidates, RS still validates) |
| 11 | Not worth fixing here | healthcare-1 (1.0 px/module), shadow-1 (glare), csp-9 (embossed), ss-3 thin bars (8, zxing reads 1) | none; ss-3 would need a print-gain model that restores 1 px bars (possible partial gain via Exhaustive, +2) | | |

## 6. Ground-truth observations

No wrong labels found among the failing symbols: the Code 39 values match the printed captions, the `ss-7` Code 128 texts match the printed IMEI/serial text, the 16 `Dynamsoft` entries of `multiple-symbologies-multiple-barcodes-2` match the 16 visible symbols (no duplicate entries). The `batch_code39` annotation lists the 32 visible labels (001-028, 030, 056, 057, 100) correctly; the annotation format is `CODE_39_EXTENDED` but the sheet uses plain Code 39 characters only.

Other observations for the architect:

- ITF reads 0 of 4 in challenging-images at Balanced in this build (zxing-cpp 4 of 4). Not analysed here.
- `zxing-samples` has 9 false positives at Balanced already before any change (2 Code 39 `3PRM8N` and `-12345678`, Data Matrix garbage); they are independent of the experiments above (identical counts with and without them).
- The bench loads by content (`custom-scan-parameters-4.png` is a JPEG); any custom tool must do the same.
