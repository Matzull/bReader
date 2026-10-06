# Baselines

First measurement, 2026-10-06. AMD Ryzen 5 5500U, release build, single image at a time. Latency = decode call only (image already loaded as luma8), in ms per image.

Reproduce:

```
tools/fetch-datasets.sh
cargo run -r -p synth -- data/synth
cargo run -r -p bench --features baselines -- data/dynamsoft/challenging-images --engine all
crates/breader-py/.venv/bin/python tools/bench/zxingcpp_bench.py data/dynamsoft/challenging-images
```

Engines: rxing (`TryHarder`, multi), rqrr (QR only), zxing-cpp 3.1.1 (Python package, default options).

## Dynamsoft `challenging-images`

458 expected symbols in v1 symbologies (56 Data Matrix / PDF417 excluded).

| Symbology | Expected | rxing | rqrr | zxing-cpp |
|---|---|---|---|---|
| QR | 234 | 120 | 22 | 220 |
| EAN-13 | 84 | 19 | 0 | 17 |
| EAN-8 | 1 | 1 | 0 | 1 |
| UPC-A | 74 | 5 | 0 | 70 |
| Code 128 | 60 | 23 | 0 | 27 |
| Code 39 | 1 | 1 | 0 | 1 |
| ITF | 4 | 2 | 0 | 4 |
| **Read rate** | 458 | 37.3 % | 4.8 % | 74.2 % |
| **False positives** | | 12 | 0 | 2 |
| Latency p50 / p95 | | 104.8 / 2129.9 | 20.5 / 248.8 | 23.1 / 329.7 |

## Synthetic (`tools/synth`, seed 1, 20 images per class)

228 expected symbols; the `negative` class has none.

| Class | rxing | rqrr | zxing-cpp |
|---|---|---|---|
| clean | 100 % | 30 % | 100 % |
| rotated | 55 % | 30 % | 55 % |
| perspective | 80 % | 35 % | 80 % |
| blur | 85 % | 30 % | 90 % |
| low-res | 100 % | 5 % | 100 % |
| illumination | 75 % | 20 % | 75 % |
| inverted | 0 % | 0 % | 35 % |
| noise | 100 % | 30 % | 100 % |
| multi | 88.2 % | 8.8 % | 88.2 % |
| **Total** | 78.5 % | 18.4 % | 82.0 % |
| **False positives** | 49 | 0 | 10 |
| Latency p50 / p95 | 55.9 / 391.8 | 20.9 / 38.6 | 13.2 / 36.5 |

No engine produced false positives on the `negative` class. Most false positives are partial ITF reads (12 of 14 digits) and spurious UPC-E/EAN-8 inside other 1D symbols. breader requires agreement between several scanlines, a strict length list for ITF and a check for overlapping conflicts, which are aimed at these cases.

## Proposed v1 targets (pending confirmation)

| Metric | Target |
|---|---|
| False positives | 0 on every dataset, including `negative` |
| Read rate, challenging-images | ≥ 74 % (zxing-cpp level) |
| Read rate, synth | ≥ 82 % overall, ≥ zxing-cpp in every class |
| Latency, Balanced | p50 ≤ zxing-cpp on both datasets |
| Latency, Fast, 1080p frame | p50 ≤ 10 ms |
