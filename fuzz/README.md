# Fuzzing

[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer) targets for
the decoders. This directory is not a workspace member: it needs the nightly
toolchain, while the rest of the repository builds on stable.

## Setup

```sh
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --locked
```

## Targets

| Target             | What it exercises                                                                 |
|--------------------|-----------------------------------------------------------------------------------|
| `pipeline`         | `Reader::scan` / `scan_pixels` on images up to 256x256: effort, inversion, symbology subset, pixel format, stride |
| `qr_grid`          | QR `decode_grid` on arbitrary module grids (valid and invalid dimensions)         |
| `microqr_grid`     | Micro QR `decode_grid` on arbitrary module grids (valid and invalid dimensions)   |
| `rmqr_grid`        | rMQR `decode_grid` on arbitrary module grids (the 32 sizes and off-size grids)    |
| `linear_runs`      | every 1D decoder (EAN/UPC, Code 128, Code 39, ITF, Codabar, GS1 DataBar) on arbitrary run-width sequences, both directions and polarities |
| `linear_profile`   | edge detection plus all 1D decoders on arbitrary gray-level profiles              |
| `dm_grid`          | Data Matrix grid and codeword-stream decoding on arbitrary input                  |
| `gs1_parse`        | `gs1::parse` / `gs1::hri`                                                         |
| `charset_decode`   | `charset::decode` for every `Charset`, including arbitrary ECI numbers            |
| `rs_decode`        | Reed-Solomon `decode` over both fields; a successful decode must yield a codeword |
| `rs929_decode`     | Reed-Solomon `decode` over GF(929): arbitrary blocks, and valid blocks with injected errors and erasures; a successful decode must yield a codeword and an exact correction count |
| `rs2m_decode`      | Reed-Solomon `decode` over the five Aztec fields GF(16) to GF(4096): arbitrary blocks, and valid blocks with injected errors and erasures; a successful decode must yield a codeword and an exact correction count |
| `aztec_grid`       | Aztec module grids (valid and arbitrary sides, transposed), bare high-level bit streams, and valid symbols and Runes built around arbitrary bytes (reaches the text layer) |
| `aztec_scan`       | The whole Aztec reader (bullseye search, core fit, sampling, decoding) on arbitrary images and on valid symbols with flipped pixels |
| `pdf417_codewords` | PDF417 codeword matrices: arbitrary rows x columns, and valid symbols built around arbitrary data codewords (reaches the compaction layer) |
| `pdf417_line` | PDF417 start pattern search, codeword walk and grid solver on arbitrary run widths of one line |
| `content_pipeline` | `Dedup`, `TextDecoder` and `ContentClassifier` on synthetic symbols with arbitrary segments |

Input layouts are documented at the top of each file in `fuzz_targets/`. The
internal entry points are exposed through `#[doc(hidden)] pub mod fuzzing` in
`breader-qr`, `breader-linear`, `breader-datamatrix`, `breader-pdf417` and `breader-aztec`; they are thin wrappers and not public API.

## Running

```sh
cd fuzz
cargo +nightly fuzz run pipeline -- -max_total_time=300
```

`cargo fuzz run` writes new interesting inputs into `corpus/<target>/`. The
corpus is not versioned (`corpus/`, `artifacts/`, `target/` and `coverage/` are
gitignored); CI keeps it in its cache between nightly runs, and locally it grows
across runs. Run `cargo +nightly fuzz cmin <target>` occasionally to shrink it.

`-max_len=<n>` raises the input size limit (the `pipeline` target needs about
65 KiB for a 256x256 image).

Baseline (5 minutes each, `-rss_limit_mb=2048 -timeout=10`, no findings):
`pipeline` 60k execs (~200/s), `charset_decode` 30M, `content_pipeline` 13M,
`gs1_parse` 5.4M, `linear_runs` 1.7M, `rs_decode` 1.4M, `linear_profile` 1.4M,
`qr_grid` 0.5M.

Coverage: `cargo +nightly fuzz coverage <target> <corpus dir>`.

## Triage

A crash is saved as `fuzz/artifacts/<target>/crash-<hash>` (or `timeout-`,
`oom-`).

1. Reproduce: `cargo +nightly fuzz run <target> artifacts/<target>/crash-<hash>`
2. Minimize: `cargo +nightly fuzz tmin <target> artifacts/<target>/crash-<hash>`
3. Print the input as bytes (`xxd`), decode it by hand to the arguments of the
   `fuzzing::*` function, and write a regression unit test in the owning crate
   that reproduces it with those arguments.
4. Fix the root cause. Decoders must not panic, loop unboundedly or accept
   wrong data; never fix a crash by relaxing validation.
5. Re-run the target from the minimized input, then for a few minutes from the
   seed corpus.

## Adding a target

1. Add `fuzz_targets/<name>.rs` with `#![no_main]` and a `fuzz_target!`.
2. Register it as a `[[bin]]` in `fuzz/Cargo.toml`.
3. If it needs an internal function, add a thin wrapper to the owning crate's
   `fuzzing` module using plain types.
4. Add a few small seeds in `corpus/<name>/` (generated from real data, a few
   KiB in total).
5. Assert cheap invariants (finite coordinates, non-empty payloads, a
   corrected Reed-Solomon block is a codeword) in addition to "does not panic".
