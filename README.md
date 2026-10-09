# breader

A fast, clean-room barcode and QR code reader written in Rust. No unsafe code, no C dependencies, optional Python bindings and optional multi-threading.

## Supported symbologies

QR Code, Data Matrix (ECC 200, rectangular and DMRE sizes), EAN-13, EAN-8, UPC-A, UPC-E, Code 128, Code 39 and Interleaved 2 of 5 (ITF). Also: ECI and Shift JIS character sets, GS1 element string parsing (Code 128 FNC1, GS1 QR, GTIN symbologies), EAN add-ons, light-on-dark and mirrored codes, and a video tracker. Input can be luma, RGB(A), BGRA, NV12, NV21 or I420.

## Rust

```toml
[dependencies]
breader = { path = "crates/breader", features = ["image"] }  # "image" enables scan_file; "rayon" enables threads
```

```rust
use breader::Reader;

let mut reader = Reader::new();
for symbol in reader.scan_file("label.png")? {
    println!("{} {:?}", symbol.symbology.name(), symbol.text);
}
```

Raw buffers go through `Reader::scan_pixels(data, PixelFormat::Nv12, width, height, stride)`. More in [`crates/breader/examples`](crates/breader/examples).

## Python

```
cd crates/breader-py && maturin develop --release
```

```python
import breader

for s in breader.read("label.png"):
    print(s.symbology, s.text, s.corners)
```

See [crates/breader-py/README.md](crates/breader-py/README.md).

## Documentation

Start at [docs/README.md](docs/README.md): status, a map of all documents by audience, and the design, testing, dataset and contribution guides. Common entry points:

- [docs/GUIDE.md](docs/GUIDE.md): options, effort presets, parallelism, tracker, GS1.
- [docs/symbologies](docs/symbologies/README.md): supported symbologies, status and design per family.
- [docs/BASELINES.md](docs/BASELINES.md): measurements against other engines.
- API reference: `cargo doc --open -p breader`.

## Performance

Release build, AMD Ryzen 5 5500U, one thread, decode call only (details in [docs/BASELINES.md](docs/BASELINES.md)).

| Dataset | Effort | Read rate | False positives | Latency p50 / p95 (ms) |
|---|---|---|---|---|
| challenging-images (392 symbols) | Fast | 71.2 % | 0 | 2.1 / 25.4 |
| challenging-images | Balanced | 88.8 % | 0 | 9.4 / 90.9 |
| challenging-images | Exhaustive | 87.8 % | 0 | 22.2 / 246.0 |
| synth (228 symbols) | Balanced | 97.8 % | 0 | 3.9 / 25.1 |

In the same session, zxing-cpp read 87.0 % of challenging-images with 1 false positive at 23.3 / 330.2 ms (p50 / p95).

## License

Copyright (c) 2026 Marcos Alonso.

breader is dual-licensed:

- GNU Affero General Public License v3.0 only (`AGPL-3.0-only`), see [LICENSE](LICENSE).
- A commercial license for use without the AGPL obligations, see [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md).

## Contributing

External contributions require signing a Contributor License Agreement (CLA), so that the project can keep offering the dual license. Open an issue before sending a large change. How to work in the repository: [docs/contributing.md](docs/contributing.md).
