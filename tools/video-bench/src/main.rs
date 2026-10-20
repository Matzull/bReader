//! Evaluates the video tracker against stateless per-frame scanning on the
//! Dynamsoft `video-based-testing` clips.

mod expected;
mod metrics;
mod video;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use benchlib::gate::{Baseline, Counts, Groups};
use benchlib::matching::canonical;
use breader::{
    Effort, FrameUpdate, Reader, ScanOptions, Symbol, TrackEvent, Tracker, TrackerConfig,
};
use serde_json::{Value, json};

use metrics::{Acc, Key, Report};

const USAGE: &str = "usage: video-bench <dataset-dir> [--engines tracker,fast,balanced] \
[--filter substr] [--max-frames N] [--jobs N] [--json out.json] [--verbose] \
[--check baseline.json | --write-baseline baseline.json] [--strips N] [--overlap F] [--lost N] [--margin F] [--roi-effort E] [--global-effort E] [--full-effort E]";

struct Args {
    dataset: PathBuf,
    engines: Vec<String>,
    filter: Option<String>,
    max_frames: Option<u64>,
    json: Option<PathBuf>,
    verbose: bool,
    jobs: usize,
    check: Option<PathBuf>,
    write_baseline: Option<PathBuf>,
    config: TrackerConfig,
}

fn effort(s: Option<String>) -> Result<Effort, String> {
    match s.as_deref() {
        Some("fast") => Ok(Effort::Fast),
        Some("balanced") => Ok(Effort::Balanced),
        Some("exhaustive") => Ok(Effort::Exhaustive),
        other => Err(format!("invalid effort {other:?}")),
    }
}

fn num<T: std::str::FromStr>(s: Option<String>, flag: &str) -> Result<T, String> {
    s.as_deref()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| format!("invalid value for {flag}"))
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut a = Args {
        dataset: PathBuf::new(),
        engines: vec!["tracker".into(), "fast".into(), "balanced".into()],
        filter: None,
        max_frames: None,
        json: None,
        verbose: false,
        jobs: 1,
        check: None,
        write_baseline: None,
        config: TrackerConfig::default(),
    };
    let mut dataset = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--engines" => {
                let v = args.next().ok_or("--engines needs a value")?;
                a.engines = v.split(',').map(str::to_owned).collect();
                if let Some(bad) = a
                    .engines
                    .iter()
                    .find(|e| !["tracker", "fast", "balanced"].contains(&e.as_str()))
                {
                    return Err(format!("unknown engine {bad}"));
                }
            }
            "--filter" => a.filter = args.next(),
            "--max-frames" => a.max_frames = Some(num(args.next(), "--max-frames")?),
            "--json" => a.json = args.next().map(PathBuf::from),
            "--verbose" => a.verbose = true,
            "--jobs" => {
                a.jobs = num(args.next(), "--jobs")?;
                if a.jobs == 0 {
                    return Err("--jobs needs a positive integer".into());
                }
            }
            "--check" => a.check = Some(args.next().ok_or("--check needs a path")?.into()),
            "--write-baseline" => {
                a.write_baseline = Some(args.next().ok_or("--write-baseline needs a path")?.into());
            }
            "--strips" => a.config.strips = num(args.next(), "--strips")?,
            "--overlap" => a.config.strip_overlap = num(args.next(), "--overlap")?,
            "--lost" => a.config.lost_after = num(args.next(), "--lost")?,
            "--margin" => a.config.roi_margin = num(args.next(), "--margin")?,
            "--roi-effort" => a.config.roi_effort = effort(args.next())?,
            "--global-effort" => a.config.global_effort = effort(args.next())?,
            "--full-effort" => a.config.full_effort = effort(args.next())?,
            s if s.starts_with("--") => return Err(format!("unknown flag {s}")),
            _ => dataset = Some(PathBuf::from(arg)),
        }
    }
    a.dataset = dataset.ok_or("missing dataset directory")?;
    if a.check.is_some() && a.write_baseline.is_some() {
        return Err("--check and --write-baseline are mutually exclusive".into());
    }
    if (a.check.is_some() || a.write_baseline.is_some())
        && (a.filter.is_some() || a.max_frames.is_some())
    {
        return Err(
            "--check/--write-baseline cannot be combined with --filter or --max-frames".into(),
        );
    }
    Ok(a)
}

enum Engine {
    Tracker(Box<Tracker>),
    Stateless(Box<Reader>),
}

struct Runner {
    name: String,
    engine: Engine,
    acc: Acc,
}

fn key_of(s: &Symbol) -> Key {
    let (sym, data) = canonical(s.symbology, &s.data);
    (sym.name(), data)
}

impl Runner {
    fn new(name: &str, config: TrackerConfig) -> Self {
        let engine = match name {
            "tracker" => Engine::Tracker(Box::new(Tracker::new(ScanOptions::default(), config))),
            "fast" => Engine::Stateless(Box::new(Reader::with_options(ScanOptions::with_effort(
                Effort::Fast,
            )))),
            _ => Engine::Stateless(Box::new(Reader::with_options(ScanOptions::with_effort(
                Effort::Balanced,
            )))),
        };
        Self {
            name: name.to_owned(),
            engine,
            acc: Acc::default(),
        }
    }

