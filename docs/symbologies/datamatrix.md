# Data Matrix (`breader-datamatrix`, milestone M8)

Design of the Data Matrix ECC 200. Part of the cross-cutting [architecture](../ARCHITECTURE.md) (pipeline, traits, validation policy); status, spec references and read rates are in the [symbology index](README.md).

Scope: ECC 200 only (ECC 000–140 are obsolete).
- **Sizes:**
  - square symbols from 10×10 to 144×144;
  - rectangular symbols from 8×18 to 16×48;
  - DMRE rectangular sizes (ISO/IEC 21471).
- **Content:** GS1 (FNC1), ECI and Structured Append (headers are parsed).

Out of scope: direct part marking. The normative source is ISO/IEC 16022 and ISO/IEC 21471.

**Tests:** the `datamatrix` crate (MIT/Apache) is allowed as a dev-dependency, to encode test symbols only. Clean-room: its decoder sources are never read.

The structure mirrors `breader-qr`. The modules are `spec` (size table, region layout, block layout), `place` (Annex F placement), `decode` (grid to payload), `detect`, `sample` and `reader`.

`DataMatrixReader` uses the binary matrix (`uses_binary() == true`) and sets `handles_inversion() == true`. It reads both polarities through a polarity view, like the QR `Plane`. A shared view type can move to `breader-core` if both readers end up needing the same one.

