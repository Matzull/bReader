# Continuous integration

Documentation hub: [README.md](README.md). How the gates are maintained (rewriting and merging baselines, false-positive triage): [testing.md](testing.md). Datasets: [datasets.md](datasets.md).

Workflows live in `.github/workflows/`. All use `contents: read` only.

| Workflow | Trigger | What it does |
| --- | --- | --- |
| `ci.yml` | pull request, push to `main`, tags `v*` | `lint`: fmt, clippy (`--workspace --all-targets --all-features -D warnings`), cargo-deny. `test`: `cargo test --workspace` on ubuntu x86_64, ubuntu arm64, macOS arm64 and Windows; a second run with `--features breader/rayon` limited to the five crates that have the feature (`breader`, `-imgproc`, `-linear`, `-qr`, `-datamatrix`); on ubuntu x86_64 also the release-only allocation test (`cargo test --release -p breader --test allocation`). `regression`: `tools/check-regression.sh --synthetic-only` at tier a on pull requests and tier b on pushes to main. `changes` decides whether the Python jobs run: always on pushes to main and tags, on pull requests only when `crates/breader-py/`, `crates/breader/` (the facade) or `ci.yml` changed. `python-wheels` / `python-test`: abi3 wheels (linux x86_64 and aarch64, macOS arm64, Windows x64) and pytest against the installed wheel. |
| `datasets.yml` | weekly (Mon 05:00 UTC), manual | One job, `gate-c2`: restores `data/dynamsoft` (cache keyed on the upstream commit, fetched on a miss) and `data/zxing-samples` (cache keyed on the pinned zxing-cpp tag, annotations regenerated), installs ffmpeg and runs `tools/check-regression.sh --tier c2`: all sets at all efforts (synthetic, the four Dynamsoft image sets, zxing-cpp `test/samples`) and the video gate with all engines. BarBeR is skipped (no data in CI). |
| `fuzz.yml` | nightly (03:00 UTC), manual | Nightly toolchain, runs every target from `cargo fuzz list` for 300 s (manual runs can change it), caches `fuzz/corpus`, uploads `fuzz/artifacts` when something crashes. |

The Dynamsoft datasets have no stated license, so they are only fetched by the scheduled or manual job, never on every push. The zxing-cpp samples are fetched by the same workflow (sparse checkout of `test/samples` at a pinned tag); only the images and their expected-result files are used, as test data.

## Tiers in CI

