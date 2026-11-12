# bReader

A fast, clean-room barcode and QR code reader written in Rust. No unsafe code, no C dependencies, optional Python bindings, a WebAssembly build and optional multi-threading.

## Supported symbologies

QR Code, Micro QR Code (M1 to M4), rMQR Code (all 32 sizes), Data Matrix (ECC 200, rectangular and DMRE sizes), PDF417 (including compact), Aztec Code (compact, full-range and Runes), EAN-13, EAN-8, UPC-A, UPC-E, Code 128, Code 39, Interleaved 2 of 5 (ITF), Codabar, Code 93 (Full ASCII) and GS1 DataBar (Omnidirectional, Limited, Expanded). Also: ECI and Shift JIS character sets, GS1 element string parsing (Code 128 FNC1, GS1 QR, GTIN symbologies), EAN add-ons, light-on-dark and mirrored codes, and a video tracker. Input can be luma, RGB(A), BGRA, NV12, NV21 or I420.

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

## Command line

```
cargo install --path crates/breader-cli
breader scan label.png                      # path, symbology, text; one line per symbol
breader scan --format json --jobs 0 photos/ # JSON Lines, directories searched recursively
```

Exit status 0 when a symbol was found, 1 when none was, 2 on a usage error or an unreadable input. See [crates/breader-cli/README.md](crates/breader-cli/README.md).

## C

```
cargo build --release -p breader-ffi   # target/release/libbreader.so and .a (.dylib, .dll elsewhere)
```

```c
#include "breader.h"

BreaderReader *reader = breader_reader_new(NULL);
BreaderResults *results = NULL;
if (breader_scan(reader, pixels, len, width, height, stride, BREADER_PIXEL_LUMA8, &results) == BREADER_OK) {
    for (size_t i = 0; i < breader_results_count(results); i++) {
        const char *text = breader_result_text(results, i, NULL);   /* NULL for binary payloads */
        if (text) puts(text);
    }
}
breader_results_free(results);
breader_reader_free(reader);
```

See [crates/breader-ffi/README.md](crates/breader-ffi/README.md).

## JavaScript / WebAssembly

```
node crates/breader-wasm/build.mjs     # needs wasm-pack and the wasm32-unknown-unknown target
```

```js
import init, { Reader } from "./pkg/breader.js";

await init();
const reader = new Reader({ symbologies: ["qr", "ean13"] });
for (const s of reader.scanImageData(ctx.getImageData(0, 0, w, h))) console.log(s.symbology, s.text);
```

Single-threaded, no image decoding (the browser does that), private package. See [crates/breader-wasm/README.md](crates/breader-wasm/README.md).

## Documentation

Start at [docs/README.md](docs/README.md): status, a map of all documents by audience, and the design, testing, dataset and contribution guides. Common entry points:

- [docs/GUIDE.md](docs/GUIDE.md): options, effort presets, parallelism, tracker, GS1.
- [docs/symbologies](docs/symbologies/README.md): supported symbologies, status and design per family.
- [docs/BASELINES.md](docs/BASELINES.md): measurements against other engines.
- [docs/API-STABILITY.md](docs/API-STABILITY.md): what 1.0 guarantees and what may change in a minor release; [CHANGELOG.md](CHANGELOG.md).
- API reference: `cargo doc --open -p breader`.

## Performance

Release build, AMD Ryzen 7 3700X (latency was measured on a Ryzen 5 5500U until 2026-10-20), library single-threaded, decode call only (details in [docs/BASELINES.md](docs/BASELINES.md#status-2026-10-20-new-machine-ryzen-7-3700x)).

| Dataset | Effort | Read rate | False positives | Latency p50 / p95 (ms) |
|---|---|---|---|---|
| challenging-images (448 symbols) | Fast | 74.3 % | 0 | 5.0 / 40.5 |
| challenging-images | Balanced | 91.7 % | 0 | 20.8 / 146.9 |
| challenging-images | Exhaustive | 94.6 % | 0 | 53.9 / 409.3 |
| synth (511 symbols) | Balanced | 98.8 % | 0 | 5.7 / 48.9 |

In the same session, zxing-cpp read 86.8 % of challenging-images with 0 false positives at 21.0 / 281.4 ms (p50 / p95, through Python).

## License

Copyright (c) 2026 Marcos Alonso.

breader is dual-licensed:

- GNU Affero General Public License v3.0 only (`AGPL-3.0-only`), see [LICENSE](LICENSE).
- A commercial license for use without the AGPL obligations, see [LICENSE-COMMERCIAL.md](LICENSE-COMMERCIAL.md).

## Contributing

External contributions require signing a Contributor License Agreement (CLA), so that the project can keep offering the dual license. Open an issue before sending a large change. How to work in the repository: [docs/contributing.md](docs/contributing.md).
