# Linear symbologies (`breader-linear`)

Design of the EAN-13, EAN-8, UPC-A, UPC-E (with EAN-2/EAN-5 add-ons), Code 128, Code 39, ITF, Codabar and GS1 DataBar (Omnidirectional/Truncated, Limited, Expanded). Part of the cross-cutting [architecture](../ARCHITECTURE.md) (pipeline, traits, validation policy); status, spec references and read rates are in the [symbology index](README.md).

`LinearReader` returns `uses_binary() == false` and `handles_inversion() == true`: it reads grayscale profiles and decodes both polarities from the same edges. It only scans the original image; the downscaled variant was measured to add no 1D reads on any dataset.

Internal modules and data flow for one pass:

```
ScanPlan ─► Scanline ─► profile (u8) ─► Edges ─► Runs ─┬─ dark bars  ─┐
                                                        └─ light bars ─┤
                       LinearDecoder × K (EAN/UPC, 128, 39, ITF, Codabar, DataBar) ◄─────┘
                                   │ LineHit
                                   ▼
                    Aggregator ◄─► Refinement ─► Symbol
```

1. **ScanPlan** — scanlines for the pass image, ordered center-out so early hits come quickly. The spacing is computed from the image extent perpendicular to each direction, capped at 48 / 24 / 12 px (Fast / Balanced / Exhaustive), and multiplied by 1.5 for non-axis directions. The cap keeps small symbols in large photos reachable.

   | Effort | Directions |
   |---|---|
   | Fast | horizontal, vertical |
   | Balanced | 8 (every 22.5°) |
   | Exhaustive | 12 (every 15°) |

2. **Profile** — samples along the scanline at 1 px steps: direct reads when axis-aligned (vertical lines in one row-major sweep). Oblique lines use 16.16 fixed-point DDA with integer bilinear weights and are sampled band by band (32 rows), so each band stays in cache while every line crosses it. Nearest and 1D interpolation were measured and lost reads.
3. **Edges** — central difference of the profile. An edge is a local extremum with `|d| ≥ max(min_edge, 0.12 · local_range, noise term)`, where the noise is estimated on flat blocks only and the threshold is capped at 0.3 · range. Signs must alternate; for consecutive same-sign extrema, the stronger one is kept. Each edge is then moved to the half-level crossing between the neighbouring extremes, which gives sub-pixel positions and corrects print gain. A [1 2 1]-smoothed variant runs in Balanced and Exhaustive only on lines where some decoder found its start pattern (it is the only variant in Fast); unsmoothed is the default because smoothing merges 1.5–2 px bars.
4. **Runs** — widths between consecutive edges, edge positions, and the polarity of the first run. Decoders read both polarities by swapping the bar/space roles. They never allocate.
5. **LinearDecoder** (family-internal trait, static dispatch through a tuple of decoders): `decode(&self, runs, opts, out: &mut Vec<LineHit>)`. Each decoder tries both reading directions:
   - finds its start (or reversed stop) pattern;
   - checks quiet zones against the estimated module width;
   - matches characters by normalizing runs to the character's module count, with print-gain bias and module-width drift (≤ 1.8×) estimated from guard patterns;
   - accepts a character only when its best match beats the second best by a margin; at most one weak character is allowed when a check digit protects it;
   - validates check characters, stop pattern and length rules.

   `LineHit` holds the symbology, text, add-on, polarity, positions along the line and module width; the reader maps positions to image points.
