use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use serde_json::{Value, json};

use crate::expected::Expected;

/// Canonical (symbology name, payload) of a code.
pub type Key = (&'static str, Vec<u8>);

#[derive(Debug, Default)]
pub struct Acc {
    latencies_ms: Vec<f64>,
    max_count: BTreeMap<Key, usize>,
    first_frame: BTreeMap<Key, u64>,
    prev_present: BTreeSet<Key>,
    flicker: usize,
    track_keys: HashMap<u64, Key>,
    lost_keys: BTreeMap<Key, usize>,
}

fn counts(keys: Vec<Key>) -> BTreeMap<Key, usize> {
    let mut m = BTreeMap::new();
    for k in keys {
        *m.entry(k).or_default() += 1;
    }
    m
}

impl Acc {
    pub fn latency(&mut self, d: Duration) {
        self.latencies_ms.push(d.as_secs_f64() * 1e3);
    }

    fn observe(&mut self, frame: u64, keys: Vec<Key>) -> BTreeSet<Key> {
        let counts = counts(keys);
        for (k, n) in &counts {
            let e = self.max_count.entry(k.clone()).or_default();
            *e = (*e).max(*n);
            self.first_frame.entry(k.clone()).or_insert(frame);
        }
        counts.into_keys().collect()
    }

    /// A code that was seen, vanished from a frame and came back counts once per return.
    pub fn observe_stateless(&mut self, frame: u64, keys: Vec<Key>) {
        let seen_before: BTreeSet<Key> = self.first_frame.keys().cloned().collect();
        let present = self.observe(frame, keys);
        self.flicker += present
            .iter()
            .filter(|k| !self.prev_present.contains(*k) && seen_before.contains(*k))
            .count();
        self.prev_present = present;
    }

    /// `keys` are the live tracks after the frame.
    pub fn observe_tracker(&mut self, frame: u64, keys: Vec<Key>) {
        self.observe(frame, keys);
    }

    pub fn track_new(&mut self, id: u64, key: Key) {
        if let Some(n) = self.lost_keys.get_mut(&key)
            && *n > 0
        {
            *n -= 1;
            self.flicker += 1;
        }
        self.track_keys.insert(id, key);
    }

    pub fn track_lost(&mut self, id: u64) {
        if let Some(k) = self.track_keys.remove(&id) {
            *self.lost_keys.entry(k).or_default() += 1;
        }
    }
}

#[derive(Debug)]
pub struct Report {
    pub video: String,
    pub engine: String,
    pub frames: u64,
    pub expected: usize,
    pub read: usize,
    pub missed: Vec<String>,
    pub false_positives: Vec<String>,
    pub ttfr_mean: Option<f64>,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub flicker: usize,
}

fn show(k: &Key) -> String {
    format!("{}:{}", k.0, String::from_utf8_lossy(&k.1))
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

impl Report {
    pub fn build(video: &str, engine: &str, frames: u64, exp: &Expected, mut acc: Acc) -> Self {
        let mut read = 0;
        let mut missed = Vec::new();
        let mut firsts = Vec::new();
        for (k, &n) in &exp.codes {
            let got = acc.max_count.get(k).copied().unwrap_or(0).min(n);
            read += got;
            if got < n {
                missed.push(format!("{} x{}", show(k), n - got));
            }
            if got > 0
                && let Some(&f) = acc.first_frame.get(k)
            {
                firsts.push(f as f64);
            }
        }
        let false_positives = acc
            .max_count
            .keys()
            .filter(|k| !exp.codes.contains_key(*k))
            .map(show)
            .collect();
        acc.latencies_ms.sort_by(f64::total_cmp);
        let l = &acc.latencies_ms;
        Self {
            video: video.to_owned(),
            engine: engine.to_owned(),
            frames,
            expected: exp.codes.values().sum(),
            read,
            missed,
            false_positives,
            ttfr_mean: (!firsts.is_empty())
                .then(|| firsts.iter().sum::<f64>() / firsts.len() as f64),
            mean_ms: if l.is_empty() {
                0.0
            } else {
                l.iter().sum::<f64>() / l.len() as f64
            },
            p50_ms: percentile(l, 0.5),
            p95_ms: percentile(l, 0.95),
            max_ms: l.last().copied().unwrap_or(0.0),
            flicker: acc.flicker,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "video": self.video, "engine": self.engine, "frames": self.frames,
            "expected": self.expected, "read": self.read, "missed": self.missed,
            "false_positives": self.false_positives, "ttfr_mean_frames": self.ttfr_mean,
            "latency_ms": {"mean": self.mean_ms, "p50": self.p50_ms, "p95": self.p95_ms, "max": self.max_ms},
            "flicker": self.flicker,
        })
    }
}

pub fn print_table(reports: &[Report], verbose: bool) {
    println!(
        "{:<36} {:<9} {:>6} {:>9} {:>3} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
        "video", "engine", "frames", "read", "FP", "ttfr", "mean", "p50", "p95", "max", "flicker"
    );
    for r in reports {
        println!(
            "{:<36} {:<9} {:>6} {:>9} {:>3} {:>7} {:>7.1} {:>7.1} {:>7.1} {:>7.1} {:>7}",
            r.video,
            r.engine,
            r.frames,
            format!("{}/{}", r.read, r.expected),
            r.false_positives.len(),
            r.ttfr_mean.map_or("-".to_owned(), |t| format!("{t:.1}")),
            r.mean_ms,
            r.p50_ms,
            r.p95_ms,
            r.max_ms,
            r.flicker
        );
        if verbose {
            for m in &r.missed {
                println!("    missed: {m}");
            }
        }
        for f in &r.false_positives {
            println!("    FALSE POSITIVE: {f}");
        }
    }
    let mut engines: Vec<&str> = reports.iter().map(|r| r.engine.as_str()).collect();
    engines.dedup();
    engines.sort();
    engines.dedup();
    println!(
        "\n{:<9} {:>9} {:>4} {:>9} {:>9} {:>8}",
        "engine", "read", "FP", "mean ms", "p95 ms", "flicker"
    );
    for e in engines {
        let rs: Vec<&Report> = reports.iter().filter(|r| r.engine == e).collect();
        let frames: u64 = rs.iter().map(|r| r.frames).sum();
        let mean =
            rs.iter().map(|r| r.mean_ms * r.frames as f64).sum::<f64>() / frames.max(1) as f64;
        println!(
            "{:<9} {:>9} {:>4} {:>9.1} {:>9.1} {:>8}",
            e,
            format!(
                "{}/{}",
                rs.iter().map(|r| r.read).sum::<usize>(),
                rs.iter().map(|r| r.expected).sum::<usize>()
            ),
            rs.iter().map(|r| r.false_positives.len()).sum::<usize>(),
            mean,
            rs.iter().map(|r| r.p95_ms).fold(0.0, f64::max),
            rs.iter().map(|r| r.flicker).sum::<usize>(),
        );
    }
}
