# Symbology page template

Copy the block below to `docs/symbologies/<name>.md`. Keep the section order so cross-references stay stable. Then add a row to [symbologies/README.md](../symbologies/README.md) and a pointer stub under section 3 of [ARCHITECTURE](../ARCHITECTURE.md#3-symbology-families). The full checklist is in [contributing](../contributing.md#adding-a-symbology).

````markdown
# <Symbology> (`breader-<name>`, milestone Mx)

One paragraph: what is read, how it fits the pipeline (`uses_binary()`, `handles_inversion()`), links to the architecture and the symbology index.

## Scope

In scope, out of scope (with the reason), normative sources (ISO/IEC number). Test-only dependencies (encoder crates) and the clean-room note.

## Modules and data flow

Module list (`spec`, `detect`, `sample`, `decode`, `reader`, ...) and a diagram or numbered stages of one pass.

## Stages

1. **Detection / candidates.** Thresholds and why.
2. **Sampling.**
3. **Validation and error correction.** Every check that guards against false positives; any capacity below the specification, stated as our own policy.
4. **Decoding.**
5. **Results and metadata.** `Symbol` fields, `SymbolMeta` variant, AIM identifier.

## Effort

What changes in Fast / Balanced / Exhaustive; retries that run only after a failure.

## Evaluation

Datasets and gates that cover it, targets, link to the measurement in BASELINES, open triage in a FAILURES page if any.

## Known gaps

What is not read and why (by design, not investigated, out of scope).
````
