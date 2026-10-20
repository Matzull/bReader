# Test and gate study (2026-10-13)

All timings in this study are from the Ryzen 5 5500U (shared with another agent); the project moved to a Ryzen 7 3700X on 2026-10-20.

> **Update 2026-10-21 ([D-051](decisions.md#d-051)).** The tiers of this study were changed by the owner after it was written: tier C1 is gone, tier B now runs the BarBeR development subset (the 1/4 hash share united with the trap images where zxing-cpp or zbar give a false positive, [datasets](datasets.md#barber-development-subset)) at Balanced and Exhaustive instead of the 1/8 subset, the full BarBeR set runs only in the release tier (the old C2), and the gates run on all threads (`--jobs`). The 1/8 subset, its baselines and its "FP seen with probability 1/8" argument below are historical; the current tiers and times are in [testing](testing.md#test-and-gate-tiers). The times in this study remain the serial ones.

Recommendation report on the cost and overlap of the test suite and the regression gates. Nothing here is implemented; the ranked change list is at the end. Owner's position: end-to-end tests are necessary but overused; no redundant tests; nothing that takes about 40 minutes unless it is really important; coverage is not to be removed blindly; the zero-false-positive rule ([D-002](decisions.md#d-002)) is a hard constraint.

## 1. Method and caveats

- Release test binaries: `cargo test --workspace --release --no-run`, then each binary run alone with `--report-time --test-threads=1` (`RUSTC_BOOTSTRAP=1` to get `-Z unstable-options` on stable). Per-test times in the inventory are these.
- Debug (what `cargo test --workspace` and CI do): wall time of the real command, per binary; per-test debug times from a 4-thread run (so the slowest test is the critical path of its binary).
- Dev profile experiment: same command with `CARGO_PROFILE_DEV_OPT_LEVEL=1` in a separate target dir (deleted afterwards).
- Gates: every dataset gate run with `--check` against its baseline, at each effort, one at a time, `RAYON_NUM_THREADS=4`; video gate timed per engine. All passed. The full BarBeR set was **not** run; its times (297 / 549 / 1071 s, the owner's 5 / 9 / 18 minutes) are from [BASELINES](BASELINES.md#gate). The BarBeR subset was timed at Fast only (36 s); Balanced and Exhaustive (67 s, 133 s) are from BASELINES.
- The machine was shared with another agent's BarBeR bench (4 threads, load about 1.2 of 12 cores, 6 to 7 GB available) for the whole session. Absolute times are indicative; ratios are reliable.
- Python: `pytest` in the existing `crates/breader-py/.venv` against its installed develop build.

## 2. Current totals

| Command | Time | Notes |
|---|---|---|
| `cargo test --workspace` (debug, as developers and CI run it) | **4 min 13 s** run time (sum of binaries 252 s) | CI runs it twice (second time `--features breader/rayon`) on 4 OS images |
| `cargo test --workspace --release` | about 45 s serial (about 35 s wall); 1 min 54 s cold build | 669 tests |
| `cargo test --workspace` with `CARGO_PROFILE_DEV_OPT_LEVEL=1` | **31 s** run time; 1 min 44 s cold build | same tests, debug assertions kept |
| `pytest` (77 tests) | 3 s | the cost in CI is the 4 wheel builds, not the tests |
| `tools/check-regression.sh` (default: synth seed 1 and 2024, 3 efforts) | 68 s + incremental build | the CI `regression` job |
| `--dynamsoft` | 111 s image sets + 285 s video = 6.6 min | weekly in `datasets.yml` |
| `--zxing-samples` | 29 s | weekly |
| `--barber` (1/8 subset, 3 efforts) | about 4.2 min (docs; Fast alone measured 36 s) | local only |
| `--barber-full` | about 32 min (297 + 549 + 1071 s) | local only |
| **`--all`** | **about 40 min** (68 + 111 + 285 + 29 + 1917 s) | the thing the owner objects to; 80 % of it is full BarBeR, 12 % the video gate |
| Fuzz (`fuzz.yml`) | 17 targets x 300 s = 85 min | nightly in CI, not on the developer's machine |

Everything except the full BarBeR set and the video gate is cheap: all datasets at all three efforts, apart from those two, take about 4.6 minutes together (synth 68 s, Dynamsoft image sets 111 s, zxing 29 s plus the subset).

## 3. Inventory

669 unit and component tests (`#[test]`; 670 with the rayon-only one) in 21 test binaries, plus 77 Python tests, 17 fuzz targets, 8 gate families and 3 workflows. Release time is the binary alone; debug time is the binary alone in the default dev profile.

### 3.1 Rust test binaries

| Binary | Tests | Kind | Release | Debug | Guards | Covered elsewhere cheaper? |
|---|---|---|---|---|---|---|
| `breader-qr` lib | 83 | unit + component + a few reader-level e2e | 12.0 s | 20.5 s | QR/Micro QR/rMQR: RS, BCH, finders, triplets, sampling, never-wrong; `qr_codes_are_never_micro_qr` / `_rmqr` are FP protections | stage tests are the cheap layer; the reader-level sweeps overlap the synth QR classes |
| `breader-datamatrix` lib | 50 | unit + component + sweeps | 11.2 s | 65.0 s | ECC200 decode, detection, sampling, reader | three sweeps over the same size x angle x px grid (section 5) |
| `breader-aztec` lib | 75 | unit + component + two e2e sweeps | 10.2 s | 51.1 s | Aztec decode, detect, sample, reader | two reader sweeps overlap the stage tests (section 5) |
| `breader-linear` lib | 196 | unit + component | 4.4 s | 12.6 s | all 1D decoders, edges, aggregation, robustness, `*_never_misread` | this is the cheap layer; nothing duplicates it |
| `breader` `tests/allocation.rs` | 1 (90 scans) | e2e (full pipeline, counting allocator) | 3.6 s | **80.4 s** | NFR-4 allocations; blank frames also assert "reads nothing" | the "reads nothing" part duplicates `ean13.rs` and `lib.rs` blank tests |
| `breader-ecc` lib | 24 | unit | 1.8 s | 0.7 s | RS over GF(256/929/2^m) | fuzz `rs*_decode` adds random depth |
| `synth` bin | 11 | unit + generator e2e | 0.9 s | 16.4 s | deterministic generation, all encoders | none needed; renders the sets twice |
| `breader-pdf417` lib | 89 | unit + component | 0.8 s | 3.0 s | PDF417 | cheap layer |
| `breader` lib | 8 | unit | 0.1 s | 1.0 s | tracker, pixel formats | cheap |
| `ean13.rs` | 4 | e2e through the facade | 0.03 s | 0.6 s | default pipeline wiring, flipped/strided views, add-on policy | EAN detail is in `breader-linear`; wiring is unique |
| `qr_content.rs` | 5 | e2e through the facade | 0.05 s | 0.4 s | QR segments -> text -> content class | charset/GS1 logic is in `breader-content` and `breader-qr` tests |
| `upscale_retry.rs` | 4 | e2e through the facade | 0.02 s | 0.3 s | 3x up-scale retry policy | blank case duplicated by `allocation.rs` |
| `rayon_determinism.rs` | 1 (9 scans of 1700x1300 pages) | e2e | 0.5 s | (not measured; feature-gated) | parallel == sequential, symbol for symbol | unique |
| `breader-content`, `-core`, `-imgproc` | 21 / 16 / 26 | unit | below 0.1 s | below 0.2 s | charset, GS1, geometry, kernels | cheap layer |
| `benchlib` (tools/bench) | 51 | unit | 0.01 s | about 0 | gate logic, matching normalisation, GT loaders | protects the gates themselves; keep |
| `video-bench`, `zxing-samples-gt`, `bench`, `stages` | 3 / 1 / 0 / 0 | unit | about 0 | about 0 | | |
| doc tests | 1 | doc | 0 | 0 | | |

Slowest single tests (debug, 4 threads / release):

| Test | Debug | Release |
|---|---|---|
| `datamatrix sample::clean_sweep_succeeds_at_every_stage` | 63.5 s | 5.7 s |
| `aztec reader::reads_compact_and_full_symbols_of_every_size_rotated` | 50.3 s | 4.7 s |
| `aztec reader::reads_in_perspective` | 41.5 s | 3.7 s |
| `allocation steady_state_scans_do_not_allocate` | 80 s (alone) | 3.6 s |
| `datamatrix sample::sampled_grid_equals_source` | 20.7 s | 1.9 s |
| `datamatrix reader::reads_all_sizes_rotated` | 19.6 s | 1.9 s |
| `qr micro::reader_tests::qr_codes_are_never_micro_qr` | 17.0 s | 1.7 s |
| `synth tests::generation_is_deterministic_and_annotated` | 16.2 s | 0.6 s |
| `qr rmqr::reader_tests::qr_codes_are_never_rmqr` | 13.5 s | 1.4 s |
| `qr decode::tests::roundtrip_all_versions_levels_modes` | 4.7 s | 3.5 s |

Finding: nine tests make up 85 % of the debug time, and the cause is the dev profile, not the tests. `Cargo.toml` optimises only `qrcode` and `breader-ecc` in dev; every decoder and `breader-imgproc` run at opt-level 0, which is 10 to 25 times slower than release for these loops (Data Matrix sweep 63 s against 5.7 s). With `opt-level = 1` for the dev profile the whole `cargo test --workspace` falls from 4 min 13 s to 31 s with the same 669 tests and debug assertions and overflow checks still on.

### 3.2 Python (`crates/breader-py/tests`, 77 tests, 3 s)

| Group | Kind | Guards | Covered elsewhere? |
|---|---|---|---|
| Input handling, formats, strides, errors, options (about 60) | binding unit/component | the PyO3 layer, GIL release, thread sharing | no; Rust tests do not see the binding |
| `test_synth_clean_matches_rust_bench` | binding e2e on `data/synth` | the wheel reads the same as the Rust bench | partly (the Rust gate); it checks the binding, keep |
| `test_*_meta_on_synth` (DM, PDF417, Aztec, Micro QR, rMQR) | binding e2e | metadata exposure per symbology | Rust-side meta is tested per crate; these check the conversion |

Nothing to remove. Cost in CI is wheel building (4 wheels per push), see section 6.

### 3.3 Fuzz (`fuzz/`, 17 targets, nightly CI 5 min each)

`pipeline`, `aztec_scan` (whole-reader targets); `qr_grid`, `microqr_grid`, `rmqr_grid`, `dm_grid`, `aztec_grid`, `pdf417_codewords`, `pdf417_line`, `linear_runs`, `linear_profile` (stage targets); `rs_decode`, `rs929_decode`, `rs2m_decode`, `gs1_parse`, `charset_decode`, `content_pipeline` (pure components). Kind: fuzz. Guards: no panic, no wrong data where a checksum exists. The stage targets reach deeper than the cheap unit tests' random tests (`random_grids_never_panic...`) and the whole-reader targets cannot replace them. No target is redundant; the 300 s is uniform, which over-spends on the small pure targets (`gs1_parse`, `charset_decode`, `rs*`) and under-spends on `pipeline` and `aztec_scan`.

### 3.4 Gates (all `--check`, count-based, any false positive fails)

Times measured today, fast / balanced / exhaustive, seconds.

| Gate | Images / labels | Kind | Fast | Bal | Exh | Sum | What it guards that unit tests cannot |
|---|---|---|---|---|---|---|---|
| synth seed 1 (20 per class, 11 classes) | 487 labels | dataset (synthetic) | 1.8 | 4.9 | 10.7 | 17 | whole-pipeline reads per class and effort; `negative` class FP |
| synth seed 2024 (60 per class) | 1311 labels | dataset (synthetic) | 5.2 | 13.8 | 31.6 | 51 | same, 2.7x the samples |
| Dynamsoft challenging-images | 448 labels | dataset (real photos) | 2.8 | 4.9 | 9.8 | 17.5 | real-photo reads, FP on real photos |
| Dynamsoft skewed-datamatrix | 1324 DM labels | dataset (real) | 6.5 | 12.3 | 18.4 | 37 | DM under skew |
| Dynamsoft UPS-Synthetic-Labels | 104 labels | dataset (real) | 8.3 | 14.4 | 29.8 | 52.5 | large shipping labels, unlabelled Code 128 |
| Dynamsoft pdf417-small-module-size | 22 labels | dataset (real) | 0.8 | 1.3 | 2.6 | 4.7 | small PDF417; FP in dense labels |
| zxing-cpp samples (about 1950 files, 49 groups) | 794 labels, 46 FP-guard images | dataset (real + synthetic mixes) | 2.7 | 8.1 | 17.7 | 28.5 | 60 directories, one per symbology; `falsepositives-1/2` |
| BarBeR 1/8 subset | 1093 labels | dataset (real) | 36 | 67* | 133* | about 4 min | 12 photo sources; FP on real photos |
| BarBeR full | 8585 labels, 8748 images | dataset (real) | 297* | 549* | 1071* | about 32 min | all of the above, 8x the FP exposure |
| Video (`video-bench`, 12 clips, 3 engines, about 3000 frames at 1080p) | 831 expected | dataset (video) | | | | **285 s** (tracker 159 s, fast 33 s, balanced about 95 s, decoding about 20 s) | the only gate on `Tracker`; 1080p frames |
| `rayon_determinism`, `allocation` | | e2e test, not a gate | | | | | |

`*` from BASELINES, shared machine.

## 4. Overlap analysis

### 4.1 What each gate caught, from the history

The baselines were rewritten many times (`git log -- tools/bench/baselines`), almost all of them to record improvements or new classes. Real catches, from [BASELINES](BASELINES.md), [decisions](decisions.md) and [testing](testing.md):

| Case | Caught by | Would a cheaper test have caught it? |
|---|---|---|
| Plain blend for edge relocation: lost the `upca-3` group of zxing-samples, synth-2024 `illumination` ITF and video `EAN_13-distance` ([BASELINES](BASELINES.md#linear-reader-fixes-2026-10-08)) | three different gates: zxing-samples, synth seed 2024, video | no; each loss was on a different dataset (the synth loss was in seed 2024; nothing says seed 1 saw it) |
| Exhaustive-only UPC-E misread of the clean EAN-13 `EAN13_09_0093.jpg` (BASELINES, BarBeR FP triage) | full BarBeR at Exhaustive (the file is in the 1/8 subset: its hash gives share 0) | no; 0 FP on all other sets at that effort |
| Code 39 Full ASCII label mismatch that only appeared when two branches merged ([testing](testing.md#merging-baselines-after-combining-branches)) | merged-tree rerun of the gates | no |
| Code 39 extra-line safeguard cost one video read (29 -> 28) (the Code 39 quiet-zone entry in [decisions](decisions.md), 2026-10-08) | video gate | no; tracker/frame-level only |
| 42 first-run BarBeR false positives, all ground truth or normalisation (loader and override fixes, `legacy_payload` and others) | BarBeR Balanced | no; real label quirks |
| pdf417-small-module-size Code 128 "FPs" `Q8`, `NUTWR`: genuine unlabelled symbols | Dynamsoft pdf417 | no |

None of the recorded catches is attributable to synth seed 1 alone, to a second effort alone on the cheap sets, or to a unit test duplicating a gate.

### 4.2 Gate pairs

**Synth seed 1 against seed 2024.** Same generator, same 11 classes, same 145 (class, symbology) cells at every effort; seed 1 has 487 expected symbols, seed 2024 has 1311 (2.7 x). At Exhaustive seed 1 misses in 4 cells and seed 2024 in 11; the one cell seed 1 misses that seed 2024 does not (Data Matrix) is a count of one image. A real regression in a reader moves seed 2024 in at least the cells it moves seed 1 (more samples), and seed 1 is only worth keeping as a fast smoke gate (1.8 s at Fast, 4.9 s at Balanced). It is not worth three efforts: seed 1 Fast and Exhaustive add 12.5 s and no coverage. Its generated dataset must stay (the Python tests read `data/synth`).

**Efforts on each set.** Fast -> Balanced is the big step on every real set (e.g. zxing 564 -> 676, BarBeR 5384 -> 6336, 37 of 49 zxing cells change). Balanced -> Exhaustive is small: synth seed 1: 1 cell, seed 2024: 5 cells (+1..2 reads each), challenging 2 of 9 cells, zxing 6 of 49, BarBeR subset 9 of 41, full 22 of 55, and one full-set cell (and one zxing cell) where Exhaustive reads **fewer** than Balanced, so Exhaustive does not strictly contain Balanced. Exhaustive is also where the only real misdecode found by BarBeR appeared. Consequences: the three efforts are not interchangeable and each one needs its own gate wherever it is cheap; only on the expensive sets is a choice needed (section 6).

**BarBeR subset against full.** The subset (hash share 1/8, 1093 labels, 41 of the 55 (source, symbology) cells) keeps all 12 sources, but 24 of its 41 cells have fewer than 8 expected labels (they cover 550 of 8585 labels in the full set), so a loss of a few reads in a small cell is invisible, and 14 cells (79 labels, mostly 1 to 9 per cell) are absent altogether. The 12 well-populated cells (at least 20 labels in the subset) hold 7444 of 8585 labels, so a regression in a mainstream (source, symbology) cell of 10 % or more is seen in the subset. For false positives the subset sees 1/8 of the images: a new, independent FP mode that the full set would find once is caught by the subset with probability 1/8 per instance. The full set is therefore the FP gate of record and the subset is a read-count tripwire.

**zxing-samples against the per-symbology unit tests.** Different layers. Unit tests render clean (or mildly perturbed) symbols from encoder crates; zxing-samples are 60 directories of real and hand-made images that break clean-room assumptions (glare, odd quiet zones, rotated, 1 px modules), including 46 images in `falsepositives-1/2` where any read is a failure. Only 29 s for three efforts. Nothing in `cargo test` covers it; keep as is.

**Dynamsoft sets against zxing and BarBeR.** challenging-images, skewed DM and UPS are small real-photo sets with unique content (skewed DM 1324 labels, UPS shipping labels); at 17 + 37 + 52 s for three efforts, merging them with others would save nothing.

**Video against image gates.** The `fast` and `balanced` stateless engines of the video gate read 1080p frames of the same symbologies as the image gates, but are 3000 frames of FP exposure at a resolution the image sets rarely have; the `tracker` engine has no other gate. Tracker-only costs 159 s of the 285 s.

**Gates against e2e tests in `cargo test`.** The gates guard counts per set and effort; `cargo test` guards "this exact synthetic geometry reads" (sweeps) and API behaviour. Where a gate and a test guard the same single behaviour (e.g. Aztec reads at several rotations: the gate sees it through synth Aztec cells, zxing `aztec-1/2`, Dynamsoft challenging and BarBeR Aztec cells), the test's value is the localised failure message, not the detection.

## 5. End-to-end tests inside `cargo test`

| Test | Kind | Runtime (debug / release) | Duplicate of | Verdict |
|---|---|---|---|---|
| `allocation::steady_state_scans_do_not_allocate` | e2e, 6 readers x 5 frames x 3 efforts = 90 measured scans | 80 s / 3.6 s | blank-frame "reads nothing" assertion duplicates `ean13::blank_and_unrelated_images_yield_nothing`, `lib::blank_image_yields_nothing_for_every_format` | Keep (NFR-4 has no other test). Shrink: per-reader rows only on the 800x600 blank and texture frames (drop the 320x240 variants for the five single-reader rows, keep them for the `all` row); run the allocation test in release only (`#[cfg_attr(debug_assertions, ignore)]`, one CI step `cargo test --release -p breader --test allocation`): the counts differ between debug and release builds and the documented NFR-4 numbers are release ones. Aztec and "all" texture bounds (600 / 10 000 / 25 000) are loose by design, so the test guards blank-frame zero and linear/DM zero, not the QR/PDF417/Aztec candidate allocations |
| `datamatrix sample::clean_sweep_succeeds_at_every_stage` | component (detect + layout + snap + sample) over sizes x 7 angles x 3 module sizes | 63 s / 5.7 s | `detect::clean_symbols_across_sizes_angles_and_scales` is the detect-only half of the same grid; `reader::reads_all_sizes_rotated` is the same grid read end to end; `sampled_grid_equals_source` repeats sizes x 5 angles x 3 px | Merge: delete `detect::clean_symbols_across_sizes_angles_and_scales` (its checks are the first stage of `clean_sweep`), cut `clean_sweep` to 4 angles x 2 module sizes; keep `sampled_grid_equals_source` (agreement metric the sweep lacks) and `reads_all_sizes_rotated` (reader level) |
| `aztec reader::reads_compact_and_full_symbols_of_every_size_rotated` | e2e at reader level, 8 layer counts x 4 angles | 50 s / 4.7 s | stage tests `detect/tests.rs` (9) and `sample/tests.rs` (9) plus `decode` tests already cover every layer count at stage level | Shrink to 5 layer counts (-1, -4, 1, 6, 32) x 3 angles; keep one of the two meta assertions as is |
| `aztec reader::reads_in_perspective` | e2e, 6 layer counts x 4 tilts | 41.5 s / 3.7 s | `sample` perspective tests | Shrink to 3 layer counts x 2 tilts |
| `qr micro::reader_tests::qr_codes_are_never_micro_qr`, `rmqr::reader_tests::qr_codes_are_never_rmqr` | e2e FP protections, 6 versions x several damage variants | 17 s + 13.5 s / 1.7 s + 1.4 s | the synth QR classes and BarBeR only show it indirectly (a QR misread as Micro QR would be an FP in a QR cell) | **Keep unchanged** (FP protection); their cost goes away with the dev-profile change |
| `synth tests::generation_is_deterministic_and_annotated` | generator e2e, builds two datasets | 16 s / 0.6 s | none | Keep; free after the profile change |
| `qr_content.rs::eci_charsets_end_to_end` | facade e2e | 0.4 s total | `breader-content` `eci_latin1_family`, `eci_cp437`, `eci_shift_jis_and_cjk`, `eci_unicode`, `eci_ascii_binary_unknown`; `breader-qr` `eci_designators` | Delete (the glue it adds is covered by `mixed_segments_decode_with_their_own_charsets`) |
| `qr_content.rs::undeclared_byte_mode_heuristic_end_to_end` | facade e2e | | `charset::undeclared_heuristic` (same three inputs) | Delete; keep the AIM-id assertion by folding `assert_eq!(s.aim_id, b"]Q1")` into `url_qr_is_classified` |
| `qr_content.rs::gs1_qr_is_parsed` + `url_qr_is_classified` + `mixed_segments...` | facade e2e glue | | partial: `gs1::*`, `classifies_gs1`, `text_from_declared_segments` | Keep (the only tests of QrReader -> content -> facade wiring) |
| `ean13.rs::blank_and_unrelated_images_yield_nothing` | facade e2e | | `lib::blank_image_yields_nothing_for_every_format`, allocation blank frames, `breader-linear` `robustness::random_images_yield_no_symbols` | Delete |
| `ean13.rs` other 3 | facade e2e | 0.03 s | EAN logic is in `breader-linear/ean.rs` (28 tests); these check the default pipeline wiring, stride and add-on option | Keep; merge `reads_a_rendered_ean13...` and `reads_the_same_symbol_upside_down...` into one test (same fixture) |
| `upscale_retry.rs::blank_frames_stay_empty_after_the_retry` | facade e2e | | allocation test's 320x240 blank frames take the retry at Balanced/Exhaustive and assert empty | Delete (keep the other 3: retry policy) |
| `rayon_determinism::parallel_scans_equal_sequential_scans` | e2e (3 pages x 3 efforts) | 0.5 s release | none | Keep; run with the `rayon` feature only |

Which could become stage-level: the two Aztec reader sweeps and the Data Matrix `clean_sweep` already have stage-level twins (`detect`, `sample`, `decode`); keep the reader tests to a handful of cases whose failure message is useful and let the stage tests carry the grid. The facade tests are small and mostly unique glue.

## 6. Proposed tiers

Tiers are defined by how often they run and what concern they protect, not by tool.

### Tier A: every change (local, about 1.5 to 2 minutes with a warm build)

| Item | Time |
|---|---|
| `cargo test --workspace` with the dev profile at opt-level 1 and the trims of section 5 | about 20 s (31 s measured before trims) |
| incremental `cargo build --release -p bench -p video-bench -p synth` | 30 to 60 s |
| synth seed 1, Balanced (smoke) | 4.9 s |
| synth seed 2024, Balanced | 13.8 s |
| Dynamsoft challenging-images, Balanced | 4.9 s |
| Dynamsoft pdf417-small, Balanced | 1.3 s |
| zxing-samples, Balanced (includes the 46 FP-guard images) | 8.1 s |

Gates in A add up to 33 s. False-positive coverage in A: synth `negative` class, zxing `falsepositives-1/2`, real photos in challenging-images, all at Balanced. Python tests run when `crates/breader-py` or the facade API changes (3 s with a built wheel).

### Tier B: merge to main (local before merging, plus CI; about 10 minutes)

| Item | Time |
|---|---|
| everything in A | about 2 min |
| `cargo test` on the CI matrix: first run `cargo test --workspace`, second run only the crates that have the `rayon` feature (`-p breader -p breader-imgproc -p breader-linear -p breader-qr -p breader-datamatrix --features breader/rayon`) | about 46 s per OS image after the profile change |
| synth seed 2024, Fast and Exhaustive (seed 1 stays Balanced only) | 37 s |
| challenging Fast + Exhaustive | 12.6 s |
| skewed DM, 3 efforts | 37 s |
| UPS, 3 efforts | 52.5 s |
| pdf417-small, Fast + Exhaustive | 3.4 s |
| zxing-samples, Fast + Exhaustive | 20.4 s |
| BarBeR 1/8 subset, Balanced + Exhaustive (Fast moves to tier C) | about 3.3 min (67 + 133 s) |
| video, **tracker engine only** (needs the `video-bench` change in section 7) | 159 s |
| pytest (needs the wheel) | 3 s |

Total about 10 minutes (about 7.5 without the video). Every effort is gated on every cheap real set here, so Exhaustive-only FPs on the sets where they matter are still caught at merge.

### Tier C: phase end, release, or a change that raises false-positive exposure (about 35 to 42 minutes, rarely)

C is today's `--all` and is deliberately kept; the saving comes from running it rarely, not from shrinking it, because the full set is the only 8748-photo FP gate.

| Item | Time |
|---|---|
| everything in B except the BarBeR subset | about 7 min |
| BarBeR full, Balanced + Exhaustive | 27 min (9 + 18, shared machine) |
| video, full (tracker + fast + balanced) | +2 min over B |
| BarBeR full, Fast (release or Fast-path changes only) | +5 min |
| fuzz (nightly in CI, 85 min, off the developer's machine) | n/a |

When to run C1 (about 35 min: full Balanced + Exhaustive): a new default symbology, a relaxed guard or acceptance rule (quiet zone, agreement lines, length list, triplet geometry such as D-040), any change to aggregation/dedup, a change of default features or escalation. Otherwise B. C2 (about 42 min: C1 + full Fast + full video) at a phase end or before a release. Measuring the full BarBeR run on an idle machine with 12 rayon threads instead of 4 on a shared one is the cheapest way to shrink C; I did not do it (memory rule) and expect a 2x to 3x gain from the load figures, which is a guess.

### Summary of the four numbers

| | Today | Proposed |
|---|---|---|
| inner loop (tests + one cheap check) | 4 min 13 s + 68 s gates, about 5.5 min | A: 1.5 to 2 min |
| merge | about 5.5 min locally (cargo test, synth) + whatever the developer remembers; CI debug matrix about 9 min per OS image | B: about 10 min including real-photo sets, the subset and the tracker video; CI test job about 3 min per OS image |
| phase / release | `--all` 40 min, recommended "before changing a reader" | C1 35 min only for FP-exposure changes, C2 42 min at phase end |

## 7. Ranked changes

Savings are measured unless marked estimate. "FP risk" says whether the change weakens the zero-false-positive protection.

1. **Optimise the dev/test profile** (`[profile.dev] opt-level = 1`, or at least `[profile.dev.package."breader-*"]` and `breader-imgproc` at 1 to 3). `cargo test --workspace` 4 min 13 s -> 31 s (measured with `CARGO_PROFILE_DEV_OPT_LEVEL=1`); cold build 1 min 44 s. CI runs the suite twice on 4 OS images, so this is the largest single saving by far. Debug assertions and overflow checks stay on. Coverage change: none. FP risk: none.
2. **Stop running `--all` as the default pre-change check**; adopt the tiers: update `docs/testing.md`, `docs/CI.md` and the line in `BASELINES.md` ("Run `--barber-full` before changing a reader") to say tier B per change, C1 for FP-exposure changes, C2 at phase end. Saves about 30 min per ordinary reader change. FP risk: low if C1 is applied to the listed change kinds; the subset sees an FP with probability 1/8, so a change that could add FPs without being on the C1 list is the residual risk.
3. **Give `tools/check-regression.sh` an `--efforts` list and tier presets** (`--tier A|B|C1|C2`), including `--video-engines tracker` passed to video-bench. Today every flag runs all three efforts, so "BarBeR Exhaustive only" is not expressible. Needed to make 2 usable.
4. **`video-bench --check` with `--engines tracker`**: ignore baseline entries of engines not run (today a missing entry fails the check). Tracker only: 285 s -> 159 s at tier B; the full video stays in C. FP risk: none for B (C still gates all three engines).
5. **Gate synth seed 1 at Balanced only; seed 2024 at all three efforts.** Drops 12.5 s of CI/local time and one redundant baseline pair (`synth-seed1-fast.json`, `synth-seed1-exhaustive.json`). Small saving, mostly bookkeeping; the real argument is that seed 1 has never been the sole catcher. Keep the dataset for pytest. FP risk: none (seed 2024 has the `negative` class and 2.7x the images).
6. **Run the allocation test in release only** (`#[cfg_attr(debug_assertions, ignore)]` plus one CI step `cargo test --release -p breader --test allocation`) and drop the 320x240 frames from the five single-reader rows. 80 s -> about 2 s debug-equivalent; with change 1 it is already 6.8 s, so this is second order. Coverage change: the 320x240 per-reader rows (the `all` row keeps both sizes). FP risk: none.
7. **Merge/delete redundant tests** (no runtime effect, removes duplicated assertions; each concern remains covered as named in section 5):
   - delete `datamatrix detect::tests::clean_symbols_across_sizes_angles_and_scales` (first stage of `sample::clean_sweep_succeeds_at_every_stage`);
   - delete `ean13.rs::blank_and_unrelated_images_yield_nothing` (covered by `lib::blank_image_yields_nothing_for_every_format`, the allocation blank frames and the linear random-image tests);
   - delete `upscale_retry.rs::blank_frames_stay_empty_after_the_retry` (allocation test's 320x240 blank frames take the retry);
   - delete `qr_content.rs::eci_charsets_end_to_end` and `undeclared_byte_mode_heuristic_end_to_end` (covered by `breader-content` `eci_*`, `undeclared_heuristic`, and `breader-qr` `eci_designators`);
   - merge the first two `ean13.rs` render tests.
8. **Shrink the sweeps that remain slow** (release seconds saved are small, debug seconds saved are large; with change 1 they are cheap anyway): Data Matrix `clean_sweep` to 4 angles x 2 module sizes, Aztec `reads_compact_and_full...` to 5 layer counts x 3 angles, Aztec `reads_in_perspective` to 3 layer counts x 2 tilts. About 40 % of the three tests' time. Coverage change: fewer grid points; the stage tests still run every layer/size. FP risk: none (these are positive reads).
9. **Narrow the second CI `cargo test` run** to the five crates with the `rayon` feature (`breader`, `-imgproc`, `-linear`, `-qr`, `-datamatrix`); `aztec`, `pdf417`, `ecc`, `content`, `core`, `synth`, `bench` have no feature-gated code, so the second run repeats their results. About 40 % of the second run (estimate). Also build the 4 Python wheels and run pytest on main/tags and when `crates/breader-py` or the facade changes, not on every push (the pytest itself is 3 s). Coverage change: none.
10. **BarBeR efforts per tier**: subset at Balanced + Exhaustive in B (Fast moves to C: -36 s); full set Balanced + Exhaustive in C1, Fast only in C2. Saves 5 minutes in C1. FP risk: small and stated: full Fast is not run in C1. Fast has the fewest passes, but Fast has its own paths (smaller scan sets, no up-scale retry) and Exhaustive does not strictly contain Balanced (one cell reads fewer), so the Fast-only FP class is not strictly excluded between C2 runs. It is still covered by every other dataset at Fast in B and by the subset at Fast in C.

Further, lower ranked: weight fuzz seconds per target (pipeline, `aztec_scan`, `qr_grid`, `linear_runs` 600 s; pure components `gs1_parse`, `charset_decode`, `rs*_decode` 100 s) for the same 85 minutes; add a `--symbology`/`--only` filter to `bench` for read-count checks of the symbology a change touched, while the FP check still uses all images (not implemented; the cost is that this is only a convenience, FP needs the whole set); measure the full BarBeR run on an idle machine with 12 threads.

### Do not weaken (false-positive protection)

- Keep the zero-FP check in every gate; keep `--check` failing on any FP.
- Keep `falsepositives-1/2` (46 images) and the synth `negative` class in tier A.
- Keep both `qr_codes_are_never_micro_qr` and `qr_codes_are_never_rmqr`, the `*_never_misread` / `never_wrong_*` / `random_*_never_panic_or_succeed_spuriously` tests (QR, Aztec, DataBar, Codabar, ECC); they are cheap in release and become cheap in dev after change 1.
- Keep full BarBeR at Exhaustive and Balanced for FP-exposure changes; do not replace it with the subset (1/8 of the photos, 24 of 41 cells with fewer than 8 labels).
- Keep Exhaustive in the cheap-set gates in tier B; the only recorded real misdecode appeared at Exhaustive only.
- Do not gate the video `fast`/`balanced` engines away permanently; they are the only 1080p frame-level FP exposure (3000 frames).

### Documentation inconsistencies found

- `docs/testing.md`: full BarBeR "about 5 to 16 minutes per effort"; `docs/CI.md` (via the script header): "about 10 minutes per effort"; `BASELINES.md`: 297 / 549 / 1071 s; the owner: 5 / 9 / 18 minutes. Use one set of numbers after the next idle-machine measurement.
- `docs/testing.md` does not mention the video gate's cost (285 s, the main part of `--dynamsoft`).
