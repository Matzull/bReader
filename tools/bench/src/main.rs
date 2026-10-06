#[cfg(feature = "baselines")]
mod baselines;
mod engine;
mod groundtruth;
mod matching;
mod report;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use breader::Effort;
use serde_json::{Value, json};

use engine::{BreaderEngine, Engine};
use groundtruth::Expected;
use matching::Stats;

const USAGE: &str = "usage: bench <dataset-dir> [--engine breader|rxing|rqrr|all] \
[--effort fast|balanced|exhaustive] [--json out.json]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EngineSel {
    Breader,
    Rxing,
    Rqrr,
    All,
}

struct Args {
    dataset: PathBuf,
    engine: EngineSel,
    effort: Effort,
    json: Option<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut dataset = None;
    let mut engine = EngineSel::Breader;
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
            "--engine" => {
                engine = match args.next().as_deref() {
                    Some("breader") => EngineSel::Breader,
                    Some("rxing") => EngineSel::Rxing,
                    Some("rqrr") => EngineSel::Rqrr,
                    Some("all") => EngineSel::All,
                    other => return Err(format!("invalid --engine value {other:?}")),
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
        engine,
        effort,
        json,
    })
}

fn build_engines(sel: EngineSel, effort: Effort) -> Result<Vec<Box<dyn Engine>>, String> {
    let mut engines: Vec<Box<dyn Engine>> = Vec::new();
    if matches!(sel, EngineSel::Breader | EngineSel::All) {
        engines.push(Box::new(BreaderEngine::new(effort)));
    }
    #[cfg(feature = "baselines")]
    {
        if matches!(sel, EngineSel::Rxing | EngineSel::All) {
            engines.push(Box::new(engine::RxingEngine));
        }
        if matches!(sel, EngineSel::Rqrr | EngineSel::All) {
            engines.push(Box::new(engine::RqrrEngine));
        }
    }
    #[cfg(not(feature = "baselines"))]
    if matches!(sel, EngineSel::Rxing | EngineSel::Rqrr) {
        return Err(
            "rxing and rqrr need the `baselines` cargo feature (--features baselines)".into(),
        );
    }
    Ok(engines)
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

struct Dataset {
    name: String,
    images: Vec<(PathBuf, Vec<Expected>)>,
}

fn load_datasets(root: &Path) -> Result<Vec<Dataset>, String> {
    let mut files = Vec::new();
    find_annotation_files(root, &mut files);
    if files.is_empty() {
        return Err(format!(
            "no annotations.json found under {} (video-based-testing CSV ground truth is not supported yet)",
            root.display()
        ));
    }
    let mut datasets = Vec::new();
    for ann_path in &files {
        let ann = match groundtruth::load_annotations(ann_path) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("skipping {}: {e}", ann_path.display());
                continue;
            }
        };
        let base = ann_path.parent().unwrap_or(Path::new("."));
        let name = match base.strip_prefix(root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
            _ => root
                .canonicalize()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| root.display().to_string()),
        };
        let images = groundtruth::expected_by_file(&ann)
            .into_iter()
            .map(|(file, expected)| (base.join(file), expected))
            .collect();
        datasets.push(Dataset { name, images });
    }
    Ok(datasets)
}

/// Decodes by content, not extension (some `.png` files in the datasets are JPEGs).
fn load_gray(path: &Path) -> Result<image::GrayImage, String> {
    let reader = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?;
    Ok(reader.decode().map_err(|e| e.to_string())?.into_luma8())
}

fn run_dataset(engine: &mut dyn Engine, ds: &Dataset) -> Stats {
    let mut stats = Stats::default();
    for (path, expected) in &ds.images {
        stats.images += 1;
        let gray = match load_gray(path) {
            Ok(img) => img,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                stats.errors += 1;
                matching::score(&[], expected, &mut stats);
                continue;
            }
        };
        let start = Instant::now();
        let result = engine.decode(&gray);
        stats.latencies.push(start.elapsed());
        let found = result.unwrap_or_else(|e| {
            eprintln!("{}: {e}", path.display());
            stats.errors += 1;
            Vec::new()
        });
        matching::score(&found, expected, &mut stats);
    }
    stats
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut engines = match build_engines(args.engine, args.effort) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let datasets = match load_datasets(&args.dataset) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let mut reports: Vec<Value> = Vec::new();
    for engine in &mut engines {
        let mut total = Stats::default();
        let mut per_dataset: BTreeMap<String, Value> = BTreeMap::new();
        for ds in &datasets {
            let stats = run_dataset(engine.as_mut(), ds);
            report::print_table(engine.name(), &ds.name, &stats);
            per_dataset.insert(ds.name.clone(), report::stats_json(&stats));
            total.merge(&stats);
        }
        if datasets.len() > 1 {
            report::print_table(engine.name(), "ALL DATASETS", &total);
        }
        reports.push(json!({
            "engine": engine.name(),
            "effort": format!("{:?}", args.effort),
            "datasets": per_dataset,
            "total": report::stats_json(&total),
        }));
    }

    if let Some(path) = args.json {
        let doc = json!({ "dataset_root": args.dataset.display().to_string(), "engines": reports });
        let text = serde_json::to_string_pretty(&doc).unwrap_or_default();
        if let Err(e) = std::fs::write(&path, text) {
            eprintln!("cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
