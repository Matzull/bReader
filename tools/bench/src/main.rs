mod report;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use breader::{Effort, Escalation};
use image::{ImageDecoder, metadata::Orientation};
use serde_json::{Value, json};

use benchlib::gate::{Baseline, Counts, Groups};
use benchlib::subset::Subset;
use benchlib::{barber, engine, groundtruth, matching};
use engine::{BreaderEngine, Engine};
use groundtruth::Expected;
use matching::Stats;

const USAGE: &str = "usage: bench <dataset-dir> [--engine breader|rxing|rqrr|all] \
[--effort fast|balanced|exhaustive] [--escalation all|found|stable] [--json out.json] [--no-overrides] [--subset N|dev] [--jobs N] [--only SOURCE] [--check baseline.json | --write-baseline baseline.json]";

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
    escalation: Option<Escalation>,
    json: Option<PathBuf>,
    overrides: bool,
    subset: Subset,
    jobs: usize,
    only: Option<String>,
    check: Option<PathBuf>,
    write_baseline: Option<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut dataset = None;
    let mut engine = EngineSel::Breader;
    let mut effort = Effort::Balanced;
    let mut escalation = None;
    let mut json = None;
    let mut overrides = true;
    let mut subset = Subset::all();
    let mut jobs = 1;
    let mut only = None;
    let mut check = None;
    let mut write_baseline = None;
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
            "--escalation" => {
                escalation = Some(match args.next().as_deref() {
                    Some("all") => Escalation::All,
                    Some("found") => Escalation::UntilFound,
                    Some("stable") => Escalation::UntilStable,
                    other => return Err(format!("invalid --escalation value {other:?}")),
                });
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
            "--no-overrides" => overrides = false,
            "--only" => only = Some(args.next().ok_or("--only needs a name")?),
            "--subset" => {
                subset = match args.next().as_deref() {
                    Some("dev") => Subset::dev()?,
                    Some(v) => v
                        .parse()
                        .ok()
                        .filter(|&n| n >= 1)
                        .map(Subset::share)
                        .ok_or("--subset needs a positive integer or `dev`")?,
                    None => return Err("--subset needs a value".into()),
                };
            }
            "--jobs" => {
                jobs = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|&n| n >= 1)
                    .ok_or("--jobs needs a positive integer")?;
            }
            "--check" => check = Some(args.next().ok_or("--check needs a path")?.into()),
            "--write-baseline" => {
                write_baseline = Some(args.next().ok_or("--write-baseline needs a path")?.into());
            }
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ if dataset.is_none() => dataset = Some(PathBuf::from(arg)),
            _ => return Err("multiple dataset directories given".into()),
        }
    }
    if check.is_some() && write_baseline.is_some() {
        return Err("--check and --write-baseline are mutually exclusive".into());
    }
    if (check.is_some() || write_baseline.is_some()) && engine != EngineSel::Breader {
        return Err("--check/--write-baseline only work with --engine breader".into());
    }
    if (check.is_some() || write_baseline.is_some()) && (escalation.is_some() || !overrides) {
        return Err("--check/--write-baseline need the default escalation and GT overrides".into());
    }
    Ok(Args {
        dataset: dataset.ok_or("missing <dataset-dir>")?,
        engine,
        effort,
        escalation,
        json,
        overrides,
        subset,
        jobs,
        only,
        check,
        write_baseline,
    })
}

fn build_engines(
    sel: EngineSel,
    effort: Effort,
    escalation: Option<Escalation>,
) -> Result<Vec<Box<dyn Engine>>, String> {
    let mut engines: Vec<Box<dyn Engine>> = Vec::new();
    if matches!(sel, EngineSel::Breader | EngineSel::All) {
        engines.push(Box::new(BreaderEngine::new(effort, escalation)));
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
        } else if groundtruth::is_annotation_file(&p) {
            out.push(p);
        }
    }
}

struct Item {
    path: PathBuf,
    expected: Vec<Expected>,
    /// Polygons of undecodable symbols: a read there is not a false positive.
    ignore: Vec<Vec<[f64; 2]>>,
}

struct Dataset {
    name: String,
    images: Vec<Item>,
}

