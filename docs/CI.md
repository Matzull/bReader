# Continuous integration

Documentation hub: [README.md](README.md). How the gates are maintained (rewriting and merging baselines, false-positive triage): [testing.md](testing.md). Datasets: [datasets.md](datasets.md).

Workflows live in `.github/workflows/`. All use `contents: read` only, except the `draft` job of `release.yml` (`contents: write`, to draft a release).

| Workflow | Trigger | What it does |
| --- | --- | --- |
| `ci.yml` | pull request, push to `main`, tags `v*` | `lint`: fmt, clippy (`--workspace --all-targets --all-features -D warnings`), cargo-deny. `msrv`: Rust 1.94 (the declared `rust-version`) runs `cargo check --all-targets --locked` on the workspace with all features and on `crates/breader-py` and `crates/breader-wasm` (D-069). `miri`: nightly `cargo miri test` of the SIMD tests of `breader-imgproc` and the unit tests of `breader-ffi`, the two places with `unsafe` (D-069). `test`: `cargo test --workspace` on ubuntu x86_64, ubuntu arm64, macOS arm64 and Windows; a second run with `--features breader/rayon` limited to the crates that have the feature (`breader`, `breader-cli`, `-imgproc`, `-linear`, `-qr`, `-datamatrix`), which includes `crates/breader/tests/rayon_determinism.rs`: it scans mixed frames of 1.9 and 0.56 million pixels (thin Code 39, EAN-13, Code 128, QR, Data Matrix) and `data/synth` when present, at all three efforts, once on the rayon pool and once under `breader_core::parallel::sequential`, and requires identical symbols, corners and order (D-069); on ubuntu x86_64 also the C interface step (cbindgen regenerates `crates/breader-ffi/include/breader.h` and `git diff --exit-code` fails on a difference, then a release build of `breader-ffi` and of the C example; the example is also compiled and run by `crates/breader-ffi/tests/c_example.rs` in `cargo test --workspace`, D-064) and the release-only allocation test (`cargo test --release -p breader --test allocation`). `regression`: `tools/check-regression.sh --synthetic-only` at tier a on pull requests and tier b on pushes to main. `changes` decides whether the Python jobs run: always on pushes to main and tags, on pull requests only when `crates/breader-py/`, `crates/breader/` (the facade) or `ci.yml` changed. `python-wheels` / `python-test`: abi3 wheels (linux x86_64 and aarch64, macOS arm64, Windows x64) and pytest against the installed wheel. `wasm`: the `crates/breader-wasm` package (its own workspace): fmt, clippy (native and `wasm32-unknown-unknown`), unit tests, `build.mjs` (wasm-pack, web and nodejs targets) and `tests/node.mjs` on Node 22. |
| `datasets.yml` | weekly (Mon 05:00 UTC), manual | One job, `gate-c2`: restores `data/dynamsoft` (cache keyed on the upstream commit, fetched on a miss) and `data/zxing-samples` (cache keyed on the pinned zxing-cpp tag, annotations regenerated), installs ffmpeg and runs `tools/check-regression.sh --tier c2`: all sets at all efforts (synthetic, the four Dynamsoft image sets, zxing-cpp `test/samples`) and the video gate with all engines. BarBeR is skipped (no data in CI). |
| `fuzz.yml` | Mon to Sat 03:00 UTC (`nightly`), Sunday 03:00 UTC (`weekly`), manual (`mode`) | Nightly toolchain. `nightly`: every target from `cargo fuzz list` for 300 s (95 minutes of fuzzing; a manual run can give the slow ones of `SLOW_TARGETS` more with `long_seconds`), job limit 210 minutes; caches `fuzz/corpus`. `weekly`: a matrix with one job per target of `WEEKLY_TARGETS` (`pipeline`, `rs2m_decode`, `aztec_scan`, `code39_template`, `linear_profile`), 3600 s each, 5 hours of runner time in all, each job limited to 2 hours; the corpus of each target is cached on its own (`fuzz-corpus-weekly-<target>-`). Both pass `-timeout=10 -rss_limit_mb=2048` (libFuzzer's default timeout is 1200 s, which would hide a hang) and `-max_len=70000` for `pipeline`, and upload `fuzz/artifacts` when something crashes. Manual runs can change the seconds (`seconds`, `long_seconds`). |
| `release.yml` | tags `v*`, manual | Builds what a release ships, publishes nothing ([D-066](decisions.md#d-066)). `crates`: lifts `publish = false` in the runner's checkout only and runs `cargo package --workspace` (every library crate packaged and built on its own, as crates.io would), keeping the `.crate` files. `wheels`: abi3 wheels for linux x86_64 and aarch64, macOS arm64 and x86_64, Windows x64. `sdist`: the Python source distribution, installed from source in a clean environment. `binaries`: one archive per platform (linux x86_64, macOS arm64, Windows x64) with the `breader` command line tool and the C library (shared and static) with its header, each crate built in its own target directory ([D-067](decisions.md#d-067)). `draft` (tags only): attaches all of it to a draft GitHub release. Publishing to crates.io or PyPI is a manual step for the owner. |

The Dynamsoft datasets have no stated license, so they are only fetched by the scheduled or manual job, never on every push. The zxing-cpp samples are fetched by the same workflow (sparse checkout of `test/samples` at a pinned tag); only the images and their expected-result files are used, as test data.

## Tiers in CI

The tiers are defined in [testing](testing.md#test-and-gate-tiers). CI maps them to events:

| Event | Tier | Runs | Time |
| --- | --- | --- | --- |
| pull request | a | `cargo test --workspace` (dev profile `opt-level = 1`, about 17 s of test run time per OS image after a warm build), the allocation test, synthetic gates at Balanced (seed 1, seed 2024); the Python jobs only when the bindings or facade changed | not measured; dominated by the builds |
| push to `main` | b | the same plus the rayon test run on five crates, synth seed 2024 at all three efforts, wheels and pytest | not measured |
| weekly (`datasets.yml`), manual | c2 | every dataset in CI at every effort and the video gate with all engines (about 7 minutes of gates plus the dataset fetch; BarBeR is local-only) | about 10 minutes of gates (estimate) |
| Mon to Sat (`fuzz.yml`) | fuzz | 19 targets at 300 s | 95 minutes |
| Sunday (`fuzz.yml`) | fuzz | 5 slow or complex targets, 3600 s each, one job per target | 5 hours of runner time, about one hour of wall time |

The real datasets are not fetched on pull requests or pushes (the Dynamsoft sets have no stated license), so CI's tier a and b cover the synthetic sets only (`--synthetic-only`); the Dynamsoft, zxing and video parts of tiers a and b are a local check, and the weekly job runs all of them. The BarBeR gates (the development subset in b, the full set in the release tier) are local only; tier b covers the changes that raise false-positive exposure.

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
| `dynamsoft-video.json` | `data/dynamsoft/video-based-testing` | `video-bench data/dynamsoft/video-based-testing` (`--engines tracker` for tier b: entries of engines not run are ignored) |
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
tools/check-regression.sh --tier b        # before merging and for FP-exposure changes: all efforts, all real sets, BarBeR development subset, tracker video
tools/check-regression.sh --tier release   # publishing a version (c2 and --all are the same): everything, all efforts, full BarBeR, all video engines
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
