# Testing

What protects the project from regressions and false positives, and how to run and maintain each layer. CI wiring is in [CI](CI.md); dataset details in [datasets](datasets.md).

## Layers

| Layer | Where | Run |
|---|---|---|
| Unit and integration tests | `crates/*/src` (`#[cfg(test)]`), `crates/breader/tests` | `cargo test --workspace` and `cargo test --workspace --features breader/rayon` |
| Python tests | `crates/breader-py/tests` (against the installed wheel) | `pytest` after `maturin develop --release`; see [breader-py](../crates/breader-py/README.md) |
| Regression gate | `tools/bench`, `tools/video-bench`, baselines in `tools/bench/baselines/` | `tools/check-regression.sh [--dynamsoft] [--zxing-samples] [--barber] [--barber-full] [--all]` |
| Fuzzing | `fuzz/` (nightly toolchain, not a workspace member) | see [fuzz/README.md](../fuzz/README.md) |
| Micro-benchmarks | `criterion` in `breader-imgproc` and `breader-ecc` (`benches/`), `tools/bench` `stages` binary | `cargo bench -p breader-imgproc` |

Test symbols are produced by encoder crates used as dev-dependencies only (for example `qrcode`); the decoder sources of those crates or of other readers are never read ([clean-room](contributing.md#clean-room-rules)). Misdecode protections (the zero-false-positive policy) get a test each, with the failing case that motivated them.

## Gates

A gate compares a run with a versioned baseline and fails CI on any regression. One baseline file per dataset and effort; the effort is part of the key.

| Gate | Dataset | Baselines | Runs in |
|---|---|---|---|
| synth seed 1 / seed 2024 | generated | `synth-seed1-*`, `synth-seed2024-*` | every push (`ci.yml` `regression`) |
| Dynamsoft challenging / skewed Data Matrix | `data/dynamsoft/...` | `dynamsoft-challenging-*`, `dynamsoft-skewed-*` | weekly or manual (`datasets.yml`), `--dynamsoft` locally |
| Dynamsoft videos | `data/dynamsoft/video-based-testing` | `dynamsoft-video.json` | same |
| zxing-cpp samples | `data/zxing-samples/test/samples` | `zxing-samples-*` | `datasets.yml` `zxing-samples` job, `--zxing-samples` locally |
| BarBeR, 1/8 subset | `data/barber` (manual download, [datasets](datasets.md#barber)) | `barber-subset-*` | locally only, `--barber` |
| BarBeR, full set | same, 8748 images | `barber-*` | locally only, `--barber-full` and `--all` |

### How it works

`bench <dataset> --effort E --check <baseline>` loads the dataset, decodes every image with the default pipeline and compares per dataset group and per symbology (video-bench: per video). It exits 1 and prints a diff when:

- a read count is lower than the baseline,
- any false positive appears (even when reads went up),
- the expected count differs (the dataset or its labels changed), or a baseline entry is missing from the run.

Higher read counts and entries not yet in the baseline are only notices. Latency is reported, never gated. `--check` and `--write-baseline` require `--engine breader`, the default escalation and the GT overrides. The implementation is `tools/bench/src/gate.rs`.

### The BarBeR gate

BarBeR has no redistributable license, so it is never run in CI. A full run takes about 5 to 16 minutes per effort (8748 photos, some of 20 megapixels), too long for every local check, so there are two gates over the same ground truth ([datasets](datasets.md#barber)):

- `--barber`: a deterministic 1/8 subset (`bench data/barber --subset 8`: the images whose FNV-1a hash of the file name is divisible by 8, about 1100 images, all twelve sources represented), baselines `barber-subset-<effort>.json`, about 4 minutes for the three efforts.
- `--barber-full` (and `--all`): the whole set, baselines `barber-<effort>.json`.

Both skip with a message when `data/barber` is missing. The subset baselines are recorded with `--subset 8`, so the expected counts in them differ from the full ones; the two sets are checked against their own files. The group of a baseline is one BarBeR source dataset, the items are the symbologies, so a regression is localised to a source. Undecodable regions are not expected reads and mask false positives on their polygon, which is why this gate can hold the zero-false-positive rule on photos whose ground truth is partly unknown ([datasets](datasets.md#loader)). Rewrite both sets after a change that moves reads:

```sh
for e in fast balanced exhaustive; do
  cargo run -r -p bench -- data/barber --effort $e --write-baseline tools/bench/baselines/barber-$e.json
  cargo run -r -p bench -- data/barber --subset 8 --effort $e --write-baseline tools/bench/baselines/barber-subset-$e.json
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
tools/check-regression.sh                  # synthetic sets
tools/fetch-datasets.sh                    # once
tools/check-regression.sh --all            # also Dynamsoft (needs ffmpeg), zxing samples and the full BarBeR set
tools/check-regression.sh --barber         # only the quick 1/8 BarBeR subset on top of the synthetic sets
```

Measure latency only on an idle machine and compare builds in interleaved runs; concurrent jobs distort timings by up to 2x.

## Fuzzing

Targets, running, corpus handling, crash triage and "adding a target" are in [fuzz/README.md](../fuzz/README.md). The corpus is not versioned; CI keeps it in a cache between nightly runs ([CI](CI.md)). A crash is fixed at the root cause with a regression unit test in the owning crate; never by relaxing validation.

## False-positive policy and triage

Requirement NFR-1 ([REQUIREMENTS](REQUIREMENTS.md)): zero wrong results on every dataset, including negatives. Any new false positive blocks the change, whatever it gains. When in doubt, return nothing.

When a gate reports a false positive (a returned symbol not in the labels), classify it before touching the decoder:

1. **Normalization.** The read is the same symbol the label describes in another notation: UPC-A vs EAN-13 with a leading 0, UPC-E expanded, Code 39 vs its Full ASCII expansion, charset-decoded text vs UTF-8 label, a label that is UTF-8 read as Latin-1. Fix the *matching* in `tools/bench/src/matching.rs` with a general rule and a unit test. Not a decoder change.
2. **Unlabelled but real.** A valid, clearly visible symbol the labels omit (partial annotations, a second nested code, a symbol of an unsupported family carried inside a supported one). Verify on the image and add a label override per the [overrides policy](datasets.md#overrides-policy). Check the decode is checksum or Reed-Solomon valid and the payload is sensible next to its printed caption.
3. **Real misdecode.** Wrong data, or a symbol that is not there. Fix the decoder: add the missing validation (quiet zone, check character, agreement, conflict removal, length list), keep a minimal repro as a unit test, and confirm the read count of the other gates does not drop. Never loosen the gate or add the read to a "known" list.

Write the finding in the triage docs when it is non-obvious ([FAILURES](FAILURES.md), [FAILURES-ZXING-SAMPLES](FAILURES-ZXING-SAMPLES.md) show the format: root-cause classes, ranked fixes, measured gain, status). A misdecode protection that costs reads needs an owner decision recorded in [decisions](decisions.md) (the Code 39 extra-line safeguard is an example: it cost one video read and 0 false positives were measured without it as well).