The tiers are defined in [testing](testing.md#test-and-gate-tiers). CI maps them to events:

| Event | Tier | Runs | Time |
| --- | --- | --- | --- |
| pull request | a | `cargo test --workspace` (dev profile `opt-level = 1`, about 17 s of test run time per OS image after a warm build), the allocation test, synthetic gates at Balanced (seed 1, seed 2024); the Python jobs only when the bindings or facade changed | not measured; dominated by the builds |
| push to `main` | b | the same plus the rayon test run on five crates, synth seed 2024 at all three efforts, wheels and pytest | not measured |
| weekly (`datasets.yml`), manual | c2 | every dataset in CI at every effort and the video gate with all engines (about 7 minutes of gates plus the dataset fetch; BarBeR is local-only) | about 10 minutes of gates (estimate) |
| nightly (`fuzz.yml`) | fuzz | 17 targets, 300 s each | 85 minutes |

The real datasets are not fetched on pull requests or pushes (the Dynamsoft sets have no stated license), so CI's tier a and b cover the synthetic sets only (`--synthetic-only`); the Dynamsoft, zxing and video parts of tiers a and b are a local check, and the weekly job runs all of them. The BarBeR gates (b subset, c1 and c2 full set) are local only. Run c1 locally for the changes in [the c1 list](testing.md#when-to-run-c1).

## The regression gate

`bench` and `video-bench` compare a run with a versioned baseline in `tools/bench/baselines/`:

| File | Dataset | Command |
| --- | --- | --- |
| `synth-seed1-balanced.json` (Balanced only) | `data/synth` (`synth data/synth`, seed 1, 20 per class) | `bench data/synth --effort balanced` |
| `synth-seed2024-{...}.json` | `data/synth-2024` (60 per class, seed 2024) | `bench data/synth-2024 --effort E` |
| `dynamsoft-challenging-{...}.json` | `data/dynamsoft/challenging-images` | `bench data/dynamsoft/challenging-images --effort E` |
| `dynamsoft-skewed-{...}.json` | `data/dynamsoft/skewed-datamatrix` | `bench data/dynamsoft/skewed-datamatrix --effort E` |
| `dynamsoft-ups-{...}.json` | `data/dynamsoft/UPS-Synthetic-Labels` (CC BY 4.0; 52 of its 58 labelled images are present) | `bench data/dynamsoft/UPS-Synthetic-Labels --effort E` |
| `dynamsoft-pdf417-{...}.json` | `data/dynamsoft/pdf417-small-module-size` | `bench data/dynamsoft/pdf417-small-module-size --effort E` |
| `dynamsoft-video.json` | `data/dynamsoft/video-based-testing` | `video-bench data/dynamsoft/video-based-testing` (`--engines tracker` for tier b and c1: entries of engines not run are ignored) |
| `zxing-samples-{...}.json` | `data/zxing-samples/test/samples` (one group per sample directory) | `bench data/zxing-samples/test/samples --effort E` |

A baseline stores, per dataset (or video) and per symbology (or engine), the expected count, the read count and the false positives. The effort is part of the key: a baseline only applies to the effort it was recorded at.

`--check` exits with code 1, and prints a diff, when:

- a read count is lower than the baseline,
- any false positive appears,
- the expected count differs (the dataset changed), or a baseline entry is missing from the run.

Higher read counts and entries not yet in the baseline only print a notice. Latency is reported but never gated.

### Run it locally

```sh
tools/fetch-datasets.sh                # once, needs git (ffmpeg for the videos)
tools/check-regression.sh              # tier a (default); synthetic sets are generated into data/ when missing
tools/check-regression.sh --tier b     # before merging: all efforts, all real sets, BarBeR subset, tracker video
tools/check-regression.sh --tier c1    # FP-exposure changes: tier b with the full BarBeR set at Balanced and Exhaustive
tools/check-regression.sh --tier c2    # phase end (--all is the same): everything, all efforts, all video engines
tools/check-regression.sh --tier b --efforts exhaustive   # keep only some efforts of a tier
tools/check-regression.sh --tier a --synthetic-only       # what CI runs
```

BarBeR (`data/barber`, manual download, internal only) is never in CI and is skipped with a message when missing. The older per-set flags (`--dynamsoft`, `--zxing-samples`, `--barber`, `--barber-full`) still work and run the synthetic sets plus the chosen set at all efforts.

Or one at a time:

```sh
cargo run -r -p bench -- data/synth --effort balanced --check tools/bench/baselines/synth-seed1-balanced.json
```

The two newer Dynamsoft sets keep their ground truth in `annotation/*.json` next to the images (`annotations_min.json`, `test-results.json`); the bench loads any `.json` in a directory named `annotation` as the `barcode-benchmark/1.0` format, resolves image names by stem (the PDF417 names end in `_annotated`, the UPS names carry no extension) and skips labels whose image is absent. Their PDF417 and MaxiCode labels count as unsupported (excluded) until the library reads them: adding the symbology to `map_symbology` in `tools/bench/src/groundtruth.rs` is the only bench change then (a test fails until it is added), followed by rewriting the baselines.

`--check` and `--write-baseline` need `--engine breader`, the default escalation and the GT overrides.

### Update a baseline

After an intended improvement (the check prints a notice), or an accepted change of the datasets:

```sh
cargo run -r -p bench -- data/synth --effort balanced --write-baseline tools/bench/baselines/synth-seed1-balanced.json
cargo run -r -p video-bench -- data/dynamsoft/video-based-testing --write-baseline tools/bench/baselines/dynamsoft-video.json
```

Do this for all three efforts, review the diff of the JSON files and commit it with the change that caused it. Writing is refused when the run has false positives. A drop in reads should be fixed, not recorded.

`data/` is gitignored; CI regenerates the synthetic sets (`synth` is deterministic for a given seed and count).

## ARM runners

`ubuntu-24.04-arm` hosted runners are not available for private repositories on every plan. If the `test (ubuntu-24.04-arm)` job cannot start, keep the entry and either enable the runners for the organisation, or replace the job's steps with cross compilation under QEMU:

```sh
cargo install cross --locked
cross test --workspace --target aarch64-unknown-linux-gnu
```

## Python tests

`crates/breader-py/tests` run against the installed wheel. The synthetic-dataset test reads `$BREADER_SYNTH_DIR` (default `<repo>/data/synth`) and is skipped when the directory is missing. CI generates it on Linux only, because rendering uses floating-point trigonometry whose last bits are not guaranteed identical across platforms.