    fn frame(&mut self, index: u64, view: breader::ImageView<'_>) {
        let t0 = Instant::now();
        match &mut self.engine {
            Engine::Stateless(reader) => {
                let found = reader.scan(view);
                self.acc.latency(t0.elapsed());
                let keys: Vec<Key> = found.iter().map(key_of).collect();
                self.acc.observe_stateless(index, keys);
            }
            Engine::Tracker(tracker) => {
                let FrameUpdate { events, .. } = tracker.process(view);
                self.acc.latency(t0.elapsed());
                let keys: Vec<Key> = tracker.tracks().iter().map(|t| key_of(&t.symbol)).collect();
                for e in &events {
                    match *e {
                        TrackEvent::New(id) => {
                            let k = tracker
                                .tracks()
                                .iter()
                                .find(|t| t.id == id)
                                .map(|t| key_of(&t.symbol));
                            if let Some(k) = k {
                                self.acc.track_new(id, k);
                            }
                        }
                        TrackEvent::Lost(id) => self.acc.track_lost(id),
                        TrackEvent::Updated(_) => {}
                    }
                }
                self.acc.observe_tracker(index, keys);
            }
        }
    }
}

fn find_videos(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()) == Some("mp4") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn gate_groups(reports: &[Report]) -> Groups {
    let mut groups = Groups::new();
    for r in reports {
        groups.entry(r.video.clone()).or_default().insert(
            r.engine.clone(),
            Counts {
                expected: r.expected,
                read: r.read,
                false_positives: r.false_positives.len(),
            },
        );
    }
    groups
}

/// Returns whether the gate passed (always true when no gate flag is given).
fn run_gate(args: &Args, reports: &[Report]) -> Result<bool, String> {
    let groups = gate_groups(reports);
    if let Some(path) = &args.write_baseline {
        Baseline::validate_for_write(&groups)?;
        Baseline::new(None, groups).save(path)?;
        println!("baseline written to {}", path.display());
    } else if let Some(path) = &args.check {
        let mut baseline = Baseline::load(path)?;
        // Engines that were not run are not part of this check (`--engines tracker`).
        baseline.retain_items(&args.engines);
        let outcome = baseline.check(None, &groups);
        outcome.print();
        return Ok(outcome.passed());
    }
    Ok(true)
}

/// Reports of one video, or the reason it could not be processed.
type VideoResult = Result<Vec<Report>, String>;

/// Runs every engine over one video; no reports when its annotation csv is missing.
fn run_video(args: &Args, path: &Path) -> VideoResult {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_owned();
    let csv = path
        .parent()
        .map(|p| p.join("annotations").join(format!("{name}.csv")));
    let Some(expected) = csv.as_deref().and_then(|c| expected::load(c).ok()) else {
        eprintln!("skip {name}: no readable annotation csv");
        return Ok(Vec::new());
    };
    let mut runners: Vec<Runner> = args
        .engines
        .iter()
        .map(|e| Runner::new(e, args.config))
        .collect();
    let frames = video::for_each_frame(path, args.max_frames, |index, view| {
        for r in &mut runners {
            r.frame(index, view);
        }
    })?;
    eprintln!("{name}: {frames} frames");
    Ok(runners
        .into_iter()
        .map(|r| Report::build(&name, &r.name, frames, &expected, r.acc))
        .collect())
}

/// One video per task over `args.jobs` threads (each video keeps its own sequential trackers);
/// results are returned in the order of `paths`.
fn run_videos_parallel(args: &Args, paths: &[PathBuf]) -> Vec<VideoResult> {
    const STACK: usize = 16 << 20;
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<VideoResult>>> = paths.iter().map(|_| Mutex::new(None)).collect();
    std::thread::scope(|scope| {
        for _ in 0..args.jobs.min(paths.len()) {
            let worker = || {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(path) = paths.get(i) else { break };
                    let result = run_video(args, path);
                    *slots[i].lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
                }
            };
            std::thread::Builder::new()
                .stack_size(STACK)
                .spawn_scoped(scope, worker)
                .expect("cannot spawn a video-bench worker thread");
        }
    });
    slots
        .into_iter()
        .map(|slot| {
            slot.into_inner()
                .unwrap_or_else(|e| e.into_inner())
                .unwrap_or_else(|| Err("video not processed".into()))
        })
        .collect()
}

fn run() -> Result<bool, String> {
    let args = parse_args(std::env::args().skip(1)).map_err(|e| format!("{e}\n{USAGE}"))?;
    let videos = find_videos(&args.dataset);
    if videos.is_empty() {
        return Err(format!("no mp4 files under {}", args.dataset.display()));
    }
    let paths: Vec<PathBuf> = videos
        .into_iter()
        .filter(|path| {
            let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
            args.filter.as_ref().is_none_or(|f| name.contains(f))
        })
        .collect();
    let results = if args.jobs > 1 {
        eprintln!("--jobs {}: latencies are not valid", args.jobs);
        run_videos_parallel(&args, &paths)
    } else {
        paths.iter().map(|p| run_video(&args, p)).collect()
    };
    // Reports keep the order of the videos whatever the number of jobs.
    let mut reports: Vec<Report> = Vec::new();
    for r in results {
        reports.extend(r?);
    }
    metrics::print_table(&reports, args.verbose);
    if let Some(p) = &args.json {
        let v: Value = json!(reports.iter().map(Report::to_json).collect::<Vec<_>>());
        std::fs::write(
            p,
            serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("{}: {e}", p.display()))?;
    }
    run_gate(&args, &reports)
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
