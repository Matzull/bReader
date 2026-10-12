# Datasets

Every dataset the project measures against. `data/` is gitignored: nothing here is committed except the baselines and the label overrides in `tools/bench/`. Numbers are in [BASELINES](BASELINES.md); how the gates use the sets is in [testing](testing.md#gates). To add one, see [adding a dataset](contributing.md#adding-a-dataset).

## Summary

| Dataset | Location | Obtain | Terms | Gated by | Baseline files |
|---|---|---|---|---|---|
| Synthetic seed 1 | `data/synth` | `cargo run -r -p synth -- data/synth` (generated, 20 per class) | ours, deterministic | CI `regression`, `tools/check-regression.sh` | `synth-seed1-*.json` |
| Synthetic seed 2024 | `data/synth-2024` | `synth data/synth-2024 --count 60 --seed 2024` | ours, deterministic | same | `synth-seed2024-*.json` |
| Dynamsoft `challenging-images` | `data/dynamsoft/challenging-images` | `tools/fetch-datasets.sh dynamsoft` | no license stated; internal only | `--dynamsoft`, `datasets.yml` | `dynamsoft-challenging-*.json` |
| Dynamsoft `skewed-datamatrix` | `data/dynamsoft/skewed-datamatrix` | same | no license stated; internal only | `--dynamsoft`, `datasets.yml` | `dynamsoft-skewed-*.json` |
| Dynamsoft `video-based-testing` | `data/dynamsoft/video-based-testing` | same (needs `ffmpeg` to run) | no license stated; internal only | `--dynamsoft`, `datasets.yml` (`video-bench`) | `dynamsoft-video.json` |
| Dynamsoft `UPS-Synthetic-Labels` | `data/dynamsoft/UPS-Synthetic-Labels` | same | CC BY 4.0 | `--dynamsoft`, `datasets.yml` (Code 128; MaxiCode unsupported) | `dynamsoft-ups-*.json` |
| Dynamsoft `pdf417-small-module-size` | `data/dynamsoft/pdf417-small-module-size` | same | no license stated; internal only | `--dynamsoft`, `datasets.yml` (PDF417 counted; 7 unlabelled Code 128 added as overrides) | `dynamsoft-pdf417-*.json` |
| zxing-cpp samples | `data/zxing-samples/test/samples` | `tools/fetch-datasets.sh zxing-samples` | Apache-2.0, test data only | `--zxing-samples`, `datasets.yml` (`zxing-samples` job) | `zxing-samples-*.json` |
| BarBeR | `data/barber/BarBeR - Dataset` | manual: `data/BarBeR_Dataset.zip` (6 GB), extract into `data/barber/` | no license stated, sources have their own terms; internal only | `--barber` (1/8 subset), `--barber-full`, `--all`; not in CI | `barber-subset-*.json`, `barber-*.json` |

All Dynamsoft subsets come from one clone, [Dynamsoft/datasets-from-dynamsoft](https://github.com/Dynamsoft/datasets-from-dynamsoft) (`git clone --depth 1`, updated with `pull --ff-only`). CI caches it keyed on the upstream commit.

## Synthetic sets

`tools/synth` renders symbols with encoder crates and degrades them with a seeded RNG, so a seed and a count reproduce the set exactly (on one platform: rendering uses floating-point trigonometry, so CI generates the Python test copy on Linux only). Classes: `clean`, `rotated`, `perspective`, `blur`, `low-res`, `illumination`, `inverted`, `noise`, `multi`, `datamatrix`, and `negative` (no symbol: any read is a false positive). Seed 1 has 20 images per class (319 symbols, 33 of them PDF417); seed 2024 has 60 (911 symbols, 80 PDF417). Each class except `negative` and `datamatrix` carries ceil(count / 9) extra Data Matrix images and then as many PDF417 images (random columns 2 to 8, level 1 to 5, a quarter of them compact; `multi` images then hold only PDF417), generated after the earlier ones, so every earlier image is byte-identical. Both are gated because the second one catches over-fitting to the first.

Adding a symbology means adding a class or extending `multi` in `tools/synth/src/scene.rs`/`encode.rs`, then regenerating both sets and baselines ([checklist](contributing.md#adding-a-symbology)).

## Dynamsoft subsets

Real-world photos. Used for internal testing only: most subsets state no license, so images and annotations are never committed, never redistributed and never fetched on every push (only by the weekly or manual `datasets.yml`).

- `challenging-images`: 68 images, 514 labels (about 400 in the nine v1 symbologies, the rest Data Matrix / PDF417 counted separately or unsupported). Ten images carry an EXIF orientation that the labels' quads already apply; the bench decodes raw pixels and matches by text only.
- `skewed-datamatrix`: 42 photos of Data Matrix at skewed angles (1323 labels after overrides).
- `video-based-testing`: 1080p videos plus one CSV per video with the expected set of codes (no per-frame labels). The CSV of `batch_rotated_small_pdf417_1080p` is empty; `tools/video-bench/src/expected.rs` has an `ADDITIONS` list (video, format, payload, reason) for labels verified by hand, empty so far: neither breader nor zxing-cpp reads a PDF417 in any of its 404 frames (a tiny label on a test tube, modules well under a pixel). `video-bench` decodes frames with `ffmpeg`; metrics are time to first read, share of the expected set ever read, false positives and per-frame latency.
- `UPS-Synthetic-Labels`: 18 synthetic UPS-style labels with two Code 128 and one MaxiCode each; CC BY 4.0, attribute as "UPS Synthetic Shipping Label Dataset, Dynamsoft, 2025." Annotations are `annotation/annotations_min.json` in the bench format, loaded from the `annotation/` subdirectory; 52 of the 58 listed images exist in the checkout (the rest are skipped with a notice).
- `pdf417-small-module-size`: 11 photos of AIAG shipping labels, 15 PDF417 labels (`annotation/test-results.json`). The QTY and SUPP Code 128 symbols on the labels are unlabelled upstream; the ones breader and zxing-cpp read were verified visually and added in `gt-overrides.json` (with a `count` field for repeated symbols).

## zxing-cpp samples

`test/samples` of zxing-cpp at the pinned tag `zxing_tag` in `tools/fetch-datasets.sh` (shallow sparse checkout; CI keys its cache on that tag). Apache-2.0; only the images and expected-result files are used, as test data, never code. 62 directories, 932 images, 46 of them `falsepositives-*` negatives. `tools/fetch-datasets.sh zxing-samples` builds and runs `zxing-samples-gt` (`tools/bench/src/bin/zxing_samples_gt.rs`), which writes an `annotations.json` into each directory. Conversion rules (directory prefix gives the symbology, `.txt`/`.bin` the value, add-on digits dropped in `*-extension-*`, what is left out and why) are in [BASELINES](BASELINES.md#zxing-cpp-samples-2026-10-08). Unsupported symbologies (MaxiCode, DataBar, rMQR, ...) are labeled but excluded from the read rate.

To move the pin, change `zxing_tag`, rerun the fetch, regenerate all `zxing-samples-*.json` baselines and review the diff of counts (the dataset change shows as an "expected count differs" failure).

## BarBeR

[BarBeR](https://github.com/Henvezz95/BarBeR) ("Barcode Benchmark Repository", 8748 real photos with 9844 annotated barcodes, collected from 12 public barcode datasets): used for the realistic read rate of the linear codes, QR and Data Matrix in volume. **No license is stated, and each source dataset has its own terms: treat it like the Dynamsoft sets, internal use only, never committed, redistributed or fetched by CI.**

Obtaining it: it is not fetched by a script. Put `BarBeR_Dataset.zip` at `data/BarBeR_Dataset.zip` and extract it so that the result is `data/barber/BarBeR - Dataset/` (the folder name has spaces) with `README.txt`, `Annotations/VIA/*.json` and `dataset/images/*.jpg`. Nothing else is used: the YOLO `labels/` and the train/val/test lists are ignored. `tools/check-regression.sh` skips the BarBeR stage with a message when the folder is missing.

### Format

Findings, checked against the data:

- `Annotations/VIA/` has 12 VGG Image Annotator projects, one per source dataset (Artelab, Artelab Extended, Bodnar, Deal Kaist, Dubska, InventBar, Muenster, Open Food Facts, ParcelBar, SKUU DSP Lab, Szentandrasi-Dubska QR, ZVZ-real). `_via_img_metadata` maps a key (file name followed by a `size`) to `{filename, size, regions, file_attributes}`; a region is `{shape_attributes, region_attributes}` with the three attributes `Type`, `PPE` and `String`. The `size` is not the file size (686 entries differ from it) and is only used as described below.
- File names map directly to `dataset/images/<filename>`: all 8748 distinct names exist, and every image is in at least one project (nothing is missing in either direction). 8773 entries name 8748 files, because 24 images are in both Dubska and Szentandrasi-Dubska QR (same photo, annotated twice, in two cases with a different QR of the image each) and `IMG_20120226_161648.jpg` has two entries in Szentandrasi-Dubska QR for two resolutions (`size` 2073600 and 4915200), of which only the first fits the 1920x1080 image.
- Regions: 9844 (the README says 9818). 7999 entries have one region, 769 have 2 to 15 and 5 have none (four in Dubska, one in Muenster). Shapes are 4-point polygons, apart from 22 polygons of 5 to 7 points and one ellipse.
- `Type` is a short code, not the README's spelling: `EAN13` 5023, `C128` 1504, `QR` 1397, `1D` 621, `UPCS` 313, `C39` 202, `PDF417` 194, `ROYALMAILCODE` 192, `DATAMATRIX` 146, `I2O5` 75, `EAN8` 68, `AZTEC` 42, `JAPANPOST` 25, `UPCA` 10, `KIX` 8, `POSTNET` 7, `UCC128` 7 (GS1-128), `2-DIGIT` 3 (EAN-2), `IATA25` 3, `INTELLIGENTMAIL` 2 and `-1` 2.
- `String` is always a JSON string. `-1` marks 727 regions as undecodable (`1D` 589, `C128` 57, `EAN13` 42, `QR` 30, ...) and 215 regions of ZVZ-real hold `^` (an unknown value; also `^` followed by tabs), one string is empty. The 32 `1D` regions with a value have no symbology. `UPCS` is always a 12-digit UPC-A (no UPC-E exists); `UCC128` strings are plain digits or text, with no parentheses or FNC1. Code 39 values may carry the `*` delimiters (`*8802041*`), some values end in a newline or carry leading spaces, and ZVZ-real strings are XML-escaped (`&gt;`, `&amp;`).
- Ground-truth quality problems found: `EAN13` regions whose string has 12 digits (UPC-A), 8 (EAN-8) or 11, 14 and 20 digits (not an EAN-13 at all); typos (labels with an invalid EAN check digit, `http//` URLs); one garbled Code 128 string; QR strings with a literal `\000001` prefix; a QR label that belongs to another plant; a `-1` polygon whose vertices lie outside the image; a 1-character Code 128 labelled as the 2-digit add-on `Q1`; unannotated symbols; and the ZVZ-real payload encodings below. Polygons are in the EXIF-corrected frame (177 images have an EXIF rotation of 90, 180 or 270 degrees), the engines see the raw pixels.

### Loader

`tools/bench/src/barber.rs` reads the VIA files directly (nothing is written to `data/`): `bench data/barber` detects `Annotations/VIA`, makes one dataset per project (the per-source tables of [BASELINES](BASELINES.md#barber-2026-10-11)) and converts the regions to the bench annotation format in memory, then applies the common cleanup (overrides keyed `barber` in `gt-overrides.json`, de-duplication). `tools/bench/py_bench.py` has the same loader (`map_region`, `legacy_payload`, `load_barber_sources`), so zxing-cpp is measured identically.

Rules:

- **Types.** `EAN13` becomes EAN-13 for 13 digits, UPC-A for 12 and EAN-8 for 8; any other string is not a well-formed EAN-13 and is treated as undecodable. `UPCS` and `UPCA` become UPC-A (12 digits) or UPC-E (8). `C128` and `UCC128` are Code 128 (a GS1-128 is compared like the bench compares GS1 everywhere: a leading FNC1 is dropped, `(AI)` parentheses are removed); `C39` is Code 39 without the `*` delimiters; `I2O5` is ITF; `QR`, `DATAMATRIX`, `PDF417`, `AZTEC` as named. Postal codes (`ROYALMAILCODE`, `KIX`, `POSTNET`, `JAPANPOST`, `INTELLIGENTMAIL`), `IATA25` and `2-DIGIT` have no breader symbology: they are "unsupported (excluded)" like in the other datasets.
- **Undecodable regions.** `Type` `1D` or `-1`, `String` `-1`, `^` or empty are not expected reads and do not count in the read rate. A read that matches no expected label and whose quad overlaps such a polygon is not a false positive: its value is unknown. Overlap is one of: a corner or the centre of the read inside the polygon, the polygon's centre inside the read, or bounding boxes that share at least a quarter of the smaller one (`matching::overlaps_region`, same in Python). A read without a position (rxing for some symbols) cannot be masked. Polygons are mapped from the EXIF-corrected frame to the raw pixels before the test. Masked reads are printed with `BENCH_SHOW_FP=1` (`masked read (undecodable region)`); at Balanced breader decodes a value on about 300 regions the dataset calls undecodable.
- **Text normalization**: XML character references are decoded; line endings CR LF and CR compare equal to LF (general, in `matching::canonical`; the labels use LF).
- **ZVZ-real binary payloads.** Its PDF417, Data Matrix and Aztec labels show the payload bytes as text or as hex: IBM437 characters (`M\u00f7glinger` for `M\xf6glinger`; recognised by box-drawing, Greek or division characters), or the hex of the UTF-16 units of such text (`0002005E...`; units that all fit a byte are byte values, otherwise IBM437 characters). A NUL and a real caret both show as `^`, so a hex payload with a `^` is ambiguous and is treated as undecodable (32 regions). Plain digit strings that merely look like hex are left alone.
- **Duplicated images.** An image named in several projects is scanned once, under the first project in file-name order (Dubska), with the union of the regions; the common de-duplication then merges the labels of one symbol (23 removed). Several entries for one file in one project keep those whose `size` equals the image's pixel count.
- **Subset.** `bench ... --subset N` keeps the images whose FNV-1a hash of the file name is divisible by N (also in `py_bench.py`): a stable, source-balanced sample for the quick gate. `--only NAME` limits the run to the sources whose name contains NAME.
- **Overrides** (`gt-overrides.json`, key `barber`, 15 entries, each with the evidence): invalid check digits and typos, a garbled Code 128, the `\000001` QR prefix, a mislabeled QR, a 1-character Code 128 labelled `Q1`, an Aztec labelled with a ticket number (masked as undecodable), and three unlabeled symbols (two EAN-13, one QR). Same [policy](#overrides-policy) as for the other sets.

## Ground-truth format

Every dataset directory (or sample directory) holds an `annotations.json`:

```json
{
  "format": "barcode-benchmark/1.0",
  "images": [
    { "file": "clean_0001.png",
      "barcodes": [ { "text": "47157812783177", "format": "QR_CODE",
                      "bytes_hex": "optional, exact bytes, wins over text",
                      "points": [[332.7, 518.2], [510.0, 518.2], [510.0, 700.0], [332.7, 700.0]] } ] } ]
}
```

`format` spellings vary by source (`EAN13`/`EAN_13`, `QRCode`/`QR_CODE`) and are normalized in `tools/bench/src/groundtruth.rs`; a format with no breader counterpart is counted as unsupported. Matching is by symbology and data (`tools/bench/src/matching.rs`); positions are informational. GS1 DataBar labels are raw element strings (`0104412345678909`), other readers print `(01)04412345678909` or keep FNC1 as GS (and rxing omits the `01` of Omnidirectional); the matching drops `(`, `)` and GS on both sides (`py_bench.py` the same). `DATABAR`, `DATABAR_LIMITED` and `DATABAR_EXPANDED` labels are supported; the stacked formats stay unsupported (`rss14-2`, `rssexpandedstacked-*`). Codabar labels (zxing-cpp samples `codabar-1`, `codabar-2`) and the other engines' reads spell the start and stop characters (`A1234B`); the library returns the data between them, so the matching drops a leading and a trailing A-D from both sides of a Codabar comparison (`py_bench.py` does the same). No Dynamsoft label is Codabar.

Cleanup on load, applied identically by every engine:

- **Deduplication.** Two labels of one image are the same symbol when their normalized text matches (UPC-A folds into EAN-13 with a leading 0) and their quads overlap. Documented with the 66 removed duplicates in [BASELINES](BASELINES.md#ground-truth-cleanup).
- **Overrides** in `tools/bench/gt-overrides.json` (embedded in the binary), keyed by dataset directory name and image file. An entry with `from` and `to` replaces a wrong label; an entry with `to` only (plus `points`) adds a missing one. Every entry has a `reason`. `--no-overrides` disables them (and `--check`/`--write-baseline` refuse that).

### Overrides policy

A label changes only when the image proves it, never because breader (or any other engine) read something different.

1. Look at the image itself, on a crop of the symbol and its caption (account for EXIF orientation). Visual verification is mandatory; a decode by any engine is not evidence.
2. For an added label: the symbol must be clearly visible and valid (check digit / Reed-Solomon) and sit away from every existing annotation.
3. Write `reason` so a reviewer can repeat the check (what is printed, what was verified, why the checksum could not decide).
4. Overrides are labels, not read-rate tuning: add a unit test only when the matching logic changes.
5. Record the addition in BASELINES next to the numbers it moves, and regenerate the affected baselines.

Normalizations that are not overrides (line endings, UTF-8 read as Latin-1 in labels, Code 39 Full ASCII expansion, UPC-E/UPC-A/EAN-13 folding, charset-decoded text) live in `matching.rs` with unit tests; see [false-positive triage](testing.md#false-positive-policy-and-triage).

### Aztec labels (zxing-cpp samples and synthetic sets)

`aztec-1` and `aztec-2` (48 labels) count since the Aztec reader landed. Two spellings differ from what a reader returns and are handled in the matching, not by overrides: the Aztec Rune labels are three digits (`001`; the reader returns them that way) and the structured append sample `Z1-sequence4of7` is labelled with the payload only (`3456`) while the reader returns the message with its header (` Z1.txt DG3456`); `tools/bench/src/matching.rs` strips a leading ` ID XY` header (space, ID without spaces, space, two capitals in order) from Aztec reads before comparing, with a unit test. The synthetic sets gained Aztec images after the PDF417 ones (3 per class and 12 in `multi` at 20 images per class, 7 per class at 60), existing images byte-identical: compact and full-range symbols over 14 layer counts (compact 1 to 4, full-range 2 to 15), every third image mirrored, inverted and perspective classes included.

### Micro QR labels (zxing-cpp samples and synthetic sets)

`microqrcode-1` (16 images, one label each: M1 to M4 including kanji, binary and mixed-mode symbols, five photographs, one symbol without quiet zone) counts since the Micro QR reader landed: `MICRO_QR_CODE` and the spellings `MicroQRCode` / `MICRO_QR` map to `Symbology::MicroQr` in `tools/bench/src/groundtruth.rs`. No overrides. The synthetic sets gained Micro QR images after the Aztec ones (existing images byte-identical): the seven variants M1, M2-L, M2-M, M3-M, M4-L, M4-M and M4-Q (M3-L is left out because the encoder's M3-L symbols are not valid, see [microqr](symbologies/microqr.md#as-built)), every third image mirrored, all classes but `negative` and `datamatrix`.

### rMQR labels (zxing-cpp samples and synthetic sets)

`rmqrcode-1` (3 images, one label each: `R7x43-H`, its inverted version and `R17x139`) counts since the rMQR reader landed: `RMQR_CODE` and the spellings `RMQRCode` / `RMQR` / `RectangularMicroQRCode` map to `Symbology::Rmqr` in `tools/bench/src/groundtruth.rs`. No overrides. The synthetic sets gained rMQR images after the Micro QR ones (existing images byte-identical): the 32 sizes at levels M and H spread over the images (numeric, alphanumeric and byte payloads of 50 to 100 % of the capacity), every third image mirrored, all classes but `negative` and `datamatrix`. The encoder is the `qrcode-generator` crate (see [rmqr](symbologies/rmqr.md#scope)).
