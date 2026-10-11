# Decision log

Short entries for decisions that shape the product, in the order they were taken. New decisions are appended at the end (copy [templates/decision.md](templates/decision.md)); a reversed decision is not edited away: add a new entry that supersedes it and mark the old one `Superseded by D-NNN`. Dates are ISO; entries before this log existed were reconstructed from the docs and git history (2026-10-09), so their dates are the date of the commit or document that first records them.

Product-level defaults and scope are owner decisions ([contributing](contributing.md#changing-defaults)).

| ID | Date | Decision |
|---|---|---|
| [D-001](#d-001) | 2026-10-06 | Clean-room implementation from the specifications |
| [D-002](#d-002) | 2026-10-06 | Accuracy first, zero false positives as a hard constraint |
| [D-003](#d-003) | 2026-10-06 | Pluggable pipeline; families independent, static dispatch inside |
| [D-004](#d-004) | 2026-10-06 | Dynamsoft data is internal-only and never committed |
| [D-005](#d-005) | 2026-10-06 | Ground-truth overrides need visual verification |
| [D-006](#d-006) | 2026-10-06 | Code 39 Full ASCII off by default |
| [D-007](#d-007) | 2026-10-07 | v1 targets |
| [D-008](#d-008) | 2026-10-07 | ITF default accepted lengths: 14 only |
| [D-009](#d-009) | 2026-10-07 | License: AGPL-3.0-only plus commercial |
| [D-010](#d-010) | 2026-10-07 | Regression gate with versioned baselines; CI on GitHub Actions |
| [D-011](#d-011) | 2026-10-07 | Fuzz corpus is not versioned |
| [D-012](#d-012) | 2026-10-07 | 1D reader scans the original image only |
| [D-013](#d-013) | 2026-10-07 | Threads optional and deterministic; Tracker uses a rolling strip scan |
| [D-014](#d-014) | 2026-10-08 | Data Matrix scope: ECC 200, no direct part marking |
| [D-015](#d-015) | 2026-10-08 | Data Matrix correction capacity below the spec maximum |
| [D-016](#d-016) | 2026-10-08 | zxing-cpp samples as a second gated dataset |
| [D-017](#d-017) | 2026-10-08 | Code 39 quiet zone 4X with an extra-line safeguard |
| [D-018](#d-018) | 2026-10-08 | Up-scale retry only in Balanced and Exhaustive |
| [D-019](#d-019) | 2026-10-09 | Open quiet zone at the profile border, except ITF |
| [D-020](#d-020) | 2026-10-09 | Label normalizations belong in bench matching, not in the decoder |
| [D-021](#d-021) | 2026-10-09 | Baselines are regenerated on the combined tree after merging branches |
| [D-022](#d-022) | 2026-10-09 | PDF417 design: reuse linear edges, GF(929) in `breader-ecc` |
| [D-024](#d-024) | 2026-10-09 | PDF417 decoding choices: ISO 8859-1 default, `]L1`/`]L2`, strict length descriptor |
| [D-025](#d-025) | 2026-10-09 | Third-party data tables need their license text in THIRD-PARTY-NOTICES.md |
| [D-023](#d-023) | 2026-10-09 | Central documentation hub, per-family pages |
| [D-038](#d-038) | 2026-10-11 | Micro QR inside `breader-qr`: format distance 2, RS guard, M1 representation, AIM |

## D-001

2026-10-06. **Clean-room implementation.** ZXing and zbar may be studied for ideas; no code, tables or structure are copied. Specifications are the normative sources; tables are generated or transcribed from them. Why: licensing independence (the dual license needs clean provenance) and better understanding of the failure modes. See [REQUIREMENTS section 5](REQUIREMENTS.md#5-clean-room-policy), [contributing](contributing.md#clean-room-rules).

## D-002

2026-10-06. **Accuracy first; zero false positives.** Priorities are accuracy, then speed. No wrong result is acceptable on any benchmark or negative image; when in doubt return nothing. Every decoder validates inside itself (checksum, quiet zones, line agreement, Reed-Solomon). Why: a wrong read in a logistics or healthcare flow is worse than a miss. See NFR-1 in [REQUIREMENTS](REQUIREMENTS.md#4-non-functional-requirements), [ARCHITECTURE 2.5](ARCHITECTURE.md#25-validation-and-false-positive-control), [testing](testing.md#false-positive-policy-and-triage).

## D-003

2026-10-06. **Pluggable pipeline.** Module crates depend only on `breader-core` (and `breader-ecc`); only the facade knows all families; dynamic dispatch at stage boundaries, static dispatch inside a family. Why: new symbologies and techniques without touching the core; hot loops inline. See [ARCHITECTURE sections 1 and 2.4](ARCHITECTURE.md#1-workspace).

## D-004

2026-10-06. **Dynamsoft datasets are internal-only.** Fetched into the gitignored `data/` by `tools/fetch-datasets.sh`, never committed or redistributed, and fetched in CI only by the weekly or manual workflow. Why: most subsets state no license (only `UPS-Synthetic-Labels` is CC BY 4.0). See [datasets](datasets.md#dynamsoft-subsets), [CI](CI.md).

## D-005

2026-10-06. **Label overrides require visual verification.** Ground truth is cleaned by deduplication and by versioned overrides in `tools/bench/gt-overrides.json`, each with a reason; a label changes only when the image proves it. First case: `barcode-with-shadow-1.jpg` (printed digits differ from the label although the check digit is equal). Why: otherwise the benchmark drifts toward whatever breader reads. See [datasets](datasets.md#overrides-policy), [BASELINES](BASELINES.md#ground-truth-cleanup).

## D-006

2026-10-06. **Code 39 Full ASCII is off by default**, as is the mod-43 requirement (FR-17). Why: Full ASCII reinterprets `$ / + %` pairs and changes the payload of plain Code 39 labels; it must be a conscious choice. The bench accepts the expansion of a label when comparing ([D-020](#d-020)). See [GUIDE](GUIDE.md#scanoptions).

## D-007

2026-10-07. **v1 targets:** 0 false positives everywhere including `negative`; challenging-images read rate at least 86 % (zxing-cpp level); synth at least 82 % overall and at least zxing-cpp in every class; Balanced p50 not above zxing-cpp; Fast p50 at most 10 ms on a 1080p frame. See [BASELINES](BASELINES.md#v1-targets-confirmed-2026-10-07).

## D-008

2026-10-07. **ITF accepts only 14 digits by default** (`ItfOptions::allowed_lengths = [14]`, minimum length 6). Why: ITF has no start/stop redundancy to reject truncated reads, and partial ITF reads were the largest source of false positives in other engines. Consequence: most 4 to 10 digit ITF labels in the zxing samples are not read by design; users widen the list for formats they expect. See [GUIDE](GUIDE.md#scanoptions), [BASELINES](BASELINES.md#zxing-cpp-samples-2026-10-08).

## D-009

2026-10-07. **Dual license: `AGPL-3.0-only` plus a commercial license**, copyright Marcos Alonso, with a CLA for external contributions. Why: open source with strong copyleft while keeping the option to sell commercial use. See [README](../README.md#license), [LICENSE-COMMERCIAL](../LICENSE-COMMERCIAL.md).

## D-010

2026-10-07. **Regression gate with versioned baselines; CI on GitHub Actions.** Per dataset and effort the gate fails on fewer reads, any false positive or a changed expected count; latency is not gated. Synthetic sets run on every push, real-world sets weekly or manual. Why: speed work and accuracy work must not silently trade reads. See [testing](testing.md#gates), [CI](CI.md).

## D-011

2026-10-07. **Fuzz corpus is not versioned.** `corpus/`, `artifacts/` and `target/` are gitignored; CI caches the corpus between nightly runs; only small seeds are generated on demand. Why: the corpus grows without bound and is reproducible. See [fuzz/README.md](../fuzz/README.md).

## D-012

2026-10-07. **The 1D reader scans only the original image.** The downscaled variant was measured to add no 1D reads on any dataset, and the smoothed edge variant runs only on lines where a start pattern was found. Why: speed without read loss (41.3 to 14.4 ms per image on challenging-images). See [ARCHITECTURE section 5](ARCHITECTURE.md#5-performance-plan-m7), [linear design](symbologies/linear.md).

## D-013

2026-10-07. **Single-threaded by default; `rayon` optional with identical output.** Speculative QR triplet decoding was built, made deterministic, then removed because it gave no gain. The video `Tracker` uses a rolling strip scan (3 strips, 50 % overlap): 40 of 74 reads against 38 for stateless Balanced, 24 flickers against 212. See [ARCHITECTURE section 5](ARCHITECTURE.md#5-performance-plan-m7), [GUIDE](GUIDE.md#video-tracker).

## D-014

2026-10-08. **Data Matrix scope: ECC 200 only, no direct part marking.** Square, rectangular and DMRE sizes, GS1, ECI, Structured Append headers. ECC 000-140 are obsolete; dot-peened and etched DPM on rough metal is out of scope (the misses `datamatrix-3` stay misses). Fast searches dark-on-light only. See [datamatrix design](symbologies/datamatrix.md), [BASELINES](BASELINES.md#data-matrix-2026-10-08).

## D-015

2026-10-08. **Data Matrix corrects fewer codewords than the spec allows on small blocks**: (ec - r) / 2 with r = 2 for ec <= 7, 1 for ec <= 12, 0 otherwise (10x10 accepts one correction, not two). Pads must match the 253-state sequence except for uncorrected large blocks. Why: our own zero-false-positive policy, analogous to QR's misdecode protection; not an ISO requirement. See [datamatrix design](symbologies/datamatrix.md).

## D-016

2026-10-08. **zxing-cpp `test/samples` is a second gated dataset**, pinned to tag `v3.1.1`, used as test data only (images and expected values; Apache-2.0). It closed the open question in REQUIREMENTS section 8 and is richer in QR and EAN/UPC. See [datasets](datasets.md#zxing-cpp-samples), [FAILURES-ZXING-SAMPLES](FAILURES-ZXING-SAMPLES.md).

## D-017

2026-10-08. **Code 39 quiet zone of 4 modules** (was 7), with a safeguard: a symbol with either quiet zone below 7 modules and no check character needs one more agreeing line. Result: `batch_code39` Fast 9 to 25 of 32. The safeguard cost one video read (28 instead of 29 at Balanced); 0 false positives were measured without it too, and it is kept as insurance. See [FAILURES](FAILURES.md#status-of-the-fixes), [BASELINES](BASELINES.md#linear-reader-fixes-2026-10-08).

## D-018

2026-10-08. **Up-scale retry only in Balanced and Exhaustive.** After the regular passes, a frame that produced nothing and has `max(w, h) <= 600` is rescanned 3x up-scaled (Catmull-Rom). Never when something was found; off in Fast (limit 0) and in `Tracker`. Measured +28 reads on the zxing samples at Balanced, 0 false positives; running it also when something was found gained one read and is not done. Whether Fast should enable it is pending an owner decision. Cost: a blank 600x600 frame +15 ms at Balanced. See [ARCHITECTURE 2.2](ARCHITECTURE.md#22-passes-and-escalation), [GUIDE](GUIDE.md#up-scale-retry).

## D-019

2026-10-09. **A quiet zone that ends at the profile border counts as open** for EAN/UPC, Code 128 and Code 39, not for ITF. Why: tight crops lose the margin but not the symbol; ITF is the most false-positive-prone symbology and stays strict. Edge relocation may move a reference level at most 0.25 of the local level difference (without the limit three groups regressed). See [BASELINES](BASELINES.md#edge-relocation-and-border-quiet-zone-2026-10-08), [linear design](symbologies/linear.md).

## D-020

2026-10-09. **Label normalizations live in bench matching**, with unit tests: UPC-A / UPC-E / EAN-13 folding, charset-decoded text, Code 39 Full ASCII expansion, UTF-8 read as Latin-1 repaired in labels only (never in reads). Why: a read that is correct in another notation must not count as a false positive, and decoders must not bend to labels. See [testing](testing.md#false-positive-policy-and-triage).

## D-021

2026-10-09. **After merging branches, regenerate baselines on the combined tree** and verify no decrease against either parent; never merge baseline JSON textually. Why: combining branches surfaced a label mismatch neither measured. See [testing](testing.md#merging-baselines-after-combining-branches).

## D-022

2026-10-09. **PDF417 design:** gray-profile reader reusing linear edge and run extraction through a small `#[doc(hidden)]` API, Reed-Solomon over GF(929) with erasures in `breader-ecc`, misdecode guard `2*errors + erasures <= ec - reserve`; MicroPDF417 and GS1 composite deferred. Status: decoding implemented (merged 2026-10-09). See [pdf417 design](symbologies/pdf417.md).

## D-023

2026-10-09. **Central documentation hub.** `docs/README.md` is the entry point; per-family designs moved out of ARCHITECTURE into `docs/symbologies/`, with ARCHITECTURE keeping section numbers 3.x as stubs; datasets, testing, contributing and this log became separate pages. Why: the project changes quickly and each change should touch one obvious page. See [docs/README.md](README.md).

## D-024

2026-10-09. **PDF417 decoding choices.** Default character set ISO/IEC 8859-1 (ISO/IEC 15438:2006) reported as `Charset::Latin1`; AIM `]L2` without ECI and `]L1` with ECI; the symbol length descriptor must match exactly; reader initialisation (921) only as the first codeword and rejected by the reader (the rejection is superseded by [D-035](#d-035): such symbols are returned with `reader_init`); rxing's encoder is a test-only dev-dependency. Why: follow the standard and reject rather than guess. Open: the AIM modifier meanings and the strict descriptor should be re-checked against ISO/IEC 15424 / real symbols. See [pdf417 design](symbologies/pdf417.md).

## D-025

2026-10-09. **Third-party data keeps its license text.** The PDF417 cluster tables are transcribed from the MIT-licensed `pdf417` crate; its notice is in [THIRD-PARTY-NOTICES.md](../THIRD-PARTY-NOTICES.md) and cited in the source. Any future table taken from a permissive source does the same; nothing is taken from zxing/zbar. Why: MIT requires the notice in substantial copies, and it keeps the clean-room record explicit.

## D-026

2026-10-09. **PDF417 start/stop search runs inside the linear scan** (`breader-linear/src/marks.rs`), its hits are shared through a typed per-frame cache (`FrameContext::shared`, `FrameCache`) and the PDF417 reader depends on `breader-linear` (a `#[doc(hidden)]` `stacked` module). Why: a second scan of the plan costs as much as the 1D scan; the shared one adds +0.25 ms p50 at Balanced on frames without PDF417 (target 1.5 ms). Without the linear reader the PDF417 reader scans the plan itself. See [pdf417](symbologies/pdf417.md#as-built).

## D-027

2026-10-09. **`pdf417` joins the default features**, registered after the linear reader, as `datamatrix` did. Why: gated at 0 false positives on every set, latency within target. With `symbologies` lacking PDF417 the linear scan does not record the hits.

## D-028

2026-10-09. **PDF417 boundary and row reading choices.** Boundaries by line walking and a trimmed least squares fit instead of RANSAC (the hits are dense and mostly collinear); row lines perpendicular to the boundary, then perspective between two boundaries, then turned and fanned rows picked by clean codewords on probe lines; the cluster check is a vote per line, not a gate; codewords are read on the module grid of the clean boundaries as well as per codeword. An ECI designator inside a byte run does not restart the groups of five codewords. Why: each step was added for a failing sample or synthetic class and keeps 0 false positives, because every symbol still passes the Reed-Solomon guard, the length descriptor and the compaction rules. Not covered: mirrored symbols, 1 px modules, compact symbols under strong perspective.

## D-029

2026-10-09. **No label is added for the PDF417 video.** Neither breader nor zxing-cpp reads any frame of `batch_rotated_small_pdf417_1080p` (modules far below a pixel), so no value can be verified; the video gate keeps 0 expected and 0 reads. `video-bench` gets an `ADDITIONS` list (video, format, payload, reason) for verified labels. Why: labels follow the images, not the engines.

## D-030

2026-10-09. **Aztec decoding choices** (ISO/IEC 24778, `breader-aztec` decode side). Generic `Gf2m<Q>` Reed-Solomon in `breader-ecc` for GF(16) to GF(4096) (first root alpha^1, errors and erasures reported separately, `Gf256` and `Gf929` unchanged); the guard is `2 * errors + erasures <= ec - reserve` with reserve 2 up to 7 check words and 1 up to 12, applied to the mode message too (so at most 1 correction compact, 2 full-range); only *data* codewords of all zeros or all ones are erasures (check codewords are not stuffed); at least 3 check codewords; a P/S shift followed by a latch or the Punct U/L code, a reserved FLG(7) and a message without data bytes are rejected; ECI digits are 4-bit Digit codes whatever the mode; default charset ISO/IEC 8859-1 (`Charset::Latin1`); the module grid is decoded only if the bullseye and orientation marks match (80 % and 10 of 12). Why: reject rather than guess; conventions confirmed against rxing's encoder. See [aztec](symbologies/aztec.md#as-built-decoding).

## D-031

2026-10-09. **Aztec Structured Append is not parsed; `AztecMeta` has no structured append field.** The sequence header lives in the Upper-mode data and nothing flags it, so recognizing it by shape would corrupt ordinary messages. Open: settle against the standard text before the reader ships, then add the field and the stripping together.

## D-032

2026-10-09. **Aztec AIM identifier and Runes.** `]z0` plain, `]z1` FNC1 first (GS1), `]z2` FNC1 second (one letter or two digits before it), `+3` when an ECI is present, `]zC` for a Rune; a Rune's payload is its value as decimal text and its mode message is XORed with `0xAAAAAAA`. Why: ISO/IEC 24778 as remembered. Open: no independent encoder makes Runes and the modifier table was not available as text, so both need a check against the standard or a reference symbol.

## D-033

2026-10-09. **Aztec reader choices.** (1) `Plane` (binary image in one polarity) and the least-squares `Homography` moved from the QR and Data Matrix crates into `breader-core` and are shared with `breader-aztec`. (2) The reader scans the first binarization only (larger frames at Fast and Balanced), later passes only when a symbol is known, a bullseye was seen but not read (per-frame note) or at Exhaustive: the row search costs about 0.8 ms per megapixel and the added p50 on `challenging-images` Balanced is +0.28 ms with it (about +7 ms scanning every pass, over the +1 ms budget); small frames (up to 160 000 px) scan every pass because a small clean symbol may only binarize in the second one. (3) Compact versus full-range and the orientation come from probing the sampled core in the eight orientations with the mode message (RS-protected), not from the ring count. (4) A symbol is accepted only through `decode_grid` (finder gate, mode message and data Reed-Solomon with the guard); the bullseye fit, the reference-grid check and the size consistency only decide what is worth decoding. Why: cost on frames without Aztec, and zero false positives.

## D-034

2026-10-09. **Aztec decoder additions from the zxing samples.** (1) A Rune's payload is its value as three digits (`001`): the labels and zxing-cpp spell it so, and a fixed width is unambiguous. (2) Reader initialisation: for one-layer symbols the top bit of the codeword count field, when the count would exceed the symbol's capacity, is a flag (`AztecMeta::reader_init`) and the count is the rest; inferred from `readerinit-compact-15x15` and `readerinit-full-19x19` (payload `$I`, three codewords), not checked against the standard text. (3) Binary Shift after a shift (U/S, then B/S) is accepted and the text after the bytes continues in the mode the shift went to (`dlusbs`). (4) Structured Append stays unparsed ([D-031](#d-031)): the bench strips a header of the form space, ID, space, two capitals in order from Aztec reads when comparing, so the labelled payload matches; the reader returns the header as data. Open: the standard text for (2) and (4).

## D-035

2026-10-09 (owner decision). **All readers return reader-initialisation / reader-programming symbols, flagged in metadata.** Aztec already did ([D-034](#d-034)); Data Matrix (Reader Programming codeword 234, valid only as the first codeword) and PDF417 (codeword 921, first data codeword only) used to decode such symbols and drop them. They now return the symbol with `reader_init: true` in `DataMatrixMeta` and `Pdf417Meta` (Python: `meta["reader_init"]`). Every decoder check stays as it was, including Reed-Solomon and the guards; only the final drop (PDF417) and the Data Matrix refusal of codeword 234 as the first codeword went. The payload is returned as data like any other; applications that do not expect programming symbols should ignore symbols whose `reader_init` is set. Why: the symbol is valid and readable, hiding it is a policy the application should own. Supersedes the "rejected by the reader" part of [D-024](#d-024) and the Data Matrix "Reader Programming is rejected" note in the [Data Matrix page](symbologies/datamatrix.md). Effect on the zxing samples: `datamatrix-1/readerinit.png` and `pdf417-1/13-readerinit.png` read at Balanced and Exhaustive, matching their labels, 0 false positives ([BASELINES](BASELINES.md#reader-initialisation-symbols-2026-10-09)).

## D-036

2026-10-09 (owner decision), 2026-10-11 (design). **Codabar is enabled by default, behind hard guards; any false positive on any dataset reopens that default.** Scope: ANSI/AIM BC3 / EN 798 characters `0-9 - $ : / . +` with start and stop characters A-D (the print-time aliases T N * E are not accepted); no check character. Guards: a start and a stop in A-D (not necessarily equal) and none inside; at least 4 data characters (`MIN_DATA_LENGTH`); every element clearly narrow or wide (wide/narrow within [1.7, 3.6], smallest wide at least 1.4 times the largest narrow in each character); inter-character gap 0.5 to 3 narrow widths; narrow width drift at most 1.5 between characters and 2 over the symbol; quiet zone at least 10 narrow widths on both sides (a margin that ends at the profile border counts as open but flags `short_quiet`, which costs one more agreeing line); at least `min_line_agreement + 1` agreeing scanlines (3 by default), as for ITF. `data` excludes start and stop, which `LinearMeta::codabar_start_stop` records; AIM identifier `]F0` (not checked against ISO/IEC 15424). The bench compares Codabar without start and stop characters on both sides, because the zxing-cpp sample labels (and zxing-cpp and rxing reads) spell them. Deviations from the first design: the lower wide/narrow bound is 1.7 instead of 2.0, because blur at 2 to 2.5 px per narrow element measures a 2:1 symbol at 1.8 (synthetic tests failed at 2.0); the hard minimum quiet zone is the full 10 (no separate short threshold except at the border). Why: the character set is small and there is no check character, so structure and agreement carry the false positive control, as for ITF. Known miss: symbols with no quiet zone at all (`codabar-1/01`).

## D-038

2026-10-11. **Micro QR Code (M1 to M4) is read by `breader-qr`, with these choices.** (1) Placement: no new crate and no new feature; `QrReader` reports `Symbology::Qr` and `Symbology::MicroQr`, runs its Micro QR path on the finders of the same scan that no decoded symbol consumed, and honours `ScanOptions::symbologies` for each separately (a frame with only `MicroQr` enabled skips the QR triplets). (2) Format information has one copy, so the accepted Hamming distance to a valid word is 2, not the 3 that QR accepts with two copies. (3) Reed-Solomon guard: corrected codewords `<= (ec - reserve) / 2`, reserve 2 up to 8 check codewords and 1 above; M1 (2 check codewords) corrects nothing and rejects any nonzero syndrome; M2-L therefore corrects 1. (4) Metadata reuses `SymbolMeta::Qr(QrMeta)` with `version` 1 to 4 meaning M1 to M4 and `mask` 0 to 3; M1 has no correction level, so `EcLevel` gained the variant `DetectionOnly` instead of reporting `L`. (5) Result gates besides the check codewords: timing patterns alternating on both sides with the module count they imply, a quiet-zone ring, the size named by the format information, strict padding (zero terminator, pad codewords 0xEC/0x11, zero half codeword) and rejection of unknown mode indicators, ECI, FNC1 and Structured Append (none exist in Micro QR). (6) AIM identifier `]Q1`, recorded as **unverified against ISO/IEC 15424**. (7) The synthetic images leave out M3-L because the `qrcode` encoder's M3-L symbols do not match the standard's code (a real M3-L sample decodes). Why: zero false positives with a single finder and a 15-bit format word needs every structural gate before Reed-Solomon, and sharing the scan keeps the cost to the candidates that survive. See [microqr](symbologies/microqr.md).

## D-037

2026-10-11 (scope by the owner, design by the implementer). **GS1 DataBar Omnidirectional/Truncated, Limited and Expanded (one row) are read by `breader-linear`, on by default, as `Symbology::{DataBar, DataBarLimited, DataBarExpanded}`.** Stacked, Stacked Omnidirectional and Expanded Stacked are the next step (the rows are two or more half symbols; see [linear](symbologies/linear.md#gs1-databar)). Output: the GS1 element string with AIM `]e0`, classified GS1 by `breader-content` (Omnidirectional and Limited: `(01)` GTIN-14 with the computed check digit; Expanded: every AI string the encodation methods can carry, FNC1 as GS). `LinearMeta::databar_segments` holds the symbol characters of an Expanded row (4 to 22, check character included); the variant is the symbology (Omnidirectional and Truncated differ in height only, so they cannot be told apart). Guards: every character must be a member of its combination set (ranks computed exactly); the checksum is mandatory (mod 79 with the two finder patterns agreeing, mod 89 with the 89-pattern check element, mod 211 with the finder sequence fixed by the symbol size); every element of the 45-run row is quantized to whole modules within 0.5 module per piece, pieces keep their module counts within 25 %; guards of one module each; light area of 0.8 module around Omnidirectional and Expanded (Limited: 0.8 before, 4 after; a symbol that starts or ends at the border of the profile is accepted, with the cut guard bar replaced by the other guard's width); Expanded rejects the linkage flag (2D component) and checks the symbol-length field. Agreement: the default `min_line_agreement`, no extra line, because three independent checks (membership, checksum, finders) leave about 1 in 10^7 random windows. Measured effect: 0 false positives on every set and effort ([BASELINES](BASELINES.md#gs1-databar-2026-10-11)).

Sources and checks. Tables were reconstructed from ISO/IEC 24724 as remembered and then cross-checked, never copied from a reader: rxing and zxing-cpp were used as black boxes only (their output on images made from candidate tables; no source read), the independent encoder crate `anyd` 0.2 (MIT, zint-derived, with BWIPP reference vectors in its tests) is a dev-dependency of `breader-linear` and `synth`, and the 13 + 2 + 162 labelled zxing-cpp samples decode to their labels. Cross-checked against `anyd`: the Omnidirectional group and finder tables and checksum weights (`4^k * 3^i` mod 79), the Limited group tables, checksum (`3^i` mod 89) and the 89 check patterns (transcribed from its output, MIT notice in [THIRD-PARTY-NOTICES](../THIRD-PARTY-NOTICES.md) under D-025; BWIPP vectors and two real samples check three of them directly), the Expanded groups, finder sequences by size, weight rows by size and all encodation methods. Unverified against the specification text: for checksum 71 of Omnidirectional symbols both neighbours of the unused finder pair are accepted (72 and 73); Expanded linkage and the 2D composite are not read; the AIM modifier `e0` for every variant; the digit-count rule of Limited (first digit 0 or 1) is the encoder's. After an FNC1 met in alphanumeric or ISO/IEC 646 encodation, two kinds of symbols exist in the samples (numeric resumes, or an explicit numeric latch `000`); the decoder tries both, the explicit latch first when the next three bits are zero, within a budget of 64 branches per symbol, so a pathological stream cannot cost more.
