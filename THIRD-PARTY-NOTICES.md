# Third-party notices

breader is licensed under AGPL-3.0-only (see `LICENSE`) or a commercial license (see `LICENSE-COMMERCIAL.md`). The following third-party material is included in its source.

## pdf417 (codeword pattern tables)

`crates/breader-pdf417/src/spec/clusters.rs` contains the PDF417 symbol character tables as transcribed from the `HL_TO_LL` table of the `pdf417` crate 0.2.1 (https://github.com/Sofiman/pdf417).

```
MIT License

Copyright (c) 2023 Sofiane

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## anyd (GS1 DataBar Limited check patterns, development oracle, stacked test symbols)

`crates/breader-linear/src/databar/limited.rs` holds the 89 check patterns of GS1 DataBar Limited (one per checksum value mod 89), written down from the output of the `anyd` crate 0.2.0 (https://github.com/KarpelesLab/anydcode, MIT, Karpeles Lab Inc.; its DataBar encoder is derived from zint and its tests carry BWIPP reference vectors) and checked against BWIPP reference vectors and photographed samples. The crate is also a dev-dependency of `breader-linear` and `tools/synth` to produce test symbols; nothing of it is in the library. D-048 (stacked DataBar) used its Stacked, Stacked Omnidirectional and Expanded Stacked encoders the same way, and its `stacked.rs` was read to learn the row layout of the Expanded Stacked symbols (row guards, mirrored rows, colour of the even rows); no code of it was copied. No zxing-cpp or zxing source was read for D-048.

```
MIT License

Copyright (c) Karpeles Lab Inc.

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## zbar (comparison engine, not included)

zbar (LGPL-2.1) is used only at bench time, as an external black-box library through pyzbar (`tools/bench/py_bench.py --engine zbar`). It is not linked into breader, not vendored and not part of any release artifact; no zbar code, table or structure is used in breader, and its sources were not read.

## barcoders (Code 93 test oracle)

The `barcoders` crate 2.0.0 (https://github.com/buntine/barcoders, MIT or Apache-2.0) is a dev-dependency of `breader-linear` and `tools/synth`. For Code 93 (D-049) its encoder is used only as an independent oracle: a unit test compares the module strings of the 43 basic characters, including the check characters, with those of the specification tables written down from AIM USS Code 93. No table, code or structure of it, and nothing of zxing or zbar, is in the library; its encoder source was not read.
