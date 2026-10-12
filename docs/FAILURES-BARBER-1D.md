# BarBeR: Code 39 and ITF labels that zxing-cpp reads and breader does not

Dataset: [BarBeR](datasets.md#barber), 198 Code 39 and 72 ITF labels, plus the 4 ITF labels of Dynamsoft challenging-images. Measured on main at `bd844ee` plus the QR triage doc, release build, breader Balanced against zxing-cpp 3.1.1 (Python, default options, black box, outputs only). This is a triage: nothing under `crates/` or `tools/` changed. The experiments ran on a scratch copy of the workspace outside the repository (instrumented `code39.rs`, `itf.rs` and `aggregate.rs` with counters and `BDBG` prints, and a one-file driver that scans Code 39 and ITF only at Balanced), and `data/` was only read.

Counting note: the table counts per label (a label is read when any read of its image has its value). The bench's own miss list (`BENCH_SHOW_FP=1`, Rust bench, Balanced, full set) is used for breader; zxing-cpp was run on the 257 images that carry a Code 39 or ITF label (the Python bench does not print misses). Code 39 matches the bench exactly (breader 122, zxing-cpp 151 of 198). ITF is 34 / 58 in the bench and 33 / 60 here (the labels with duplicate or masked entries differ); the picture does not change. The 1/8 subset is a subset of the same images and has the same causes; it was not run separately.

## 1. Summary

| | Code 39 | ITF |
|---|---|---|
| labels | 198 | 72 |
| read by both | 117 | 31 |
| zxing-cpp only (analysed) | 34 | 29 |
| breader only | 5 (all ZVZ-real, 10-14 characters) | 2 (`20140607_202218`, `Special_0255`) |
| neither | 42 (28 InventBar) | 10 |

Challenging-images ITF (breader 0 of 4, zxing-cpp 4 of 4): `air-travel-luggage-bag` (2 x `0328056306`, 10 digits), `datamatrix-15` (16 digits), `multiple-symbologies-multiple-barcodes-7` (8 digits). All four are the length rule ([FAILURES](FAILURES.md#status-of-the-fixes) already recorded 3 of 4 reading with open lengths).

Code 39 misses by source: InventBar 34 of 34 (all 7-digit numeric labels of one print layout, `ProductBarcode*.jpg`). The breader-only reads are ZVZ-real. ITF misses by source: ZVZ-real 14, SKUU DSP Lab 13, Deal Kaist 1, Muenster 1.

Measured on the misses (my own profile along the annotated axis, 2-means on the run widths; `PPE` is the annotation):

| | Code 39 misses | ITF misses |
|---|---|---|
| digits / characters | 7 (all) | 14: 13, 16: 6, 12: 3, 18: 3, 10: 3, 8: 1 |
| X, px per narrow module (`PPE` from the annotation) | 1.4 to 3.3, median 1.6 (`PPE` 1.22-1.91, median 1.53) | 2.1 to 11.5, median 5 (`PPE` 2-8.6 where annotated; ZVZ-real has none) |
| wide/narrow ratio | 2.0 to 2.9 (median 2.45; one 4.3 measurement error) | 1.8 to 3.0 (ZVZ-real 16/18-digit: 1.8-1.9) |
| quiet zone, both sides, in X | 8.6 to 75 (median 30) | 5 to 70; 5 of 29 below 10 X (`20140607_202722` 7.5 and 10.8, `201406`, `Special_0060`, `Special_0066`: 9 to 12) |

Read rate of the InventBar Code 39 labels by module size (breader / zxing-cpp, over all InventBar labels): X below 1.5 px 6 / 17 of 29; 1.5-1.8 px 18 / 28 of 38; 1.8-2.2 px 54 / 64 of 67; above 2.2 px 7 / 10 of 13. So Code 39 is a resolution problem: the quiet zone and the wide/narrow ratio of the misses are the same as the labels breader reads.

## 2. Pipeline stage of the first failure

Counters per scanline over all scans of an image (Balanced, Code 39 and ITF only), for the 34 Code 39 and 25 ITF miss images.

Code 39 (34 images):

| stage | labels | evidence |
|---|---|---|
| decoder reads a correct symbol, aggregation drops it | 8 | `ProductBarcode152_0`, `283`, `320`, `334_0`, `336_0`, `351`, `467_0`, `475`: the hit text equals the label. `aggregate.rs::required`: fewer than two distinct plan lines (`baselines={15}`, one plan line) raise the agreement from 2 to 3. Lines: 2 agreeing for `152_0`, `320`, `336_0`, `351`, `467_0` (so 5 of them miss by one line), 1 for `283`, `334_0`, `475` |
| symbols start but a character fails the element tests | 24 | start pattern passes 1-14 times, then `read_char` fails: `MIN_SEPARATION` (1.3) 239 times, `RATIO` (1.6-3.6) 106, inter-character gap 41, no start-to-stop run completes. Examples `ProductBarcode177`, `179_0`, `202`, `330`, `392`, `420`, `425`, `483_0` |
| start character itself rejected | 2 | `ProductBarcode176`, `295`: the star fails on separation or ratio |

Quiet zone is not a cause here (the 8 `start quiet fail` events of the whole set are all in images that fail later anyway; the labels have 9 to 75 X). Code 39 here has no check digit, so only the line agreement stands between a misread and an output.

ITF (29 labels in 25 images):

| stage | labels | evidence |
|---|---|---|
| length rule (length is not 14) | 13 | full symbols are decoded and counted as `lenrej`: `9008420501238-01.jpg` and `_N95` (16 digits, 20 and 17 full reads), `Special_0008` (8), `0054` (12), `0119` (3 labels, 18), `0218`, `0219`, `0251`, `0252` (16), `0249`, `0250` (12). With `allowed_lengths` open these 13 read |
| decoder reads a correct 14-digit symbol, aggregation drops it | 3 | `20140607_201513` (2 hits on 1 line), `202226` (3 hits, 2 lines), `202722` (6 hits, 3 lines). `ITF_MIN_AGREEMENT` is 3 and the single-plan-line rule makes it 4 (`required=4`) |
| check digit | 1 | `20140607_201410`: 12 full reads of `18803718101326` rejected by the ITF-14 check; the label itself fails GTIN (expected check digit 3, printed 6), zxing-cpp does not check. It is the only invalid one of 53 14-digit labels |
| decode fails inside the symbol | 10 | `201406` (2 labels), `201446`, `201505`, `204332`, `204336`, `201416`, `Special_0060`, `0066`, `0268`: the start pattern passes 70-190 times but pairs fail on separation (30-110), wide/narrow above 3.4 (11-35) and module drift (20-75); no complete symbol is found (`0066`, `0268`, `201416` decode a few symbols of the wrong length that are not the label). I could not isolate one reason (see section 3) |
| unexplained | 2 | `20140607_202330` has two labels; in the one-file driver (Code 39 + ITF only) one reads `18801019605403` (12 full reads, 6 lines), the full-pipeline bench misses both. Probably another reader or the escalation shapes the result; not investigated |

The rows add up to 29. With lengths open the scratch driver reads 15 of the 29 labels: the 13 of the length row and the 2 `202330` labels.

## 3. Causes with examples

### C1. ITF accepts only 14 digits (owner decision D-008)

13 of the 29 ITF misses, 3 of the 4 Dynamsoft ITF. All have wide quiet zones (50-70 X) and clean modules (X 2.5 to 7 px), the decoder already produces the full symbol. Images: `Special_0218.jpg`, `Special_0219.jpg` (16 digits, ZVZ-real), `9008420501238-01.jpg` (16), `Special_0249.jpg`, `Special_0250.jpg` (12, rotated 90 degrees), `Special_0119.jpg` (3 labels of 18), `Special_0008.jpg` (8), Dynamsoft `air-travel-luggage-bag.jpg`, `datamatrix-15.jpg`, `multiple-symbologies-multiple-barcodes-7.jpg`.

False-positive evidence: on the 257 images with open lengths, the scratch driver and the Python reader produced one ITF read that matches no label (`Special_0166`: `0067641801002510`; not checked on the image; the ZVZ-real label there may be unknown or `^`). That is one in 257 images, over the images chosen for having ITF, not a measurement on the 8748 images; a full-set run with open lengths is the missing evidence.

### C2. Code 39 at 1.2 to 2 px per module: separation and ratio tests fail

24 plus 2 labels. The labels are 7 digits at 1.4-2.2 px per narrow bar, wide/narrow about 2.4 after blur, so the third-largest element is not 1.3 times above the fourth. Images: `ProductBarcode177.jpg`, `179_0.jpg`, `202.jpg` (X 3.3 by my measurement, ratio 4.3: a different print), `330.jpg`, `392.jpg`, `420.jpg`, `483_0.jpg`, `176.jpg`, `295.jpg` (viewed: sharp enough, wide quiet zones, small label with the number printed below). zxing-cpp reads 17 of the 29 InventBar labels below 1.5 px, 28 of 38 at 1.5-1.8 px, so the information is in the profile.

### C3. Aggregation: the single-plan-line rule

Code 39 8 labels (5 with 2 lines, 3 with 1 line), ITF 3 labels (`202226`, `202722` have 2-3 agreeing lines). Hit lines are mostly retry lines (indices above the plan) around one plan line, so the rule that asks for one more line when fewer than two plan lines agree fires. The correct text is the only text hit in every one of these images (no conflicting text seen: `GROUP` lines list one text each).

### C4. ITF decode failures inside the symbol

10 labels, SKUU DSP Lab photos of boxed ITF-14 labels (a rectangular frame around the symbol, 100-400 px high, 2-12 px modules, some rotated by 8-15 degrees or by 90 degrees with perspective: `20140607_201406.jpg`, `201446.jpg`, `204336.jpg` viewed) and 3 small ZVZ-real symbols (`Special_0060.jpg`: 34 px high, X 2.4). Quiet zones measure 10 X and more, start patterns pass, pair decoding fails. A hypothesis, not proven: the frame is near the symbol and perspective or rotation changes the module size along the line (drift), and the profile merges bars.

### C5. ITF label with an invalid check digit

`20140607_201410.jpg`: 1 label. breader decodes the digits correctly 12 times and discards them by the ITF-14 check. This is a label problem, no fix proposed (a check-digit-optional mode is an owner decision).

## 4. Ranked fixes (stage plus idea, no code)

| rank | stage | idea | labels | FP risk |
|---|---|---|---|---|
| 1 | aggregation, `aggregate.rs::required` | The single-plan-line `+1` applies to Code 39 and ITF the same as to checked symbologies. For Code 39 (2 lines) and ITF (3 lines) replace it by a geometric guard: the agreeing lines must be distinct scanlines at least 2 steps apart (not just retry lines at the same place) and their module estimates agree within 15 %. Expected: Code 39 +5 (`152_0`, `320`, `336_0`, `351`, `467_0`), ITF +1 to +3 (`202722` has 3 lines; `202226` 2; `201513` 1 stays unread) | up to 8 | Low to moderate. Code 39 has no check digit; the replacement guard keeps misreads from a single row out; to be gated with the full BarBeR run at 0 false positives |
| 2 | decoder, `code39.rs::read_char` / `itf.rs::read_pair` element classification | At about 1.5 px per module use a relative decision: pick the 3 widest (Code 39) or 2 widest (ITF) elements per group and accept them when the third is above the fourth by a smaller factor (for example 1.15) provided the whole symbol passes start, stop, inter-character gap and module-drift checks and at least 3 lines agree (4 below 1.8 px). Also try sub-pixel edge positions from an upscaled profile. Expected: part of 24 Code 39 labels (InventBar below 1.8 px: 27 labels read by zxing-cpp only) | up to 26 | Highest of the three: Code 39 has no check digit and 7-digit numeric labels are easy to hit with a wrong width; the replacement guard for the relaxed separation is the 3 or 4 line agreement plus both quiet zones at or above 7 X |
| 3 | ITF decoder, quiet zone / frame and drift | Examine the SKUU boxed labels (`201406`, `201446`, `201505`, `204332`): trace which pair first fails (drift or ratio above 3.4) with the frame lines excluded; probable fix is a local module re-estimation per pair instead of the 1.7 drift limit. Not proven; needs a further experiment | up to 10 | Moderate (ITF) |

C1 (lengths) is the largest single gain (13 of 29 ITF labels) and is not in this list because it is an owner decision. Items 1 and 2 together are the Code 39 route; their upper bound is 8 + 26 = 34 of the 34 misses, a realistic gain is well below that.

## 5. Needs an owner decision

1. ITF default lengths ([D-008](decisions.md#d-008), `[14]`): recovers 13 of 29 ITF misses on BarBeR and 3 of 4 on Dynamsoft, 16 labels of the 33 zxing-cpp-only ITF labels of both sets. A narrower option than an open list: add the lengths seen in the data (8, 10, 12, 16, 18) with 4 agreeing lines and both quiet zones at least 10 X, keep 14 at 3 lines. FP evidence so far is one unverified read in 257 images; a full-set run with open lengths and the quiet and line limits is needed before deciding.
2. A Code 39 line-agreement rule with fewer lines (ranks 1 and 2) trades misread protection for reads on a symbology without a check character ([D-017](decisions.md#d-017) recorded the earlier extra-line rule as insurance).
3. ITF with an invalid GTIN-14 check digit (`20140607_201410`): keep rejecting (current) or allow it with more lines. One label.
