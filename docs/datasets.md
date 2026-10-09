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
| Dynamsoft `UPS-Synthetic-Labels` | `data/dynamsoft/UPS-Synthetic-Labels` | same | CC BY 4.0 | not gated yet (Code 128 only; MaxiCode unsupported) | none |
| Dynamsoft `pdf417-small-module-size` | `data/dynamsoft/pdf417-small-module-size` | same | no license stated; internal only | not gated yet (waits for the PDF417 reader) | none |
| zxing-cpp samples | `data/zxing-samples/test/samples` | `tools/fetch-datasets.sh zxing-samples` | Apache-2.0, test data only | `--zxing-samples`, `datasets.yml` (`zxing-samples` job) | `zxing-samples-*.json` |

All Dynamsoft subsets come from one clone, [Dynamsoft/datasets-from-dynamsoft](https://github.com/Dynamsoft/datasets-from-dynamsoft) (`git clone --depth 1`, updated with `pull --ff-only`). CI caches it keyed on the upstream commit.

## Synthetic sets

`tools/synth` renders symbols with encoder crates and degrades them with a seeded RNG, so a seed and a count reproduce the set exactly (on one platform: rendering uses floating-point trigonometry, so CI generates the Python test copy on Linux only). Classes: `clean`, `rotated`, `perspective`, `blur`, `low-res`, `illumination`, `inverted`, `noise`, `multi`, `datamatrix`, and `negative` (no symbol: any read is a false positive). Seed 1 has 20 images per class (286 symbols); seed 2024 has 60 (831 symbols). Both are gated because the second one catches over-fitting to the first.

Adding a symbology means adding a class or extending `multi` in `tools/synth/src/scene.rs`/`encode.rs`, then regenerating both sets and baselines ([checklist](contributing.md#adding-a-symbology)).

## Dynamsoft subsets

Real-world photos. Used for internal testing only: most subsets state no license, so images and annotations are never committed, never redistributed and never fetched on every push (only by the weekly or manual `datasets.yml`).

- `challenging-images`: 68 images, 514 labels (about 400 in the nine v1 symbologies, the rest Data Matrix / PDF417 counted separately or unsupported). Ten images carry an EXIF orientation that the labels' quads already apply; the bench decodes raw pixels and matches by text only.
- `skewed-datamatrix`: 42 photos of Data Matrix at skewed angles (1323 labels after overrides).
- `video-based-testing`: 1080p videos plus one CSV per video with the expected set of codes (no per-frame labels). `video-bench` decodes frames with `ffmpeg`; metrics are time to first read, share of the expected set ever read, false positives and per-frame latency.
- `UPS-Synthetic-Labels`: 18 synthetic UPS-style labels with two Code 128 and one MaxiCode each; CC BY 4.0, attribute as "UPS Synthetic Shipping Label Dataset, Dynamsoft, 2025." Annotations are `annotation/annotations_min.json`, not the bench format yet.
- `pdf417-small-module-size`: 11 PDF417 images with small modules; reserved for the PDF417 reader (M9).

## zxing-cpp samples

`test/samples` of zxing-cpp at the pinned tag `zxing_tag` in `tools/fetch-datasets.sh` (shallow sparse checkout; CI keys its cache on that tag). Apache-2.0; only the images and expected-result files are used, as test data, never code. 62 directories, 932 images, 46 of them `falsepositives-*` negatives. `tools/fetch-datasets.sh zxing-samples` builds and runs `zxing-samples-gt` (`tools/bench/src/bin/zxing_samples_gt.rs`), which writes an `annotations.json` into each directory. Conversion rules (directory prefix gives the symbology, `.txt`/`.bin` the value, add-on digits dropped in `*-extension-*`, what is left out and why) are in [BASELINES](BASELINES.md#zxing-cpp-samples-2026-10-08). Unsupported symbologies (Aztec, PDF417, MaxiCode, DataBar, ...) are labeled but excluded from the read rate.

To move the pin, change `zxing_tag`, rerun the fetch, regenerate all `zxing-samples-*.json` baselines and review the diff of counts (the dataset change shows as an "expected count differs" failure).

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

`format` spellings vary by source (`EAN13`/`EAN_13`, `QRCode`/`QR_CODE`) and are normalized in `tools/bench/src/groundtruth.rs`; a format with no breader counterpart is counted as unsupported. Matching is by symbology and data (`tools/bench/src/matching.rs`); positions are informational.

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

Normalizations that are not overrides (UTF-8 read as Latin-1 in labels, Code 39 Full ASCII expansion, UPC-E/UPC-A/EAN-13 folding, charset-decoded text) live in `matching.rs` with unit tests; see [false-positive triage](testing.md#false-positive-policy-and-triage).
