# Linear symbologies (`breader-linear`)

Design of the EAN-13, EAN-8, UPC-A, UPC-E (with EAN-2/EAN-5 add-ons), Code 128, Code 39, ITF and Codabar. Part of the cross-cutting [architecture](../ARCHITECTURE.md) (pipeline, traits, validation policy); status, spec references and read rates are in the [symbology index](README.md).

`LinearReader` returns `uses_binary() == false` and `handles_inversion() == true`: it reads grayscale profiles and decodes both polarities from the same edges. It only scans the original image; the downscaled variant was measured to add no 1D reads on any dataset.

Internal modules and data flow for one pass:

```
ScanPlan ─► Scanline ─► profile (u8) ─► Edges ─► Runs ─┬─ dark bars  ─┐
                                                        └─ light bars ─┤
                       LinearDecoder × K (EAN/UPC, 128, 39, ITF, Codabar) ◄─────┘
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
6. **Aggregator** — groups hits by (symbology, text, polarity) with single-link proximity: center distance below the longer segment, any direction, so lines at different angles confirm each other. A group becomes a `Symbol` with at least `min_line_agreement` lines (ITF: at least 3).
   - **Conflicts:** if two groups of the same family overlap (segment and rectangle overlap) with different text, the one with fewer lines survives only when the other has at least twice as many; otherwise both are dropped (NFR-1). This also removes partial reads such as a UPC-E inside an EAN-13 or 12 digits of an ITF-14.
   - **Refinement:** a group that is still short of agreement gets parallel lines on both sides (at most 24 per side, stopping after 3 misses, at most 48 seeds per pass). Adjacent lines see nearly the same pixels, so a group backed by a single line of the sparse plan needs `min_line_agreement + 1` lines.
   - **Coordinates:** the quad comes from the endpoint extents along and across the mean direction, mapped to original-frame coordinates with `FrameContext::scale`.

EAN/UPC specifics: EAN-13 encodes its first digit in the L/G parity of the left half. An EAN-13 with leading 0 is reported as UPC-A (12 digits) when UPC-A is enabled, and as EAN-13 otherwise. UPC-E is expanded to UPC-A only for check digit validation; the reported data is the 8-digit UPC-E. Add-ons are searched after the end guard (gap of 5–14 modules, start pattern 1011) and need their own line agreement. Minimum quiet zones are 4.5 / 5.5 / 6.5 modules for EAN-13 and UPC-A / EAN-8 / UPC-E. A symbol whose end guard cannot be verified is never accepted, even when the digits and check digit would be. EAN-2 parity comes from the value mod 4, EAN-5 parity from its weighted checksum. `EanAddonPolicy` decides acceptance.

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

