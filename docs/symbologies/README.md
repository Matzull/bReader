# Symbologies

One row per symbology. To add a symbology, follow [adding a symbology](../contributing.md#adding-a-symbology), start the page from [the template](../templates/symbology-page.md) and add the row here.

Read rates are never copied into this table (they go stale); the link goes to the measurement in [BASELINES](../BASELINES.md). The gates that protect each symbology are listed in [testing](../testing.md#gates).

| Symbology | Crate | Status | Spec | Design page | Read rate | Known gaps |
|---|---|---|---|---|---|---|
| QR Code (Model 2, v1-40, ECI, mirrored, light-on-dark) | `breader-qr` | implemented | ISO/IEC 18004 | [qr.md](qr.md) | [challenging-images](../BASELINES.md#dynamsoft-challenging-images), [zxing samples](../BASELINES.md#zxing-cpp-samples-2026-10-08) | Micro QR, rMQR, Model 1 and Structured Append reassembly out of scope for v1; remaining misses in [FAILURES](../FAILURES.md#3-qr-misses-in-challenging-images-9-of-234) and [FAILURES-ZXING-SAMPLES](../FAILURES-ZXING-SAMPLES.md#qr-code); kana-only Shift JIS `text` is Latin-1 (`data` exact) |
| Data Matrix (ECC 200, square, rectangular, DMRE, GS1, ECI) | `breader-datamatrix` | implemented | ISO/IEC 16022, ISO/IEC 21471 | [datamatrix.md](datamatrix.md) | [Data Matrix](../BASELINES.md#data-matrix-2026-10-08) | Direct part marking out of scope (dot-peened, rough metal); Fast reads dark-on-light only; misses listed in BASELINES |
| EAN-13, EAN-8, UPC-A, UPC-E, EAN-2/EAN-5 add-ons | `breader-linear` | implemented | ISO/IEC 15420 | [linear.md](linear.md) | [zxing samples](../BASELINES.md#zxing-cpp-samples-2026-10-08), [challenging-images](../BASELINES.md#dynamsoft-challenging-images) | Fast misses many small retail symbols; single-line and edge cases in [FAILURES-ZXING-SAMPLES](../FAILURES-ZXING-SAMPLES.md#ean-13) |
| Code 128 (sets A/B/C, FNC1 / GS1-128) | `breader-linear` | implemented | ISO/IEC 15417 | [linear.md](linear.md) | [challenging-images](../BASELINES.md#dynamsoft-challenging-images) | Weak-start and aggregation misses in [FAILURES](../FAILURES.md#4-code-128-misses-in-challenging-images-26-of-60) |
| Code 39 (standard; Full ASCII and mod-43 optional) | `breader-linear` | implemented | ISO/IEC 16388 | [linear.md](linear.md) | [linear fixes](../BASELINES.md#linear-reader-fixes-2026-10-08) | Full ASCII off by default; 4-module quiet zone with extra-line safeguard ([decision](../decisions.md)); Code 32 / PZN not supported |
| ITF (Interleaved 2 of 5) | `breader-linear` | implemented | ISO/IEC 16390 | [linear.md](linear.md) | [zxing samples](../BASELINES.md#zxing-cpp-samples-2026-10-08) | Default accepts 14 digits only, by design ([decision](../decisions.md)); shorter labels need `allowed_lengths` |
| PDF417 (compact, Macro, ECI) | `breader-pdf417` | decoding done (GF(929) RS, compaction, Macro, ECI); detection and reader in progress (M9) | ISO/IEC 15438 | [pdf417.md](pdf417.md) | none yet; datasets ready: `pdf417-small-module-size`, zxing `pdf417-*` ([datasets](../datasets.md)) | MicroPDF417 and GS1 composite deferred |
| Aztec, Micro QR, rMQR, DataBar, Codabar, others | none | not started (phase D) | - | - | - | Counted as unsupported in the zxing samples read rate |

Status values: `implemented` (reader in the default build, gated at 0 false positives), `design only` (page written, no crate), `not started`.
