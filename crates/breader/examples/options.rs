//! Tuning a scan: effort, symbology selection, ITF lengths, time budget.
//!
//! cargo run --example options --features image -- data/synth/clean/clean_0001.png

use std::time::{Duration, Instant};

use breader::{
    EanAddonPolicy, Effort, Escalation, ItfOptions, Reader, ScanOptions, Symbology, SymbologySet,
};

fn run(label: &str, options: ScanOptions, path: &str) {
    let mut reader = Reader::with_options(options);
    let start = Instant::now();
    let symbols = reader.scan_file(path).expect("readable image");
    println!(
        "{label:<28} {} symbol(s) in {:.1?}",
        symbols.len(),
        start.elapsed()
    );
}

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: options <image>");
        std::process::exit(2);
    };

    // Efforts trade latency for robustness: Fast for video, Balanced by
    // default, Exhaustive for hard stills.
    for effort in [Effort::Fast, Effort::Balanced, Effort::Exhaustive] {
        run(
            &format!("{effort:?}"),
            ScanOptions::with_effort(effort),
            &path,
        );
    }

    // Only look for the symbologies you need; readers of the others are skipped.
    run(
        "QR only",
        ScanOptions {
            symbologies: SymbologySet::EMPTY.with(Symbology::Qr),
            ..ScanOptions::default()
        },
        &path,
    );
    run(
        "retail 1D only",
        ScanOptions {
            symbologies: [
                Symbology::Ean13,
                Symbology::Ean8,
                Symbology::UpcA,
                Symbology::UpcE,
            ]
            .into_iter()
            .collect(),
            ean_addon: EanAddonPolicy::Optional,
            ..ScanOptions::default()
        },
        &path,
    );

    // ITF accepts only 14-digit payloads by default (GTIN-14). An empty list
    // accepts any even length of at least `min_length`, at the cost of more
    // false positives.
    run(
        "ITF, 6 or 14 digits",
        ScanOptions {
            symbologies: SymbologySet::EMPTY.with(Symbology::Itf),
            itf: ItfOptions {
                allowed_lengths: vec![6, 14],
                min_length: 6,
            },
            ..ScanOptions::default()
        },
        &path,
    );

    // A time budget is checked between passes: the scan returns what it has
    // once the budget is exceeded, but a pass in progress is not interrupted.
    run(
        "Exhaustive, 5 ms budget",
        ScanOptions {
            effort: Effort::Exhaustive,
            time_budget: Some(Duration::from_millis(5)),
            ..ScanOptions::default()
        },
        &path,
    );

    // Stop after the first hit, or after the first pass that adds nothing.
    run(
        "Exhaustive, until found",
        ScanOptions {
            effort: Effort::Exhaustive,
            escalation: Some(Escalation::UntilFound),
            max_symbols: 1,
            ..ScanOptions::default()
        },
        &path,
    );
}
