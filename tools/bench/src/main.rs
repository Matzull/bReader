mod engine;
mod groundtruth;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use breader::Effort;
use serde_json::json;

use engine::{BreaderEngine, Engine};
use groundtruth::Expected;

const USAGE: &str =
    "usage: bench <dataset-dir> [--effort fast|balanced|exhaustive] [--json out.json]";

struct Args {
    dataset: PathBuf,
    effort: Effort,
    json: Option<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut dataset = None;
    let mut effort = Effort::Balanced;
    let mut json = None;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--effort" => {
                effort = match args.next().as_deref() {
                    Some("fast") => Effort::Fast,
                    Some("balanced") => Effort::Balanced,
                    Some("exhaustive") => Effort::Exhaustive,
                    other => return Err(format!("invalid --effort value {other:?}")),
                };
            }
            "--json" => json = Some(args.next().ok_or("--json needs a path")?.into()),
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ if dataset.is_none() => dataset = Some(PathBuf::from(arg)),
            _ => return Err("multiple dataset directories given".into()),
        }
    }
    Ok(Args {
        dataset: dataset.ok_or("missing <dataset-dir>")?,
        effort,
        json,
    })
}

fn find_annotation_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != ".git") {
                find_annotation_files(&p, out);
            }
        } else if p.file_name().is_some_and(|n| n == "annotations.json") {
            out.push(p);
        }
    }
}

#[derive(Default)]
struct Stats {
    images: usize,
    errors: usize,
    expected: usize,
    unsupported: usize,
    correct: usize,
    false_positives: usize,
    latencies: Vec<Duration>,
}

fn run_image(engine: &mut dyn Engine, path: &Path, expected: &[Expected], stats: &mut Stats) {
    let start = Instant::now();
    let result = engine.scan_file(path);
    stats.latencies.push(start.elapsed());
    stats.images += 1;

    let found = result.unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        stats.errors += 1;
        Vec::new()
    });
    let mut used = vec![false; found.len()];

    for exp in expected {
        let Some(sym) = exp.symbology else {
            stats.unsupported += 1;
            continue;
        };
        stats.expected += 1;
        let hit = found
            .iter()
            .enumerate()
            .find(|(i, (s, d))| !used[*i] && *s == sym && *d == exp.data);
        if let Some((i, _)) = hit {
            used[i] = true;
            stats.correct += 1;
        }
    }

    for (i, (sym, data)) in found.iter().enumerate() {
        if used[i] {
            continue;
        }
        let known = expected
            .iter()
            .any(|e| e.symbology == Some(*sym) && e.data == *data);
        if !known {
            stats.false_positives += 1;
        }
    }
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

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    let mut files = Vec::new();
    find_annotation_files(&args.dataset, &mut files);
    if files.is_empty() {
        eprintln!(
            "no annotations.json found under {} (video-based-testing CSV ground truth is not supported yet)",
            args.dataset.display()
        );
        return ExitCode::FAILURE;
    }

    let mut engine = BreaderEngine::new(args.effort);
    let mut stats = Stats::default();

    for ann_path in &files {
        let ann = match groundtruth::load_annotations(ann_path) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("skipping {}: {e}", ann_path.display());
                continue;
            }
        };
        let base = ann_path.parent().unwrap_or(Path::new("."));
        for (file, expected) in &groundtruth::expected_by_file(&ann) {
            run_image(&mut engine, &base.join(file), expected, &mut stats);
        }
    }

    stats.latencies.sort();
    let rate = if stats.expected == 0 {
        0.0
    } else {
        stats.correct as f64 / stats.expected as f64
    };
    let (p50, p95, max) = (
        ms(percentile(&stats.latencies, 0.5)),
        ms(percentile(&stats.latencies, 0.95)),
        ms(stats.latencies.last().copied().unwrap_or_default()),
    );

    println!("engine:           {}", engine.name());
    println!("images:           {}", stats.images);
    println!("scan errors:      {}", stats.errors);
    println!("expected codes:   {}", stats.expected);
    println!("unsupported:      {} (excluded)", stats.unsupported);
    println!("decoded correctly:{:>4}", stats.correct);
    println!("false positives:  {}", stats.false_positives);
    println!("read rate:        {:.2}%", rate * 100.0);
    println!("latency ms:       p50 {p50:.2}  p95 {p95:.2}  max {max:.2}");

    if let Some(path) = args.json {
        let report = json!({
            "engine": engine.name(),
            "effort": format!("{:?}", args.effort),
            "images": stats.images,
            "scan_errors": stats.errors,
            "expected": stats.expected,
            "unsupported": stats.unsupported,
            "correct": stats.correct,
            "false_positives": stats.false_positives,
            "read_rate": rate,
            "latency_ms": { "p50": p50, "p95": p95, "max": max },
        });
        let text = serde_json::to_string_pretty(&report).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
