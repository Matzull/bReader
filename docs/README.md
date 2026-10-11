# breader documentation

breader is a clean-room barcode and QR reader in Rust: no `unsafe`, no C dependencies, optional Python bindings and optional multi-threading. Priorities, in order: never return wrong data (zero false positives is a hard constraint), read as many symbols as possible under hard real-world conditions, be fast enough for real-time video. Dual-licensed `AGPL-3.0-only` plus a commercial license.

This page is the entry point. Find the audience below, follow the link, and when you add a document, add its line to the map.

## Status at a glance

| Symbology | Status | Details |
|---|---|---|
| QR Code | implemented | [qr](symbologies/qr.md) |
| Data Matrix (ECC 200) | implemented | [datamatrix](symbologies/datamatrix.md) |
| EAN-13, EAN-8, UPC-A, UPC-E (+ EAN-2/5 add-ons) | implemented | [linear](symbologies/linear.md) |
| Code 128, Code 39, ITF, Codabar | implemented | [linear](symbologies/linear.md) |
| PDF417 (+ compact, Macro, ECI) | implemented | [pdf417](symbologies/pdf417.md) |
| Aztec Code (compact, full-range, Runes) | implemented | [aztec](symbologies/aztec.md) |
| Micro QR Code (M1 to M4) | implemented | [microqr](symbologies/microqr.md) |
| rMQR Code (R7x43 to R17x139) | implemented | [rmqr](symbologies/rmqr.md) |
| rMQR, DataBar, Codabar | not started | [index](symbologies/README.md) |

Headline numbers (Balanced; zero false positives at every effort on every gated dataset; these are dated snapshots, the current tables, other efforts, other engines and latency are in [BASELINES](BASELINES.md)):

| Dataset | breader | Reference |
|---|---|---|
| Dynamsoft challenging-images, nine symbologies (442 labels) | 400 read (2026-10-08, before the QR/Data Matrix retries) | v1 target for the 392 non-Data-Matrix labels: at least 86 %, zxing-cpp level ([targets](BASELINES.md#v1-targets-confirmed-2026-10-07)) |
| zxing-cpp samples (470 labels) | 348 read (74.0 %) | zxing-cpp 396 with 5 false positives ([table](BASELINES.md#zxing-cpp-samples-2026-10-08)) |
| Synthetic seed 1 (286 symbols) | 283 read | zxing-cpp 82.0 % on the first 228 ([table](BASELINES.md#synthetic-toolssynth-seed-1-20-images-per-class)) |

Per-symbology status, spec references and known gaps: [symbologies/README.md](symbologies/README.md). Architectural and product decisions: [decisions](decisions.md).

## Map

### Users (using the library)

| Document | What it covers |
|---|---|
| [../README.md](../README.md) | Install, five-line examples for Rust and Python, license |
| [GUIDE](GUIDE.md) | Options, effort presets, escalation, time budget, up-scale retry, parallelism, video tracker, GS1, custom pipelines |
| [../crates/breader-py/README.md](../crates/breader-py/README.md) | Python API, options table, threading |
| [../crates/breader/examples](../crates/breader/examples) | Runnable Rust examples |
| `cargo doc --open -p breader` | API reference |
| [../LICENSE-COMMERCIAL.md](../LICENSE-COMMERCIAL.md) | Commercial licensing |
| [../THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md) | License texts of third-party material in the source |

### Contributors (changing the code)

| Document | What it covers |
|---|---|
| [contributing](contributing.md) | Clean-room rules, style, definition of done, checklists (new symbology, dataset, preprocessor, changing defaults) |
| [ARCHITECTURE](ARCHITECTURE.md) | Workspace, pipeline, traits, validation policy, ECC, content, facade, performance plan |
| [symbologies/](symbologies/README.md) | One design page per family ([linear](symbologies/linear.md), [qr](symbologies/qr.md), [datamatrix](symbologies/datamatrix.md), [pdf417](symbologies/pdf417.md), [aztec](symbologies/aztec.md)) and the status table |
| [REQUIREMENTS](REQUIREMENTS.md) | Functional and non-functional requirements (stable IDs), scope, clean-room policy, roadmap |
| [testing](testing.md) | Test layers, regression gates, rewriting and merging baselines, false-positive triage |
| [datasets](datasets.md) | Every dataset: how to fetch, terms, ground-truth format, overrides policy, which gate uses it |
| [../fuzz/README.md](../fuzz/README.md) | Fuzz targets, running, crash triage, adding a target |
| [templates/](templates/symbology-page.md) | Templates for a symbology page and a decision entry |

### Maintainers (measuring, releasing, deciding)

| Document | What it covers |
|---|---|
| [BASELINES](BASELINES.md) | Dated measurements against rxing, rqrr and zxing-cpp; v1 targets |
| [FAILURES](FAILURES.md) | Triage of the Dynamsoft misses and the Code 39 video: root-cause classes, fixes, status |
| [FAILURES-ZXING-SAMPLES](FAILURES-ZXING-SAMPLES.md) | Triage of the zxing-cpp samples that zxing-cpp reads and breader does not |
| [CI](CI.md) | Workflows, the regression gate, ARM runners, Python tests |
| [decisions](decisions.md) | Decision log (license, zero false positives, defaults, scope) |

## Keeping the docs current

- A change that moves reads: add a dated section to [BASELINES](BASELINES.md) and rewrite the baselines ([testing](testing.md#rewriting-baselines)).
- A new symbology: follow [adding a symbology](contributing.md#adding-a-symbology); it touches the design page, the symbologies table, ARCHITECTURE, REQUIREMENTS, GUIDE, both READMEs and the datasets page.
- A decision: append to [decisions](decisions.md).
- A new document: add its line to the map above. The layout is plain Markdown with relative links; a `SUMMARY.md` for mdBook can be added later without moving files.
