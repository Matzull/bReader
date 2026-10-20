# Contributing

How to work in this repository. External contributions also need a signed CLA (see the [README](../README.md#contributing)); open an issue before a large change. Product priorities, in order: never return wrong data, read as much as possible, be fast ([REQUIREMENTS](REQUIREMENTS.md)).

## Clean-room rules

- ZXing, zbar and other readers may be studied for ideas and approaches; no code, tables or structure may be copied or transliterated. Normative sources are the specifications (list in [REQUIREMENTS](REQUIREMENTS.md#5-clean-room-policy)).
- Tables derivable from a specification (character sets, BCH codes, generator polynomials, capacities) are generated or transcribed from the specification, never from another implementation.
- Third-party readers are black boxes for comparison (zxing-cpp, zbar, rxing, rqrr in the bench; zbar only through its public API via pyzbar); do not read their decoder sources while working on the matching family. Encoder crates are allowed as dev-dependencies to produce test symbols.
- Test datasets are test data only: zxing-cpp samples are images plus expected values, never code ([datasets](datasets.md)).
- Dependencies stay minimal and optional ones sit behind features (NFR-8). `unsafe` is denied workspace-wide (NFR-9).

## Code style

- English identifiers, docs and comments. Comments only where they add information (why, not what); doc comments on public items.
- `cargo fmt`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`.
- No panics on any input, no unbounded loops or allocations (NFR-2); steady-state scanning does not allocate (NFR-4); output is deterministic (NFR-3).
- A decoder only returns results that passed every check for its symbology ([ARCHITECTURE 2.5](ARCHITECTURE.md#25-validation-and-false-positive-control)). Reject reasons are diagnostics, not API.
- Commit messages: short imperative subject, scope prefix when useful (`QR:`, `Linear:`, `Bench:`, `docs:`), the why in the body. Measured effects belong in the message and in [BASELINES](BASELINES.md).

## Definition of done

A change is done when all of this holds:

1. `cargo fmt --all --check` clean.
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean; `cargo doc --no-deps` warning-free when doc comments changed.
3. `cargo test --workspace`; for the five crates with the `rayon` feature also `cargo test -p breader -p breader-imgproc -p breader-linear -p breader-qr -p breader-datamatrix --features breader/rayon` (output must be identical).
4. The gates pass with `--check` ([testing](testing.md#test-and-gate-tiers)): tier a locally (`tools/check-regression.sh`), tier b before merge and for changes that raise false-positive exposure (`--tier b`, which includes the BarBeR development subset; [testing](testing.md#test-and-gate-tiers)), the release tier only when publishing a version. No read count drops.
5. **0 false positives** on every dataset including negatives.
6. Baselines rewritten if reads increased, in the same change, with the movement written in BASELINES.
7. Docs updated (see the table at the end), `tools/check-docs.py` passes (no broken links or anchors) and, if a design choice was made or reversed, a [decision](decisions.md) entry.
8. New public behavior has tests; a bug fix has the failing case as a test; a misdecode fix has its repro.

## Checklists

### Adding a symbology

Example: PDF417 (`breader-pdf417`). Start the design page first; review it before coding.

1. **Design page.** Copy [templates/symbology-page.md](templates/symbology-page.md) to `docs/symbologies/<name>.md`, add the row to [symbologies/README.md](symbologies/README.md) and a pointer stub in [ARCHITECTURE section 3](ARCHITECTURE.md#3-symbology-families). Record scope and out-of-scope in [decisions](decisions.md).
2. **Core types** (`crates/breader-core`): a variant in `Symbology` (`symbology.rs`, name and AIM handling), `SymbolMeta::<Name>(...)` metadata struct in `symbol.rs`, re-exports in `lib.rs`. Options go in `options.rs` only if the symbology has user-visible switches.
3. **Crate** `crates/breader-<name>`: depends on `breader-core` (and `breader-ecc`) only; implements `SymbologyReader` (`uses_binary()`, `handles_inversion()`); family-internal `Detector`/`SymbolDecoder` with static dispatch; checks before error correction (cheap gates) and validation inside the decoder. Add it to the workspace `members`, `default-members` and `[workspace.dependencies]`. Expose `#[doc(hidden)] pub mod fuzzing` entry points with plain types for fuzz targets.
4. **Error correction** in `breader-ecc` if a new field or erasure support is needed (with a property or exhaustive test).
5. **Content** (`breader-content`): charset segments, AIM identifier, classifier rules (GS1 markers) if the symbology carries them.
6. **Facade** (`crates/breader`): optional dependency, a Cargo feature (default-on once gated), registration in `default_pipeline()`, feature added to `rayon`, symbology names in `symbologies` option handling.
7. **Python** (`crates/breader-py`): symbology name in `src/lib.rs`, metadata mapping, `python/breader/_breader.pyi`, a test in `tests/test_breader.py`, README symbology list and `meta` description.
8. **Synthetic data** (`tools/synth`): encoder dev-dependency, `encode.rs` support, a class or inclusion in existing classes (`scene.rs`). Regenerate seed 1 and seed 2024; the new labels change `expected`.
9. **Bench mapping** (`tools/bench`): `groundtruth.rs::map_symbology` spellings, `matching.rs` normalization if the symbology has equivalent notations, `baselines.rs` engine mappings for rxing/rqrr/zxing-cpp, `py_bench.py`, `zxing_samples.rs` directory prefix, and `tools/video-bench/src/expected.rs` for video CSV names.
10. **Datasets** ([datasets](datasets.md)): which subsets contain the symbology, new overrides (visually verified), unsupported labels that become supported (the read rate denominator changes).
11. **Gates**: write baselines for every effort and dataset ([testing](testing.md#rewriting-baselines)); the first baseline of a new symbology starts from the measured counts, with 0 false positives. Add it to the `tools/check-regression.sh` and CI only if a new dataset was added.
12. **Fuzz targets** ([fuzz/README.md](../fuzz/README.md#adding-a-target)): at least a grid or run-sequence decoder target; the `pipeline` target already covers the facade once registered.
13. **Docs**: design page, symbologies table, ARCHITECTURE workspace tree and facade features, REQUIREMENTS scope table (and a new FR/NFR if needed; IDs are never renumbered), README and breader-py README symbology lists, [GUIDE](GUIDE.md) if options changed, BASELINES section with the first measurement, decisions entry.

### Adding a dataset

1. Check the license/terms; if unclear, treat it as internal-only: never commit it, fetch only in the scheduled/manual workflow.
2. Add the fetch step to `tools/fetch-datasets.sh` (pinned tag or commit, shallow clone) and an annotations converter if the labels are not in the `barcode-benchmark/1.0` format (see `tools/bench/src/bin/zxing_samples_gt.rs`).
3. Verify labels visually where they disagree with reads; add overrides per the [policy](datasets.md#overrides-policy), not ad-hoc code.
4. Run the bench at three efforts, triage every false positive ([policy](testing.md#false-positive-policy-and-triage)), then `--write-baseline` into `tools/bench/baselines/<dataset>-<effort>.json`.
5. Wire the gate into `tools/check-regression.sh` (a flag) and the right workflow (`ci.yml` only for generated or redistributable sets; otherwise `datasets.yml` with a cache keyed on the upstream commit or tag).
6. Document it in [datasets](datasets.md), and in [CI](CI.md) (minimal: table rows) and BASELINES (measurements, with the matching changes).

### Adding a preprocessor or binarizer

1. Implement `Preprocessor` or `Binarizer` in `breader-imgproc` (or an external crate: modules need only `breader-core`). Row-wise, integer, bounds-check-free inner loops; `unsafe` only in the isolated `simd` module with a scalar fallback and an equivalence test (NFR-9).
2. Register it in `default_pipeline()` at the position that fits its cost; passes are ordered cheapest first, and effort presets decide which are used ([ARCHITECTURE 2.2](ARCHITECTURE.md#22-passes-and-escalation)).
3. Measure on all gates at three efforts: read counts must not drop anywhere, false positives stay 0, latency compared in interleaved runs on an idle machine. A pass that adds no reads on any dataset is removed, not kept "just in case" (the downscaled variant for 1D was dropped this way).
4. Add a micro-benchmark under `benches/` when it is a hot kernel; record ns/px.
5. Update GUIDE (effort table) and BASELINES if presets change; a change of what a preset runs is a default change (below).

### Changing defaults

Defaults (effort preset contents, option defaults such as ITF lengths, Code 39 Full ASCII, add-on policy, `min_line_agreement`, up-scale retry limits, quiet-zone minimums, feature flags) are product decisions and need an explicit owner decision before the change, not after a good benchmark.

1. Write down the proposal with measurements: reads gained, reads lost, false positives, latency, per dataset and effort.
2. Wait for the owner's decision; do not merge a default change on your own authority.
3. Record it in [decisions](decisions.md) with date and reason; update [GUIDE](GUIDE.md), the breader-py README, and the doc comments of the option.

## Documentation map for contributors

| You changed | Touch |
|---|---|
| A decoder or detector | the family page in [symbologies](symbologies/README.md); BASELINES if reads moved |
| Pipeline, traits, options | [ARCHITECTURE](ARCHITECTURE.md), [GUIDE](GUIDE.md), breader-py README |
| Datasets, labels, overrides | [datasets](datasets.md), BASELINES |
| Gates, baselines, workflows | [testing](testing.md), [CI](CI.md) |
| A decision | [decisions](decisions.md) (append, use the [template](templates/decision.md)) |
| A new document | the map in [docs/README.md](README.md) |

Style of the docs: plain Markdown that renders on GitHub, English, relative links only, one topic per page, numbers with the date and the command that produced them. Keep a stable section order in a family page so [ARCHITECTURE](ARCHITECTURE.md) numbers (3.x) and anchors keep working.