6. **Aggregator** — groups hits by (symbology, text, polarity) with single-link proximity: center distance below the longer segment, any direction, so lines at different angles confirm each other. A group becomes a `Symbol` with at least `min_line_agreement` lines (ITF: at least 3; the default ITF lengths other than 14: 4, [D-041](../decisions.md#d-041)).
   - **Conflicts:** if two groups of the same family overlap (segment and rectangle overlap) with different text, the one with fewer lines survives only when the other has at least twice as many; otherwise both are dropped (NFR-1). This also removes partial reads such as a UPC-E inside an EAN-13 or 12 digits of an ITF-14.
   - **Refinement:** a group that is still short of agreement gets parallel lines on both sides (at most 24 per side, stopping after 3 misses, at most 48 seeds per pass). Adjacent lines see nearly the same pixels, so a group backed by a single line of the sparse plan needs `min_line_agreement + 1` lines. For Code 39 and ITF that extra line is waived when `min` of the agreeing lines are at least 1.9 times the smaller of their scanline steps apart and their module estimates are within 15 % of the group median ([D-042](../decisions.md#d-042)).
   - **Coordinates:** the quad comes from the endpoint extents along and across the mean direction, mapped to original-frame coordinates with `FrameContext::scale`.

EAN/UPC specifics: EAN-13 encodes its first digit in the L/G parity of the left half. An EAN-13 with leading 0 is reported as UPC-A (12 digits) when UPC-A is enabled, and as EAN-13 otherwise. UPC-E is expanded to UPC-A only for check digit validation; the reported data is the 8-digit UPC-E. Add-ons are searched after the end guard (gap of 5–14 modules, start pattern 1011) and need their own line agreement. Minimum quiet zones are 4.5 / 5.5 / 6.5 modules for EAN-13 and UPC-A / EAN-8 / UPC-E. A symbol whose end guard cannot be verified is never accepted, even when the digits and check digit would be. EAN-2 parity comes from the value mod 4, EAN-5 parity from its weighted checksum. `EanAddonPolicy` decides acceptance.

## Code 39 thin modules

Decision: [D-043](../decisions.md#d-043). Tried only where the normal read of a Code 39 symbol failed and the start character's narrow element is at most 2.4 samples. The classification already picks the three widest elements of a character; the relaxation lowers the required gap between the third widest and the fourth from 1.3 to 1.12. Accepted only with quiet zones of 7 narrow widths on both sides (measured, never open), the normal start, stop, gap, ratio and drift checks, and 4 independent agreeing lines. `LineHit::thin` carries the flag to the aggregator. Tests cover symbols of 1.4 to 1.6 px that only the relaxed rules read, quiet zones of 5 and 6 modules, modules above the limit, border cuts and truncated symbols, random stripes and EAN, ITF, Code 128 and Codabar profiles. An upscaled profile was not tried; the remaining InventBar misses fail on separation, ratio and gap tests that looser thresholds did not recover (see [FAILURES-BARBER-1D](../FAILURES-BARBER-1D.md#6-outcome)).

## Codabar

Decision and scope: [D-036](../decisions.md#d-036). Specification: ANSI/AIM BC3 and EN 798; the 20 element patterns are transcribed from the specification table and cross-checked against an independent encoder (the `barcoders` dev-dependency, which draws `+` wrongly and is skipped for that character).

**Design.** `codabar.rs` plugs into the same scan plan, edges, runs and aggregation as Code 39 and ITF; there is no extra scan. A character is 7 elements (bar first, 4 bars and 3 spaces); characters are separated by a gap.

- Start and stop are any of A-D (they need not match); A-D inside the data rejects the symbol, so the start pattern cannot occur in the middle of a longer symbol and a symbol cut by the profile border has no stop. The data must have at least 4 characters (`MIN_DATA_LENGTH`).
- Each character is split into 2 wide elements (digits, `-`, `$`) or 3 (the rest) by size; the smallest wide element must exceed the largest narrow one by 1.4 and wide/narrow must lie in [1.7, 3.6]. The bar/space print-gain bias is estimated from the start character, which always has wide bars and wide spaces.
- Gap 0.5 to 3 narrow widths, narrow width drift at most 1.5 between neighbouring characters and 2 over the symbol, quiet zone at least 10 narrow widths on both sides (open at the profile border, then `short_quiet`).
- No check character is verified or stripped. `data` excludes start and stop, `LinearMeta::codabar_start_stop` has them; AIM `]F0` (unverified against ISO/IEC 15424). Hits of the same text with different guards are different symbols in the aggregator.
- The aggregator asks for `min_line_agreement + 1` agreeing lines (3 by default), plus the usual extra line for a single sparse plan line or a short quiet zone.

**As built.** Tests: table against specification and encoder, every character in both directions and polarities, noise, length, guards inside, truncated symbols (no stop, no start, border cuts), quiet zone rules, gaps and ratios, Code 39 / ITF / EAN / Code 128 and random stripes producing no Codabar, structured random symbols. Synthetic class images are added after the Aztec ones (`generate_codabar`), so earlier synthetic images stay byte-identical; the writer draws wide elements of 2 or 3 modules.

**Effort.** Codabar is decoded at every effort: the scan is shared and the decoder rejects almost everything at its first check (quiet zone fraction of the start character), see [BASELINES](../BASELINES.md#codabar-2026-10-11) for the measured cost.

**Gaps.** `codabar-1/01` has no quiet zone (bars touch the image border; the first bar is lost to the profile), `codabar-1/14` and `15` have 1 px narrow elements and are read only from Balanced up (up-scale retry). Wide/narrow ratios below 1.7 after blur are rejected by design.

## GS1 DataBar

Decision and scope: [D-037](../decisions.md#d-037). Specification: ISO/IEC 24724 and the GS1 General Specifications; code in `crates/breader-linear/src/databar/` (`omni`, `limited`, `expanded`, shared `rank`, `quantize`, `message`).

**Design.** Unlike the width-ratio decoders these symbologies are module-quantized: every element is a whole number of modules and each piece has a fixed module count. One `DataBarDecoder` runs at every window of 45 runs that starts at a bar (Omnidirectional and Limited have the same run count) and, separately, at every bar with a finder A behind it (Expanded). A window passes cheap screens (three guard runs alike, every element between a third and ten guard widths, narrow elements where every Omnidirectional finder has them, finder A for Expanded) before it is copied and quantized.

- **Quantization** (`quantize.rs`). Per piece (guard, character, finder) the widths are scaled so that their sum is the piece's module count, the print gain is estimated from the rounding residuals of bars against spaces and removed, and the elements are rounded with the largest-remainder rule so the sum stays exact. A piece fails when an element is more than 0.5 module from a whole number or the piece's width is more than 25 % off. Guards of one module and the piece sums of the first pass reject most windows.
- **Characters** (`rank.rs`). A character's odd and even elements are each a combination of widths with a given sum and widest element (Omnidirectional/Expanded: 4 elements, Limited: 7); the combinations are numbered in lexicographic order and the value of the character is `group start + odd rank * even count + even rank` (inside characters of Omnidirectional and Expanded swap the roles, and use only the first 48 and 81 (87, 52, 30, 10, 1 for Expanded) combinations of some odd sets: the standard's capacity limits). The rank is computed exactly (inclusion-exclusion counts), and verified against an enumeration for every parameter set in a test; a combination outside its set rejects the character.
- **Omnidirectional/Truncated** (`omni.rs`, 46 elements, 96 modules): characters `1 f 2 4 f 3` between the guards; characters 2 and 3 are numbered from the right. The checksum over the 32 elements (weights `3^i * 4^k` mod 79) selects the pair of finder patterns (79 of the 81 pairs are used); the pair read must be the pair selected. The 13-digit GTIN body is `(c1 * 1597 + c2) * 4537077 + c3 * 1597 + c4`.
- **Limited** (`limited.rs`, 46 elements, 74 modules): two 14-element characters of 26 modules around a 14-element check pattern of 18 modules; `sum(3^i * e_i)` mod 89 over the 28 data elements selects one of 89 patterns, all of which are in the table. Body `left * 2013571 + right`; first digit 0 or 1.
- **Expanded** (`expanded.rs`, `message.rs`): 4 to 22 characters of 17 modules, a finder of 15 after every character of even index, the finder pattern of each position fixed by the number of characters, odd characters printed reversed. The first character is `211 * (n - 4) + checksum`, the checksum being the sum of element widths times powers of 3 mod 211 with weight rows fixed by the number of characters. The remaining characters are a bit stream of 12 bits each: linkage flag, an encodation method (`1`: GTIN then general data; `00`: general data; weight methods with `(3103)`, `(3202)`, `(3203)`; `(392x)`, `(393x)`; `(310x)`/`(320x)` with a date `(11)`, `(13)`, `(15)` or `(17)`, or none), the odd/long flags that must equal the symbol size, then numeric, alphanumeric and ISO/IEC 646 general purpose data. FNC1 is returned as GS; a trailing FNC1 (pairing with the last odd digit) is dropped. After an FNC1 met in alphanumeric or ISO/IEC 646 mode both known continuations are tried (see D-037).
- **Borders.** A guard bar cut by the start or end of the profile has no edge; windows may start at run -1 or end at run `n` and the missing bar takes the other guard's width. Light area beyond the guards is not required then.
- **Agreement.** `min_line_agreement` as for the EAN family; `checksum_verified` is true and `short_quiet` never set.
- **Output.** `data` is the element string (`01` + GTIN-14 for Omnidirectional and Limited), AIM `]e0`, content GS1 through `breader-content`; `LinearMeta::databar_segments` for Expanded.

**As built.** Unit tests: rank against enumeration, group tables against combination counts, finder and check pattern sums, 400 random bodies per variant through the renderer (blur, print gain, 1.8 to 4.8 px per module), every symbol size and every encodation method for Expanded, a flipped module at every position of valid symbols never reads as another symbol, symbols cut by the border, turned and inverted symbols, noise and other symbologies (EAN, Code 39, ITF) read nothing, and reader tests at several angles with the meta. Synthetic images come from `anyd` (`tools/synth`, class images appended after the Micro QR ones).

**Effort.** Decoded at every effort; the screens make the cost +0.05 ms p50 at Fast and about +0.2 ms at Balanced on `challenging-images` ([BASELINES](../BASELINES.md#gs1-databar-2026-10-11)).

**Gaps.** Not read: Stacked, Stacked Omnidirectional and Expanded Stacked (two or more rows to join; the zxing samples `rss14-2` and `rssexpandedstacked-*`; their labels stay unsupported); symbols below about 1.5 px per module or blurred until narrow bars merge (`rss14-1` images 8, 11 and 13 to 15 at Fast; three of them still at Balanced); linked (2D composite) Expanded symbols. Next step: stacked rows (joined by the finder geometry), element correction search for ambiguous widths (the checksum picks the valid alternative).
