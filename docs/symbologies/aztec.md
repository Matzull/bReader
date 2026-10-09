# Aztec Code (`breader-aztec`, milestone M10)

Design of the Aztec Code reader (design only, not implemented yet). Aztec is a 2D matrix symbology with a central bullseye finder and no quiet zone requirement. The reader works on the binary plane (`uses_binary() == true`) and handles inversion itself (`handles_inversion() == true`) through a polarity view like the QR `Plane`. Part of the cross-cutting [architecture](../ARCHITECTURE.md); status and read rates are in the [symbology index](README.md).

## Scope

- **In scope:** compact (1 to 4 layers) and full-range (1 to 32 layers) symbols, Aztec Runes (compact, 0 layers), all encodation modes (Upper, Lower, Mixed, Punct, Digit, Binary Shift), FLG(n) for FNC1 (GS1) and ECI, Structured Append headers (parsed into metadata), mirrored symbols, light-on-dark symbols.
- **Out of scope:** none planned.
- **Normative source:** ISO/IEC 24778.
- **Tests:** rxing's Aztec encoder (Apache-2.0) is a dev-dependency, to encode test symbols only. Clean-room: no decoder sources are read.

## Modules and data flow

`spec` (layer and codeword-size tables, mode-message layout, reference grid), `detect` (bullseye candidates), `sample` (core fit, orientation, layer sampling), `decode` (mode message, codeword extraction, bit unstuffing, high-level decoding), `reader`.

## Stages

1. **Bullseye candidates.** On rows of the binary plane, a run sequence of equal widths 1:1:1:1:1 around a dark center (compact: two dark rings around the center module; full-range: three) is a candidate center. Confirm on the column and both diagonals through the same center with the same module estimate (tolerance per run, like the QR finder ratio test), and count the rings on each ray to tell compact (bullseye 9×9 inside an 11×11 core) from full-range (13×13 inside 15×15). Candidates from several rows that agree are merged.
2. **Core fit.** Walk each ring boundary along rays from the center to get ring corners; fit a homography from the corners of the outermost bullseye ring (the square ring is preserved by perspective as a quadrilateral). Refine with the corners of the inner rings by least squares.
3. **Orientation and mode message.** The ring just outside the bullseye holds the mode message and the four orientation marks (3, 2, 1 and 0 dark modules at the four corners), which fix the rotation and detect mirroring (a mirrored symbol reads the marks in the other order). Read the mode message bits: compact 28 bits (7 four-bit words: layers − 1 in 2 bits, data codewords − 1 in 6 bits, 5 check words), full-range 40 bits (10 words: layers − 1 in 5 bits, data codewords − 1 in 11 bits, 6 check words). Reed-Solomon over GF(16) (x^4 + x + 1). Aztec Runes are compact symbols whose mode message (with the rune XOR pattern) is the payload.
4. **Layer sampling.** Layers wrap around the core two modules thick, spiralling outwards. Full-range symbols have a reference grid (alternating lines every 16 modules from the center); the grid points are located and used to re-anchor the mapping per block (piecewise homography), so large symbols survive perspective and slight warping. Modules under 3 px are sampled at their center, larger ones with a small majority vote. The total size follows from the layer count and is checked against the measured outer extent.
5. **Error correction.** Codeword size from the layer count: 6 bits (1–2 layers), 8 (3–8), 10 (9–22), 12 (23–32). Reed-Solomon over GF(64) (x^6 + x + 1), GF(256) (x^8 + x^5 + x^3 + x^2 + 1), GF(1024) (x^10 + x^3 + 1) and GF(4096) (x^12 + x^6 + x^5 + x^3 + 1), first root α^1. `breader-ecc` gets a generic binary-extension field with `u16` symbols for these (the existing `Gf256` stays as is). Misdecode guard, like Data Matrix: corrections are capped below the specification for small check counts (reserve 2 up to 7 check words, 1 up to 12) and all-zero or all-one codewords (invalid after bit stuffing) are erasures. The mode message RS is also capped (at most 1 correction for compact, 2 for full-range).
6. **Decoding.** Remove bit stuffing (a codeword of all ones but the last bit, or all zeros but the last bit, loses its last bit), then the high-level modes: Upper, Lower, Mixed, Punct, Digit with latches and shifts, Binary Shift (5-bit length, or 11-bit extended), FLG(0) = FNC1 (GS1 when first), FLG(1–6) = ECI with that many digits. Structured Append: the header at the start in Upper mode (optional message ID, sequence letters). Data outside an ECI is ISO/IEC 8859-1 (`Charset::Latin1`), as the standard's default interpretation.
7. **Results and metadata.** `SymbolMeta::Aztec(AztecMeta { compact, layers, data_codewords, ec_percent, errors_corrected, mirrored, rune, structured_append })`. AIM identifier `]z` with the modifier the standard assigns (to be checked against ISO/IEC 24778 annex: FNC1 and ECI change it). The quad is the symbol outline from the fitted mapping.

## Effort

Fast: row candidates every few rows, dark-on-light only (like Data Matrix). Balanced: all rows, both polarities, mirror retry. Exhaustive: also retries the core fit with each ring as the anchor and a grid-only resample.

## Evaluation

zxing-cpp `aztec-1` and `aztec-2` (48 labels) plus synth classes (compact and full-range, rotated, perspective, blur, low resolution, inverted, mirrored, runes). Targets: read at least as many as zxing-cpp, 0 false positives, no latency increase above 1 ms p50 Balanced on frames without Aztec (bullseye candidates must be cheap to reject).

## Known gaps

None yet (not implemented).
