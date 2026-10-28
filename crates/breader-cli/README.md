# breader-cli

Command line front end of the [breader](../../README.md) reader. The binary is called `breader`. It is a thin layer over the `breader` facade crate (feature `image`): the options are the ones of the [Python bindings](../breader-py/README.md), and the results are the same `Symbol` values.

## Build

```
cargo build --release -p breader-cli            # target/release/breader
cargo build --release -p breader-cli --features rayon
cargo install --path crates/breader-cli          # into ~/.cargo/bin
```

The `rayon` feature forwards `breader/rayon` (multi-threaded kernels inside one image). `--jobs` is independent of it: it scans several images at once on std threads.

## Usage

```
breader scan [OPTIONS] <PATH>...
breader --version
breader --help
```

`PATH` is an image file (PNG or JPEG, whatever its extension), a directory (searched recursively for `.png`, `.jpg` and `.jpeg` files, sorted by path), or `-` for an image read from stdin (once).

| Option | Meaning |
|---|---|
| `--effort fast\|balanced\|exhaustive` | Speed/accuracy preset (default `balanced`) |
| `--symbologies LIST` | Comma separated ids: `qr`, `ean13`, `ean8`, `upca`, `upce`, `code128`, `code39`, `itf`, `datamatrix`, `pdf417`, `aztec`, `codabar`, `micro-qr`, `rmqr`, `databar`, `databar-limited`, `databar-expanded`, `code93` (default: all) |
| `--try-invert` | Also look for light-on-dark symbols (default: by effort) |
| `--max-symbols N` | Stop after N symbols per image (0 = unlimited) |
| `--time-budget-ms N` | Time budget per image, checked between passes |
| `--ean-addon ignore\|optional\|required` | EAN/UPC add-on handling (default `ignore`) |
| `--code39-full-ascii` | Decode Code 39 Full ASCII |
| `--code39-checksum` | Require a valid Code 39 mod-43 check character |
| `--format text\|json` | Output format (default `text`) |
| `-j, --jobs N` | Images scanned in parallel, one `Reader` per thread; 0 = one per CPU (default 1) |

`--name value` and `--name=value` are the same. Output order is the input order for every `--jobs`.

## Output

`--format text` prints one line per symbol, `path<TAB>symbology<TAB>payload`. The payload is the text; backslash, tab, newline, carriage return and other control characters are written as `\\`, `\t`, `\n`, `\r` and `\xNN`, so a symbol is always one line. A payload that is not UTF-8 is written as `hex:` followed by its bytes in hex.

`--format json` prints JSON Lines, one object per symbol:

```
{"path":"label.png","symbology":"qr","text":"hello","data_hex":"68656c6c6f","content":"text","gs1_hri":null,"corners":[[10,10],[110,10],[110,110],[10,110]],"orientation":0,"aim_id":"]Q1","quality":0.97}
```

`text` is `null` unless the payload is UTF-8 (the exact bytes are in `data_hex`); `content` is `text`, `binary`, `url`, `gs1` or `unparsed`; `gs1_hri` is the human readable GS1 form such as `(01)09506000134352` or `null`; `corners` are four `[x, y]` points in image pixels; `orientation` is in degrees; `quality` is informational.

## Examples

```
breader scan label.png
breader scan --symbologies qr,datamatrix --format json photos/ | jq -r .text
breader scan --effort exhaustive --try-invert --jobs 0 scans/
curl -s https://example.org/code.png | breader scan -
```

## Exit status

| Code | Meaning |
|---|---|
| 0 | At least one symbol was found and every input was read |
| 1 | No symbol was found |
| 2 | Usage error, or an input could not be read or decoded (reported on stderr; the other inputs are still scanned, and their symbols printed) |

An input that cannot be read makes the status 2 even when other inputs gave symbols, so a script never mistakes a partial scan for a complete one.

## Notes

- Only PNG and JPEG are decoded (the facade's `image` feature); convert other formats first.
- Images are scanned as luma; there is no region of interest, rotation or tracker option.