/// BarBeR: one dataset per source (VIA project file), ground truth converted in memory.
fn load_barber(
    via: &Path,
    use_overrides: bool,
    subset: &Subset,
    only: &Option<String>,
) -> Result<Vec<Dataset>, String> {
    let overrides = groundtruth::parse_overrides(groundtruth::OVERRIDES_JSON)?;
    let base = barber::image_dir(via);
    let mut datasets = Vec::new();
    for mut source in barber::load_sources(via)? {
        if only
            .as_ref()
            .is_some_and(|o| !source.name.contains(o.as_str()))
        {
            continue;
        }
        let name = source.name.clone();
        if use_overrides {
            let n = groundtruth::apply_overrides(
                &mut source.annotations,
                barber::OVERRIDES_KEY,
                &overrides,
            );
            if n > 0 {
                eprintln!("{name}: applied {n} GT override(s)");
            }
        }
        let removed = groundtruth::dedup_annotations(&mut source.annotations);
        eprintln!("{name}: removed {removed} duplicate GT entries");
        let mut images = Vec::new();
        for img in source.annotations.images {
            if !subset.contains(&img.file) {
                continue;
            }
            let path = base.join(&img.file);
            if !path.is_file() {
                eprintln!("{name}: skipping {}: labelled but no such image", img.file);
                continue;
            }
            let (barcodes, ignore) = barber::split_undecodable(img.barcodes);
            let ann = groundtruth::Annotations {
                format: String::new(),
                images: vec![groundtruth::ImageEntry {
                    file: img.file.clone(),
                    barcodes,
                }],
            };
            let expected = groundtruth::expected_by_file(&ann)
                .remove(&img.file)
                .unwrap_or_default();
            images.push(Item {
                path,
                expected,
                ignore,
            });
        }
        datasets.push(Dataset { name, images });
    }
    Ok(datasets)
}

fn load_datasets(
    root: &Path,
    use_overrides: bool,
    subset: &Subset,
    only: &Option<String>,
) -> Result<Vec<Dataset>, String> {
    if let Some(via) = barber::find_via_dir(root) {
        return load_barber(&via, use_overrides, subset, only);
    }
    let mut files = Vec::new();
    find_annotation_files(root, &mut files);
    if files.is_empty() {
        return Err(format!(
            "no annotations.json found under {} (video-based-testing CSV ground truth is not supported yet)",
            root.display()
        ));
    }
    let overrides = groundtruth::parse_overrides(groundtruth::OVERRIDES_JSON)?;
    let mut datasets = Vec::new();
    for ann_path in &files {
        let mut ann = match groundtruth::load_annotations(ann_path) {
            Ok(a) => a,
            Err(e) => {
                eprintln!("skipping {}: {e}", ann_path.display());
                continue;
            }
        };
        let base_buf = groundtruth::dataset_base(ann_path);
        let base = base_buf.as_path();
        let name = match base.strip_prefix(root) {
            Ok(rel) if !rel.as_os_str().is_empty() => rel.display().to_string(),
            _ => root
                .canonicalize()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| root.display().to_string()),
        };
        let dir_name = base
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_default();
        if use_overrides {
            let n = groundtruth::apply_overrides(&mut ann, &dir_name, &overrides);
            if n > 0 {
                eprintln!("{name}: applied {n} GT override(s)");
            }
        }
        let removed = groundtruth::dedup_annotations(&mut ann);
        eprintln!("{name}: removed {removed} duplicate GT entries");
        let mut index = None;
        let mut images = Vec::new();
        for (file, expected) in groundtruth::expected_by_file(&ann) {
            match groundtruth::resolve_image(base, &file, &mut index) {
                Some(path) => images.push(Item {
                    path,
                    expected,
                    ignore: Vec::new(),
                }),
                None => eprintln!("{name}: skipping {file}: labelled but no such image"),
            }
        }
        datasets.push(Dataset { name, images });
    }
    Ok(datasets)
}

/// Decodes by content, not extension (some `.png` files in the datasets are JPEGs).
fn load_gray(path: &Path) -> Result<(image::GrayImage, Orientation), String> {
    let reader = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?;
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let img = image::DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    Ok((img.into_luma8(), orientation))
}

/// Scores one image; `timed` records the decode latency (only valid when nothing else runs).
fn run_item(engine: &mut dyn Engine, item: &Item, timed: bool) -> Stats {
    let Item {
        path,
        expected,
        ignore,
    } = item;
    let mut stats = Stats {
        images: 1,
        ..Stats::default()
    };
    let (gray, orientation) = match load_gray(path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("{}: {e}", path.display());
            stats.errors += 1;
            matching::score_masked(&[], expected, ignore, &mut stats);
            return stats;
        }
    };
    // Ground-truth polygons are in the EXIF-corrected frame, the engines see the raw pixels.
    let ignore = &barber::to_raw_frame(ignore, orientation, gray.width(), gray.height());
    let start = Instant::now();
    let result = engine.decode(&gray);
    if timed {
        stats.latencies.push(start.elapsed());
    }
    let found = result.unwrap_or_else(|e| {
        eprintln!("{}: {e}", path.display());
        stats.errors += 1;
        Vec::new()
    });
    if std::env::var_os("BENCH_SHOW_FP").is_some() {
        eprintln!("image {}", path.display());
    }
    matching::score_masked(&found, expected, ignore, &mut stats);
    stats
}

