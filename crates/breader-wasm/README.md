# breader for JavaScript (WebAssembly)

`wasm-bindgen` bindings over the `breader` facade, for browsers and Node. The package is **private**: nothing is published to npm (`"private": true`, name `breader`, `AGPL-3.0-only`).

The browser decodes image files; this package scans pixel buffers: an `ImageData` from a canvas, or luma / RGB / YUV bytes. It is built without the `image` feature and without `rayon` (no threads, no file IO), so scans run on the calling thread. Use a Web Worker for video so the page stays responsive.

## Build

Needs the Rust target and wasm-pack:

```
rustup target add wasm32-unknown-unknown
cargo install wasm-pack --locked
node crates/breader-wasm/build.mjs          # both targets; or: web | nodejs
```

This writes `pkg/` (ES module, `--target web`) and `pkg-node/` (CommonJS, `--target nodejs`) next to the crate, each with `breader.js`, `breader_bg.wasm`, `breader.d.ts` and a `package.json` made private by `build.mjs`. Neither directory is committed. The crate has its own `[workspace]` and is excluded from the root workspace, like `breader-py`.

## Use

Browser (ES module, serve over HTTP):

```js
import init, { Reader } from "./pkg/breader.js";

await init();
const reader = new Reader({ effort: "balanced", symbologies: ["qr", "ean13"] });
const image = canvas.getContext("2d").getImageData(0, 0, canvas.width, canvas.height);
for (const s of reader.scanImageData(image)) {
  console.log(s.symbology, s.text, s.corners);
}
reader.free(); // optional: releases the scratch buffers
```

Node:

```js
const { Reader } = require("./pkg-node/breader.js");
const reader = new Reader();
const symbols = reader.scan(luma, width, height); // Uint8Array, tightly packed rows
```

## API

- `new Reader(options?)`: `options` is a plain object; unknown keys and bad values throw an `Error` with a message that names the option.
- `reader.scanImageData(imageData)`: RGBA (`ImageData`, or any object with `data` (`Uint8ClampedArray` or `Uint8Array`), `width`, `height`).
- `reader.scan(bytes, width, height, format = "luma8")`: `format` is `luma8`, `rgb8`, `rgba8`, `bgra8`, `nv12`, `nv21` or `i420`; rows are tightly packed (stride is `width` times the bytes per pixel; for the YUV formats the luma plane comes first).
- `version()`: the library version.

Options (the Python names in camelCase, see [../breader-py/README.md](../breader-py/README.md)):

| Option | Values | Default |
|---|---|---|
| `effort` | `"fast"`, `"balanced"`, `"exhaustive"` | `"balanced"` |
| `symbologies` | array of `qr`, `ean13`, `ean8`, `upca`, `upce`, `code128`, `code39`, `itf`, `datamatrix`, `pdf417`, `aztec`, `codabar`, `micro-qr`, `rmqr`, `databar`, `databar-limited`, `databar-expanded`, `code93` | all |
| `tryInvert` | boolean (light-on-dark symbols) | decided by the effort |
| `maxSymbols` | integer, 0 for no limit | 0 |
| `eanAddon` | `"ignore"`, `"optional"`, `"required"` | `"ignore"` |
| `code39FullAscii`, `code39Checksum` | boolean | `false` |
| `code39MinLength` | integer >= 1 | `3` |
| `escalation` | `"all"`, `"found"`, `"stable"` | by effort |
| `upscaleMaxSide` | integer >= 0 (0 disables) | by effort |
| `minLineAgreement` | integer >= 1 | `2` |
| `itfLengths` | array of even integers | any even length >= `itfMinLength` |
| `itfMinLength` | integer >= 1 | `6` |
| `timeBudgetMs` | **not supported**: throws | |

A result is a plain object: `{ symbology, text (string or null), data (Uint8Array), content ("text", "binary", "url", "gs1", "unparsed"), gs1Hri (string or null), corners: [{x, y} x4], orientation (degrees), aimId, quality }`. The TypeScript types (`ReaderOptions`, `BarcodeSymbol`, ...) are in `breader.d.ts`.

## Not supported

- `timeBudgetMs`: the scan loop does not read a clock when no budget is set (so the default path runs on `wasm32-unknown-unknown`), but `std::time::Instant` panics there, so a budget is rejected. Bound the work with `effort`, `symbologies` and `maxSymbols`, or scan in a worker and drop frames.
- Image files (PNG, JPEG): decode them with the browser (`createImageBitmap`, canvas) and pass the pixels.
- The video tracker and the per-symbology `meta`: not exposed yet (D-067: until someone asks).
- Multi-threading (`rayon`, which would need `SharedArrayBuffer` and cross-origin isolation).

## Test

```
cargo test --manifest-path crates/breader-wasm/Cargo.toml    # option parsing, native
node crates/breader-wasm/build.mjs nodejs && node crates/breader-wasm/tests/node.mjs
```

The Node test draws a QR code from its module matrix (embedded in the script, no image decoder or binary fixture), scans it in every pixel format and through `scanImageData`, checks the result fields, the options, noise at Exhaustive (no clock or thread is touched) and the error messages.

## Demo

`demo/index.html` and `demo/main.js`: plain JavaScript, no bundler. Pick an image or start the camera; results are listed and the corners drawn.

```
node crates/breader-wasm/build.mjs web
python3 -m http.server -d crates/breader-wasm 8000    # then open http://localhost:8000/demo/
```

The camera needs `https` or `localhost`.
