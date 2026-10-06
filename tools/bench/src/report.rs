use std::time::Duration;

use breader::Symbology;
use serde_json::{Value, json};

use crate::matching::{Stats, SymStats};

pub struct Latency {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

pub fn latency(stats: &Stats) -> Latency {
    let mut sorted = stats.latencies.clone();
    sorted.sort();
    Latency {
        p50: ms(percentile(&sorted, 0.5)),
        p95: ms(percentile(&sorted, 0.95)),
        max: ms(sorted.last().copied().unwrap_or_default()),
    }
}

fn rate(s: &SymStats) -> Option<f64> {
    (s.expected > 0).then(|| s.read as f64 / s.expected as f64)
}

fn rate_text(s: &SymStats) -> String {
    rate(s).map_or_else(|| "-".into(), |r| format!("{:.1}%", r * 100.0))
}

/// Symbologies present in the stats, in `Symbology::ALL` order.
fn rows(stats: &Stats) -> Vec<(&'static str, SymStats)> {
    Symbology::ALL
        .iter()
        .filter_map(|s| stats.per_symbology.get(s.name()).map(|v| (s.name(), *v)))
        .collect()
}

pub fn print_table(engine: &str, dataset: &str, stats: &Stats) {
    println!("[{engine}] {dataset}  ({} images)", stats.images);
    println!(
        "{:<10} {:>9} {:>6} {:>8} {:>4}",
        "symbology", "expected", "read", "rate", "FP"
    );
    for (name, s) in rows(stats) {
        println!(
            "{:<10} {:>9} {:>6} {:>8} {:>4}",
            name,
            s.expected,
            s.read,
            rate_text(&s),
            s.false_positives
        );
    }
    let t = stats.totals();
    println!(
        "{:<10} {:>9} {:>6} {:>8} {:>4}",
        "TOTAL",
        t.expected,
        t.read,
        rate_text(&t),
        t.false_positives
    );
    let l = latency(stats);
    println!(
        "scan errors {}, unsupported (excluded) {}, latency ms p50 {:.2} p95 {:.2} max {:.2}\n",
        stats.errors, stats.unsupported, l.p50, l.p95, l.max
    );
}

pub fn stats_json(stats: &Stats) -> Value {
    let sym = |s: &SymStats| {
        json!({
            "expected": s.expected,
            "read": s.read,
            "read_rate": rate(s),
            "false_positives": s.false_positives,
        })
    };
    let symbologies: serde_json::Map<String, Value> = rows(stats)
        .iter()
        .map(|(n, s)| ((*n).to_string(), sym(s)))
        .collect();
    let l = latency(stats);
    json!({
        "images": stats.images,
        "scan_errors": stats.errors,
        "unsupported": stats.unsupported,
        "symbologies": symbologies,
        "total": sym(&stats.totals()),
        "latency_ms": { "p50": l.p50, "p95": l.p95, "max": l.max },
    })
}