fn run_dataset(engine: &mut dyn Engine, ds: &Dataset) -> Stats {
    let mut stats = Stats::default();
    for item in &ds.images {
        stats.merge(&run_item(engine, item, true));
    }
    stats
}

/// Runs the images of `ds` on `jobs` threads, each with its own engine (`make`). Per-image results are
/// merged in image order, so counts match the sequential run exactly; latencies are not recorded.
fn run_dataset_parallel(
    make: &(dyn Fn() -> Box<dyn Engine> + Sync),
    ds: &Dataset,
    jobs: usize,
) -> Stats {
    const STACK: usize = 16 << 20;
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<Stats>>> = ds.images.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(ds.images.len()) {
            let worker = || {
                let mut engine = make();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(item) = ds.images.get(i) else { break };
                    let stats = run_item(engine.as_mut(), item, false);
                    *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(stats);
                }
            };
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(scope, worker)
                .expect("cannot spawn a bench worker thread");
        }
    });
    let mut stats = Stats::default();
    for slot in slots {
        let item = slot.into_inner().unwrap_or_else(|e| e.into_inner());
        stats.merge(&item.expect("every image is scored by some worker"));
    }
    stats
}

fn gate_groups(per_dataset: &[(String, Stats)]) -> Groups {
    per_dataset
        .iter()
        .map(|(name, stats)| {
            let items = stats
                .per_symbology
                .iter()
                .map(|(sym, s)| {
                    (
                        (*sym).to_string(),
                        Counts {
                            expected: s.expected,
                            read: s.read,
                            false_positives: s.false_positives,
                        },
                    )
                })
                .collect();
            (name.clone(), items)
        })
        .collect()
}

fn effort_name(effort: Effort) -> String {
    format!("{effort:?}").to_lowercase()
}

fn run_gate(args: &Args, groups: &Groups) -> ExitCode {
    let effort = effort_name(args.effort);
    if let Some(path) = &args.write_baseline {
        let result = Baseline::validate_for_write(groups)
            .and_then(|()| Baseline::new(Some(&effort), groups.clone()).save(path));
        return match result {
            Ok(()) => {
                println!("baseline written to {}", path.display());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Some(path) = &args.check {
        return match Baseline::load(path) {
            Ok(b) => {
                let outcome = b.check(Some(&effort), groups);
                outcome.print();
                if outcome.passed() {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::FAILURE
                }
            }
            Err(e) => {
                eprintln!("{e}");
                ExitCode::FAILURE
            }
        };
    }
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let mut engines = match build_engines(args.engine, args.effort, args.escalation) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let datasets = match load_datasets(&args.dataset, args.overrides, &args.subset, &args.only) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };

    let mut reports: Vec<Value> = Vec::new();
    let mut gate_groups_slot: Option<Groups> = None;
    for (engine_idx, engine) in engines.iter_mut().enumerate() {
        let mut total = Stats::default();
        let mut per_dataset: BTreeMap<String, Value> = BTreeMap::new();
        let mut all_stats: Vec<(String, Stats)> = Vec::new();
        for ds in &datasets {
            let stats = if args.jobs > 1 {
                let make = || {
                    build_engines(args.engine, args.effort, args.escalation)
                        .map(|mut v| v.swap_remove(engine_idx))
                        .expect("engines were built once already")
                };
                run_dataset_parallel(&make, ds, args.jobs)
            } else {
                run_dataset(engine.as_mut(), ds)
            };
            report::print_table(engine.name(), &ds.name, &stats);
            per_dataset.insert(ds.name.clone(), report::stats_json(&stats));
            total.merge(&stats);
            all_stats.push((ds.name.clone(), stats));
        }
        if engine.name() == "breader" {
            gate_groups_slot = Some(gate_groups(&all_stats));
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

    if let Some(path) = &args.json {
        let doc = json!({ "dataset_root": args.dataset.display().to_string(), "engines": reports });
        let text = serde_json::to_string_pretty(&doc).unwrap_or_default();
        if let Err(e) = std::fs::write(path, text) {
            eprintln!("cannot write {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    }
    if let Some(groups) = &gate_groups_slot {
        return run_gate(&args, groups);
    }
    ExitCode::SUCCESS
}
