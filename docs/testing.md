# Testing

What protects the project from regressions and false positives, and how to run and maintain each layer. CI wiring is in [CI](CI.md); dataset details in [datasets](datasets.md).

## Layers

| Layer | Where | Run |
|---|---|---|
| Unit and integration tests | `crates/*/src` (`#[cfg(test)]`), `crates/breader/tests` | `cargo test --workspace` (about 17 s run time with a warm build; the dev profile is `opt-level = 1`, see below) and, for the crates with the `rayon` feature, `cargo test -p breader -p breader-imgproc -p breader-linear -p breader-qr -p breader-datamatrix --features breader/rayon` |
| Allocation test (NFR-4) | `crates/breader/tests/allocation.rs` (counting allocator; own test binary) | Release only: it is ignored in debug builds, because the counts differ and the documented NFR-4 numbers are release ones. Run `cargo test --release -p breader --test allocation -- --nocapture` (about 3 s plus the release build) in tiers a and b below, and in CI; results in [REQUIREMENTS](REQUIREMENTS.md) NFR-4 |
| Python tests | `crates/breader-py/tests` (against the installed wheel) | `pytest` after `maturin develop --release`; see [breader-py](../crates/breader-py/README.md) |
| Regression gate | `tools/bench`, `tools/video-bench`, baselines in `tools/bench/baselines/` | `tools/check-regression.sh [--tier a\|b\|release] [--efforts LIST]`, see [tiers](#test-and-gate-tiers) |
| Fuzzing | `fuzz/` (nightly toolchain, not a workspace member) | see [fuzz/README.md](../fuzz/README.md) |
| Micro-benchmarks | `criterion` in `breader-imgproc` and `breader-ecc` (`benches/`), `tools/bench` `stages` binary | `cargo bench -p breader-imgproc` |

Test symbols are produced by encoder crates used as dev-dependencies only (for example `qrcode`); the decoder sources of those crates or of other readers are never read ([clean-room](contributing.md#clean-room-rules)). Misdecode protections (the zero-false-positive policy) get a test each, with the failing case that motivated them.

## Test and gate tiers

How much to run depends on the change; the measurements and the reasoning are in [TEST-STUDY](TEST-STUDY.md). `tools/check-regression.sh` takes `--tier a|b|release` (default a; `c2` and `--all` are aliases of `release`; the old `c1` is gone and answers with an error) and `--efforts fast,balanced,exhaustive` to keep only some efforts of a tier. It runs the gates on `BENCH_JOBS` threads (default `nproc`; [parallel gates](#parallel-gates)). Any false positive fails every tier. Synth seed 1 is gated at Balanced only; seed 2024 keeps all three efforts.

| Tier | When | Contents | Time |
|---|---|---|---|
| **a** | every change, locally | `cargo test --workspace`; `cargo test --release -p breader --test allocation`; gates at Balanced on synth seed 1, synth seed 2024, Dynamsoft challenging-images, Dynamsoft pdf417-small and zxing-cpp samples (includes the `falsepositives` images and the synth `negative` class); pytest when `crates/breader-py` or the facade API changed | gates 19 s on 16 threads (about 35 s serial), plus tests 17 s and the incremental release build |
| **b** | before merging, and for changes that raise false-positive exposure (new default symbology, relaxed guard or acceptance rule, aggregation or dedup, default features or escalation); CI on pushes to main | a, the rayon test run, and all three efforts on synth seed 2024, challenging-images, skewed-datamatrix, UPS-Synthetic-Labels, pdf417-small and zxing samples; the BarBeR development subset (1/4 plus the trap images, [datasets](datasets.md#barber-development-subset)) at Balanced and Exhaustive; the video gate with the tracker engine only; pytest | gates 128 s on 16 threads (about 10 min serial) |
| **release** | publishing a version; weekly in CI as `c2` (without BarBeR, which is local) | every set at every effort, the full BarBeR set (Fast too) and the video gate with all engines (tracker, fast, balanced); `c2` and `--all` are aliases | gates 277 s on 16 threads (about 42 min serial) |

Tier b replaced the old c1 ([D-051](decisions.md#d-051)): the development subset keeps every BarBeR image where another reader gives a false positive, so a change that can add false positives is exposed to the images most likely to show them, at a fraction of the cost. A new FP mode that appears only on an image outside the subset is caught by the release tier, which is the false-positive gate of record for a version.

Fuzzing is separate (nightly in CI). CI runs tier a on pull requests and tier b on pushes to main, restricted to the synthetic sets (`--synthetic-only`, because the real datasets are only fetched by the weekly job); the weekly job runs c2 ([CI](CI.md)). The times are measured on the Ryzen 7 3700X (16 threads) with the heavy lock held and nothing else running; the serial figures are the documented ones from the shared Ryzen 5 5500U (before 2026-10-20), where a full BarBeR run took 297 s (Fast), 549 s (Balanced) and 1071 s (Exhaustive).

### Dev profile

`[profile.dev] opt-level = 1` in the workspace `Cargo.toml` (debug assertions and overflow checks stay on; `qrcode` and `breader-ecc` keep their opt-level 3 overrides). It takes the workspace tests from about 4 min 13 s to about 20 s run time; the decoder sweeps are 10 to 25 times slower at opt-level 0. Cold build of the tests is about 1.5 minutes.

## Gates

A gate compares a run with a versioned baseline and fails CI on any regression. One baseline file per dataset and effort; the effort is part of the key.

| Gate | Dataset | Baselines | Runs in |
|---|---|---|---|
| synth seed 1 (Balanced only) / seed 2024 | generated | `synth-seed1-balanced`, `synth-seed2024-*` | every pull request and push to main (`ci.yml` `regression`) |
| Dynamsoft challenging / skewed Data Matrix | `data/dynamsoft/...` | `dynamsoft-challenging-*`, `dynamsoft-skewed-*`, `dynamsoft-ups-*`, `dynamsoft-pdf417-*` | weekly or manual (`datasets.yml`, tier c2), locally from tier a (challenging, pdf417) and tier b (all) |
| Dynamsoft videos | `data/dynamsoft/video-based-testing` | `dynamsoft-video.json` | same; tier b runs the tracker engine only, the release tier all three engines (272 s serial, 65 s on 16 threads) |
| zxing-cpp samples | `data/zxing-samples/test/samples` | `zxing-samples-*` | weekly (`datasets.yml`), locally from tier a |
| BarBeR, development subset (3094 images) | `data/barber` (manual download, [datasets](datasets.md#barber)) | `barber-dev-*` | locally only, tier b (`--barber` in the older flag form) |
| BarBeR, full set | same, 8748 images | `barber-*` | locally only, the release tier (`--barber-full`) |

### How it works

`bench <dataset> --effort E --check <baseline>` loads the dataset, decodes every image with the default pipeline and compares per dataset group and per symbology (video-bench: per video). It exits 1 and prints a diff when:

- a read count is lower than the baseline,
- any false positive appears (even when reads went up),
- the expected count differs (the dataset or its labels changed), or a baseline entry is missing from the run.

Comparison engines are measured outside the gate: rxing and rqrr in `bench --features baselines`, zxing-cpp and zbar in `tools/bench/py_bench.py` (`--engine zxingcpp|zbar|all`, `--subset N|dev` as in the Rust bench). zbar needs the system library (`apt install libzbar0`) and `pip install pyzbar`; its normalisations are listed in [BASELINES](BASELINES.md#zbar-2026-10-13). Run them one at a time, like the other heavy jobs.

Higher read counts and entries not yet in the baseline are only notices. Latency is reported, never gated. `--check` and `--write-baseline` require `--engine breader`, the default escalation and the GT overrides. The implementation is `tools/bench/src/gate.rs`.

### Parallel gates

`bench --jobs N` and `video-bench --jobs N` (the latter in `--check` mode too) split the work over N threads: the images of a dataset (`std::thread::scope`, a work queue, one engine per thread because engines are stateful) or the videos (the tracker stays sequential inside a video). Results are merged by image index, so tables, `--json` and gate outcomes are identical to `--jobs 1`; the equivalence was checked on challenging-images, zxing samples, synth seed 2024 and the BarBeR development subset at the three efforts, and on the video gate with all engines. The default is 1, and **latency is only valid with `--jobs 1`**: with more threads it is printed as `n/a` (JSON `latency_ms: null`) in `bench`, and `video-bench` warns that its latencies are not valid. `tools/check-regression.sh` passes `--jobs ${BENCH_JOBS:-$(nproc)}`. Memory is the limit on small machines (one decoded image per thread, up to 20 megapixels in BarBeR): set `BENCH_JOBS` lower. Measure latency with `--jobs 1` as documented below.

### The BarBeR gate

BarBeR has no redistributable license, so it is never run in CI. A full run takes about 5, 9 and 18 minutes at Fast, Balanced and Exhaustive serially (297, 549 and 1071 s on a shared Ryzen 5 5500U with 4 threads; 8748 photos, some of 20 megapixels), too long for every local check, so there are two gates over the same ground truth ([datasets](datasets.md#barber)):

- `--barber` (tier b, Balanced and Exhaustive): the development subset (`bench data/barber --subset dev`: the 1/4 hash share united with the trap list `tools/bench/barber-dev-extra.json`, 3094 images, all twelve sources represented), baselines `barber-dev-<effort>.json`. On the 3700X it takes 11 s (Fast), 20 s (Balanced) and 42 s (Exhaustive) with `--jobs 16`, and 95, 187 and 386 s with `--jobs 1`.
- `--barber-full` (the release tier, all three efforts): the whole set, baselines `barber-<effort>.json`.

Both skip with a message when `data/barber` is missing. The subset baselines are recorded with `--subset dev`, so the expected counts in them differ from the full ones; the two sets are checked against their own files. The group of a baseline is one BarBeR source dataset, the items are the symbologies, so a regression is localised to a source. Undecodable regions are not expected reads and mask false positives on their polygon, which is why this gate can hold the zero-false-positive rule on photos whose ground truth is partly unknown ([datasets](datasets.md#loader)). Rewrite both sets after a change that moves reads:

```sh
for e in fast balanced exhaustive; do
  cargo run -r -p bench -- data/barber --effort $e --jobs 16 --write-baseline tools/bench/baselines/barber-$e.json
  cargo run -r -p bench -- data/barber --subset dev --effort $e --jobs 16 --write-baseline tools/bench/baselines/barber-dev-$e.json
done
```

### Rewriting baselines

After an intended improvement or an accepted dataset or label change:

```sh
cargo run -r -p bench -- data/synth --effort balanced --write-baseline tools/bench/baselines/synth-seed1-balanced.json
cargo run -r -p video-bench -- data/dynamsoft/video-based-testing --write-baseline tools/bench/baselines/dynamsoft-video.json
```

Rules:

- **Increases only.** A baseline is rewritten to record more reads (or a deliberate dataset change). A drop in reads is a bug to fix, never something to record. Writing is refused when the run has false positives.
- Rewrite all three efforts of the affected dataset, review the JSON diff (counts per symbology), and commit it with the change that caused it, quoting the movement in [BASELINES](BASELINES.md).
- A dataset change (new labels, new override, new pinned zxing tag) legitimately changes `expected`; say so in the commit.
- `data/` is gitignored; CI regenerates the synthetic sets, and `synth` is deterministic for a seed and count.

### Merging baselines after combining branches

Branches that each improve reads edit the same JSON files, so a textual merge is wrong even when it applies cleanly: neither branch measured the combined tree.

1. Merge the code first; for conflicting baseline files take either side.
2. Build the merged tree and run every gate with `--check` against the *better* of the two sides. Interactions can lose reads on one branch's gain (for example a label mismatch surfaced only when two branches met: Code 39 Full ASCII).
3. Fix any drop or false positive in code or matching; do not record it.
4. Only then regenerate all affected baselines on the merged tree with `--write-baseline`, review the diff for no decrease against either parent, and commit as a separate "baselines regenerated on the combined tree" change.
5. Update BASELINES with before and after for the merged result.

### Local runs

```sh
tools/fetch-datasets.sh                    # once (Dynamsoft video needs ffmpeg)
tools/check-regression.sh                  # tier a
tools/check-regression.sh --tier b         # before merging and for FP-exposure changes, about 2 min
tools/check-regression.sh --tier release   # publishing a version, about 5 min (c2 and --all are the same)
tools/check-regression.sh --tier b --efforts exhaustive   # one slice of a tier
tools/check-regression.sh --tier a --synthetic-only       # no downloaded datasets (CI)
tools/check-regression.sh --dynamsoft      # older per-set flags: synthetic sets plus that set, all efforts
```

The video gate also accepts `--engines`; with `--engines tracker`, baseline entries of the engines not run are ignored. The frames come from `ffmpeg -pix_fmt gray`, whose range handling depends on the ffmpeg version, so `dynamsoft-video.json` is valid for the version it was written with (n9.0.2 since 2026-10-21, [BASELINES](BASELINES.md#status-2026-10-21-per-class-scoreboard)); a different ffmpeg can move cells by a read or more without any decoder change. Run one heavy job at a time (a tier run, a bench, a wheel build); the gates use all threads by default, so on a shared machine set `BENCH_JOBS` (and `RAYON_NUM_THREADS=4` for latency runs).

Measure latency with `--jobs 1` and only on an idle machine and compare builds in interleaved runs; concurrent jobs distort timings by up to 2x.

## Fuzzing

Targets, running, corpus handling, crash triage and "adding a target" are in [fuzz/README.md](../fuzz/README.md). The corpus is not versioned; CI keeps it in a cache between nightly runs ([CI](CI.md)). A crash is fixed at the root cause with a regression unit test in the owning crate; never by relaxing validation.

## False-positive policy and triage

Requirement NFR-1 ([REQUIREMENTS](REQUIREMENTS.md)): zero wrong results on every dataset, including negatives. Any new false positive blocks the change, whatever it gains. When in doubt, return nothing.

When a gate reports a false positive (a returned symbol not in the labels), classify it before touching the decoder:

1. **Normalization.** The read is the same symbol the label describes in another notation: UPC-A vs EAN-13 with a leading 0, UPC-E expanded, Code 39 vs its Full ASCII expansion, charset-decoded text vs UTF-8 label, a label that is UTF-8 read as Latin-1. Fix the *matching* in `tools/bench/src/matching.rs` with a general rule and a unit test. Not a decoder change.
2. **Unlabelled but real.** A valid, clearly visible symbol the labels omit (partial annotations, a second nested code, a symbol of an unsupported family carried inside a supported one). Verify on the image and add a label override per the [overrides policy](datasets.md#overrides-policy). Check the decode is checksum or Reed-Solomon valid and the payload is sensible next to its printed caption.
3. **Real misdecode.** Wrong data, or a symbol that is not there. Fix the decoder: add the missing validation (quiet zone, check character, agreement, conflict removal, length list), keep a minimal repro as a unit test, and confirm the read count of the other gates does not drop. Never loosen the gate or add the read to a "known" list.

Write the finding in the triage docs when it is non-obvious ([FAILURES](FAILURES.md), [FAILURES-ZXING-SAMPLES](FAILURES-ZXING-SAMPLES.md) show the format: root-cause classes, ranked fixes, measured gain, status). A misdecode protection that costs reads needs an owner decision recorded in [decisions](decisions.md) (the Code 39 extra-line safeguard is an example: it cost one video read and 0 false positives were measured without it as well).
