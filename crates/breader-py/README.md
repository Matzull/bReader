# breader

Fast barcode and QR code reader for Python, written in Rust. No runtime dependencies (numpy and Pillow are optional). Ships as a single `abi3` wheel for Python 3.9+.

Supported symbologies: QR Code, Micro QR Code (M1 to M4), rMQR Code, Data Matrix (ECC 200, including rectangular and DMRE sizes), PDF417 (including compact), Aztec Code (compact, full-range and Runes), EAN-13, EAN-8, UPC-A, UPC-E (with EAN-2/5 add-ons), Code 128, Code 39, Code 93, ITF (Interleaved 2 of 5), Codabar and GS1 DataBar (Omnidirectional, Limited, Expanded, one row and stacked). GS1 element strings are parsed for QR/Code 128 FNC1 payloads, GS1 DataBar and GTIN symbologies.

## Install

```
pip install breader
```

## Quick start

```python
import breader

for s in breader.read("label.png"):          # path, numpy array, PIL image or bytes
    print(s.symbology, s.text, s.corners)

# Reuse a Reader for many images; use one Reader per thread
reader = breader.Reader("balanced", symbologies=["qr", "ean13"])
symbols = reader.scan(gray_array)             # (H, W) uint8 numpy array
symbols = reader.scan(rgb_array)              # (H, W, 3) -> rgb8, (H, W, 4) -> rgba8
symbols = reader.scan(raw_bytes, width=640, height=480, format="nv12")
```

## API

- `breader.read(image, *, format=None, width=None, height=None, stride=None, **options) -> list[Symbol]`: convenience wrapper. `image` is a path, a PIL image (converted to grayscale), or any buffer. Without options it reuses a per-thread default `Reader`.
- `Reader(effort="balanced", symbologies=None, *, ...)`: see the options table. `Reader.options` returns the effective configuration as a dict.
- `Reader.scan(image, *, format=None, width=None, height=None, stride=None) -> list[Symbol]`: `image` is any object with the buffer protocol and unsigned 8-bit items.
  - 2D `(H, W)`: luma8. 3D `(H, W, 3)`: rgb8, `(H, W, 4)`: rgba8 (or `format="bgra8"`), `(H, W, 1)`: luma8. BGR (3 channels) is not supported; reverse the channel axis first.
  - 1D buffers (`bytes`, `bytearray`, `memoryview`) need `width` and `height`; `format` defaults to `luma8` and may be `rgb8`, `rgba8`, `bgra8`, `nv12`, `nv21` or `i420` (only the luma plane is used for YUV). `stride` is the row pitch in bytes and is accepted for 1D buffers only.
  - Any strides are accepted (`arr[::2, ::2]`, Fortran order, ...). Bad shapes, dtypes or sizes raise `ValueError`.
  - The GIL is released while scanning.
- `Reader.scan_file(path)`: decode an image file with the Rust `image` crate (PNG and JPEG; for other formats decode with Pillow and use `read`) and scan it. Raises `OSError` on failure.
- `Symbol`: `symbology` ("qr", "ean13", "ean8", "upca", "upce", "code128", "code39", "itf", "datamatrix", "pdf417", "aztec", "codabar", "micro-qr", "rmqr", "databar", "databar-limited", "databar-expanded", "code93"), `data` (bytes), `text` (str or None), `content` ("text", "binary", "url", "gs1", "unparsed"), `gs1` (list of `(AI, value)` or None), `gs1_hri` (e.g. `"(01)09506000134352"` or None), `corners`, `orientation`, `aim_id`, `quality`, `meta`. Symbols compare and hash on `(symbology, data)`.
  - QR `meta`: `kind="qr"`, `version`, `width`, `height` (modules per row and column), `ec_level`, `mask`, `errors_corrected`, `eci`, `mirrored`. Micro QR uses the same `meta`: `version` 1 to 4 for M1 to M4, `mask` 0 to 3, `ec_level` `"DetectionOnly"` for M1 (error detection, no correction). rMQR uses it too: `version` 1 to 32 (the version indicator plus one, R7x43 is 1 and R17x139 is 32), `width` and `height` the module counts, `ec_level` `"M"` or `"H"`, `mask` always 0 (one fixed pattern), `eci` as for QR.
  - 1D `meta`: `kind="linear"`, `addon`, `checksum_verified` (always true for Code 93, whose two check characters are verified and left out of `data`), `lines_agreeing`, `codabar_start_stop` (`(start, stop)` characters of a Codabar symbol, which `data` leaves out; None otherwise), `databar_segments` (symbol characters of a GS1 DataBar Expanded row, check character included; None otherwise), `databar_rows` (rows of a Stacked, Stacked Omnidirectional or Expanded Stacked symbol; None for one row).
  - Data Matrix `meta`: `kind="datamatrix"`, `rows`, `cols`, `errors_corrected`, `mirrored`, `eci`, `structured_append`, `reader_init`.
  - Aztec `meta`: `kind="aztec"`, `compact`, `layers`, `data_codewords`, `ec_codewords`, `errors_corrected`, `erasures`, `mirrored`, `rune` (None, or the Rune value 0 to 255), `reader_init`, `eci`.
  - PDF417 `meta`: `kind="pdf417"`, `rows`, `cols`, `ec_level`, `errors_corrected`, `erasures`, `compact`, `reader_init`, `macro` (None, or a dict with `segment_index`, `file_id`, `last_segment`, `file_name`, `segment_count`, `timestamp`, `sender`, `addressee`, `file_size`, `checksum`).

## Options

| Option | Default | Meaning |
|---|---|---|
| `effort` | `"balanced"` | `"fast"` (single pass, for video), `"balanced"`, `"exhaustive"` (all binarizers, both polarities) |
| `symbologies` | all | Name or list of names to look for |
| `try_invert` | by effort | Also try light-on-dark codes |
| `escalation` | by effort | `"all"`, `"found"` (stop at first hit), `"stable"` |
| `max_symbols` | `0` | Stop after this many symbols (0 = unlimited) |
| `time_budget_ms` | `None` | Checked between passes; a pass in progress is not interrupted |
| `upscale_max_side` | `None` | Frames up to this size (longer side) that yield nothing are rescanned 3x up-scaled; `None` = 600 for balanced/exhaustive, off for fast; 0 disables |
| `min_line_agreement` | `2` | Scanlines that must agree on a 1D result |
| `ean_addon` | `"ignore"` | `"ignore"`, `"optional"` (report add-on), `"required"` |
| `code39_full_ascii` | `False` | Decode Code 39 Full ASCII |
| `code39_checksum` | `False` | Require a valid mod-43 check character |
| `code39_min_length` | `3` | Minimum Code 39 length |
| `itf_lengths` | `()` | Accepted ITF digit counts; empty = any even length of at least `itf_min_length`, every one but 14 needing one more agreeing scanline |
| `itf_min_length` | `6` | Minimum ITF length |

## Threading and performance

A `Reader` can be shared between threads (calls are serialized by an internal lock), but it keeps scratch memory between scans, so create one `Reader` per thread to scale; scans in separate Readers run in parallel because the GIL is released.

`scan` copies the pixel data once (zero copy for `bytes`), so the data cannot change while the GIL is released. The binding adds roughly 0.5 ms per megapixel-class image over the Rust library; typical decode time is 10-25 ms per image on a laptop CPU. Use `effort="fast"` for video streams.

## Development

```
python -m venv .venv && . .venv/bin/activate
pip install maturin pytest numpy pillow segno
maturin develop --release
pytest
```