1. **Candidates.**
   - Dark-pixel counts over 4x4 blocks of the binary plane are built for each polarity, and the coarser levels (8, 16, ...) sum 2x2 groups of them.
   - The finest level keeps a cell when at least 30 % of its pixels are dark; this drops speckle that would otherwise bridge a symbol to a textured surface. Coarser levels keep any cell with a dark pixel, because a share there would erase the thin clock tracks on the symbol boundary.
   - Cells that are fully dark in a block (the AND plane) are eroded and dilated to cut out thick solid regions (shadows, filled areas) before labeling.
   - Connected components are labeled with union–find over word-packed runs. A component is kept when it has at least 16 cells and a 3x3 bounding box. Compact, filled components are fitted first; the per-level budget (48 / 128 / 512 components by effort) is split between them and the largest others (merged symbols, clusters).
   - The pixels of a component in full resolution give per-row extents and a convex hull (no sorting: the extents are already ordered by row). A quad is fitted to the hull from the four best supported side lines (turns of 55 to 125 degrees), and two further quads come from pairs of solid hull sides (the L finder), which still work when the clock track is fragmented.
   - Fast searches 4 and 8 px blocks first and the 16 px level only when no symbol is known in the frame ([D-046](../decisions.md#d-046)); Balanced and Exhaustive search all their levels in one stage.
   - Quads found at several levels are merged when their corners agree, and ranked by convexity and density.
2. **Orientation and size.**
   - A cheap precheck runs on every candidate in rank order: coarse samples along a side half a module inside the edge must be dark (at least 73 %) on two adjacent sides (the L) while the other two are not solid, then the module size is estimated. A polarity is abandoned after a run of candidates (48 / 96 / 384 by effort) without an L, since the tail of the ranking is texture.
   - Survivors are re-fitted (RANSAC lines through the boundary pixels), the L is confirmed on the refined quad (78 %), and the two clock sides are sampled at the middle of their modules (a shallower probe is tried again for bold prints) and must alternate (regular runs ≥ 70 %). The corner of the L is the symbol's bottom-left corner.
   - Size hypotheses are the table entries that best match the counted runs, the side lengths divided by the module size, and the alternating pattern of each clock sample series (module counts of an alternating track are matched against every table dimension, which survives runs that merge or split). The top three are tried, a transposed (mirrored) reading when decoding fails.
3. **Sampling.**
   - An initial homography maps the four quad corners.
   - Refinement: the clock-track module centers are located along both clock sides (dark/light run midpoints with sub-pixel interpolation), and the homography is refit by least squares over the L corners plus the clock centers. This handles the top-right corner, which perspective distorts the most.
   - For multi-region symbols, the alignment patterns between data regions (two-module solid and dotted lines) are skipped by the region layout, and their positions are also used as extra fit points.
   - Modules under 3 px are sampled at their center; larger modules use a 5-point majority.
   - When Reed-Solomon fails on a plausible grid, the grid is sampled again through a mapping corrected per column and row boundary with the clock transitions of the top rows and right columns of every region (the same measurement `refine` uses, kept as an offset per boundary instead of a global fit). This follows the uneven pitch of 2x2-region prints, where the left regions drift against the right ones.
4. **Cheap gates** (before Reed-Solomon, like the QR gates): L solidity, clock alternation and alignment-pattern agreement on the sampled grid.
5. **Decoding.**
   - Remove the finder, clock and alignment patterns to obtain the mapping matrix.
   - Extract codewords with the Annex F placement, including the four corner cases.
   - De-interleave the blocks (the 144×144 symbol has unequal blocks).
   - Run Reed-Solomon over GF(256) with polynomial 0x12D and first root α^1, using `breader-ecc`. Every block must succeed.
   - A block with more than 12 check codewords that decodes with no correction does not need the pad check; non-standard pad values (some encoders write them) are accepted there. Corrected or small blocks keep it.
   - Small blocks may correct fewer codewords than the spec allows: (ec − r) / 2, where r = 2 when ec ≤ 7, r = 1 when ec ≤ 12, and r = 0 otherwise. This is our own zero-FP policy, analogous to QR's misdecode protection, not an ISO 16022 requirement. As a result, 10×10 accepts one correction instead of two.
   - Grid contract: the L finder is on the left and bottom edges. A mirrored retry reflects the grid across the diagonal through the bottom-left corner, which swaps the dimensions.
   - Ambiguous cases, resolved conservatively (details in `decode/encodation.rs`):
     - Reader Programming (234) is accepted as the first codeword only and reported as `DataMatrixMeta::reader_init` ([D-035](../decisions.md#d-035)); anywhere else it rejects the symbol.
     - Empty payloads are rejected.
     - Pads must match the 253-state sequence exactly, except for uncorrected large blocks (above).
     - Structured Append is accepted only in the first position, and macros only before any data.
   - Decode the encodation modes:
     - ASCII, including digit pairs, Upper Shift and padding with its 253-state randomization check;
     - C40 and Text, with the three shift sets;
     - X12, EDIFACT, and Base256 with its length field and 255-state derandomization;
     - FNC1 (first or second position, or GS inside data), ECI, macros 05/06 (header and trailer applied to `data`), Structured Append (233, parsed and recorded), and Reader Programming (234, flagged in `reader_init`).
   - Invalid sequences reject the symbol. Data after the end of a mode must follow the spec's unlatch rules.
6. **Results.** `data` holds the raw bytes. `segments` gives the charset (`Eci(n)` after ECI 241, otherwise `Latin1`, which is the Data Matrix default). The new `SymbolMeta::DataMatrix(DataMatrixMeta { rows, cols, errors_corrected, mirrored, structured_append })` carries the metadata. The AIM identifiers are `]d1` (default), `]d2` (FNC1 first, GS1), `]d3` (FNC1 second), and `]d4`/`]d5`/`]d6` for the same with ECI. `ContentClassifier` already treats AIM GS1 markers; `]d2` is added to its list.

**Passes and effort.** The first pass of a frame (original image, first binarizer) finds nearly every symbol. Later passes repeat the search only when a symbol is already known in the frame (more may be hidden) or at Exhaustive effort. Fast reads dark-on-light symbols only (the inverted polarity costs about as much as the first) and uses two pooling levels (the third added no read at Fast, [D-044](../decisions.md#d-044)); Balanced searches both polarities with a third of the component budget for the inverted plane; Exhaustive adds two levels and larger budgets.

Evaluation sets:
- Dynamsoft `challenging-images` (50 Data Matrix), `skewed-datamatrix` (42) and the Data Matrix videos;
- synth Data Matrix classes (square, rectangular, DMRE, inverted, rotated, perspective, blur, low-res).

Targets: read at least as many as zxing-cpp on every set, with 0 FP.
