//! `breader`: command line front end of the breader reader.

mod args;
mod json;

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;

use args::{Command, Format, ScanArgs, symbology_id};
use breader::{Content, PixelFormat, Reader, ScanOptions, Symbol, gs1};

/// One thing to scan: a file, or the bytes read from stdin.
enum Input {
    File(PathBuf),
    Stdin(Vec<u8>),
}

impl Input {
    fn label(&self) -> String {
        match self {
            Self::File(p) => p.display().to_string(),
            Self::Stdin(_) => "-".into(),
        }
    }
}

fn main() -> ExitCode {
    let command = match args::parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("breader: {msg}\nTry `breader --help`.");
            return ExitCode::from(2);
        }
    };
    match command {
        Command::MainHelp => print!("{}", args::MAIN_HELP),
        Command::ScanHelp => print!("{}", args::SCAN_HELP),
        Command::Version => println!("breader {}", env!("CARGO_PKG_VERSION")),
        Command::Scan(scan) => return run_scan(scan),
    }
    ExitCode::SUCCESS
}

fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg"))
}

/// Adds the images below `dir` to `out`; unreadable directories are reported and skipped.
fn collect_dir(dir: &Path, out: &mut Vec<PathBuf>, failed: &mut bool) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("breader: {}: {e}", dir.display());
            *failed = true;
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            *failed = true;
            continue;
        };
        let path = entry.path();
        // Symlinked directories are not followed, so a loop cannot trap the walk.
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            collect_dir(&path, out, failed);
        } else if has_image_extension(&path) && path.is_file() {
            out.push(path);
        }
    }
}

/// Expands the arguments into inputs, in order; a directory contributes its images sorted by path.
fn expand(paths: &[String], failed: &mut bool) -> Vec<Input> {
    let mut inputs = Vec::new();
    let mut stdin_seen = false;
    for arg in paths {
        if arg == "-" {
            if stdin_seen {
                eprintln!("breader: `-` given more than once, ignoring it");
                *failed = true;
                continue;
            }
            stdin_seen = true;
            let mut bytes = Vec::new();
            match io::stdin().lock().read_to_end(&mut bytes) {
                Ok(_) => inputs.push(Input::Stdin(bytes)),
                Err(e) => {
                    eprintln!("breader: -: {e}");
                    *failed = true;
                }
            }
            continue;
        }
        let path = Path::new(arg);
        if path.is_dir() {
            let mut found = Vec::new();
            collect_dir(path, &mut found, failed);
            found.sort();
            inputs.extend(found.into_iter().map(Input::File));
        } else {
            // A missing file is reported by the scan, in input order.
            inputs.push(Input::File(path.to_path_buf()));
        }
    }
    inputs
}

fn scan_input(
    reader: &mut Reader,
    input: &Input,
) -> Result<Vec<Symbol>, Box<dyn std::error::Error>> {
    match input {
        Input::File(path) => Ok(reader.scan_file(path)?),
        Input::Stdin(bytes) => {
            let gray = image::load_from_memory(bytes)?.into_luma8();
            let (w, h) = (gray.width() as usize, gray.height() as usize);
            Ok(reader.scan_pixels(gray.as_raw(), PixelFormat::Luma8, w, h, w)?)
        }
    }
}

type Outcome = Result<Vec<Symbol>, String>;

/// Scans every input on `jobs` threads, one `Reader` each, and hands the outcomes to `emit` in
/// input order as soon as they are in sequence.
fn scan_all(
    inputs: &[Input],
    options: &ScanOptions,
    jobs: usize,
    mut emit: impl FnMut(usize, Outcome),
) {
    let jobs = jobs.clamp(1, inputs.len().max(1));
    if jobs == 1 {
        let mut reader = Reader::with_options(options.clone());
        for (i, input) in inputs.iter().enumerate() {
            emit(i, scan_input(&mut reader, input).map_err(|e| e.to_string()));
        }
        return;
    }
    let next = AtomicUsize::new(0);
    let (tx, rx) = mpsc::channel::<(usize, Outcome)>();
    thread::scope(|scope| {
        for _ in 0..jobs {
            let tx = tx.clone();
            let next = &next;
            scope.spawn(move || {
                let mut reader = Reader::with_options(options.clone());
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(input) = inputs.get(i) else { break };
                    let outcome = scan_input(&mut reader, input).map_err(|e| e.to_string());
                    if tx.send((i, outcome)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(tx);
        let mut pending = BTreeMap::new();
        let mut want = 0;
        for (i, outcome) in rx {
            pending.insert(i, outcome);
            while let Some(outcome) = pending.remove(&want) {
                emit(want, outcome);
                want += 1;
            }
        }
    });
}

fn run_scan(scan: ScanArgs) -> ExitCode {
    let mut failed = false;
    let inputs = expand(&scan.paths, &mut failed);
    let mut found = 0usize;
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    let mut broken_pipe = false;
    scan_all(&inputs, &scan.options, scan.jobs, |i, outcome| {
        let label = inputs[i].label();
        match outcome {
            Ok(symbols) => {
                found += symbols.len();
                let mut text = String::new();
                for s in &symbols {
                    match scan.format {
                        Format::Text => text_line(&mut text, &label, s),
                        Format::Json => json_line(&mut text, &label, s),
                    }
                }
                // Flush per input so a consumer sees results as they are in order.
                if !broken_pipe && (out.write_all(text.as_bytes()).is_err() || out.flush().is_err())
                {
                    broken_pipe = true;
                }
            }
            Err(msg) => {
                eprintln!("breader: {label}: {msg}");
                failed = true;
            }
        }
    });
    if failed {
        ExitCode::from(2)
    } else if found == 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Payload for the text format: tabs, newlines, backslashes and controls escaped so that a symbol
/// is always one line of three fields.
fn text_payload(s: &Symbol) -> String {
    let Some(text) = &s.text else {
        return format!("hex:{}", hex(&s.data));
    };
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                // Controls of the C0 and C1 blocks all fit in two hex digits.
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out
}

fn text_line(out: &mut String, path: &str, s: &Symbol) {
    let _ = writeln!(
        out,
        "{path}\t{}\t{}",
        symbology_id(s.symbology),
        text_payload(s)
    );
}

fn content_kind(c: &Content) -> &'static str {
    match c {
        Content::Text => "text",
        Content::Binary => "binary",
        Content::Url => "url",
        Content::Gs1(_) => "gs1",
        _ => "unparsed",
    }
}

fn json_line(out: &mut String, path: &str, s: &Symbol) {
    out.push_str("{\"path\":");
    json::push_string(out, path);
    out.push_str(",\"symbology\":");
    json::push_string(out, symbology_id(s.symbology));
    out.push_str(",\"text\":");
    match &s.text {
        Some(t) => json::push_string(out, t),
        None => out.push_str("null"),
    }
    let _ = write!(out, ",\"data_hex\":\"{}\"", hex(&s.data));
    out.push_str(",\"content\":");
    json::push_string(out, content_kind(&s.content));
    out.push_str(",\"gs1_hri\":");
    match &s.content {
        Content::Gs1(elements) => json::push_string(out, &gs1::hri(elements)),
        _ => out.push_str("null"),
    }
    out.push_str(",\"corners\":[");
    for (i, p) in s.quad.corners.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push('[');
        json::push_f32(out, p.x);
        out.push(',');
        json::push_f32(out, p.y);
        out.push(']');
    }
    out.push_str("],\"orientation\":");
    json::push_f32(out, s.orientation_deg);
    let aim: Vec<u8> = s.aim_id.iter().copied().filter(|b| *b != 0).collect();
    out.push_str(",\"aim_id\":");
    json::push_string(out, &String::from_utf8_lossy(&aim));
    out.push_str(",\"quality\":");
    json::push_f32(out, s.quality);
    out.push_str("}\n");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_are_matched_case_insensitively() {
        for yes in ["a.png", "a.PNG", "dir/a.JpG", "a.jpeg"] {
            assert!(has_image_extension(Path::new(yes)), "{yes}");
        }
        for no in ["a.gif", "png", "a.png.txt", ""] {
            assert!(!has_image_extension(Path::new(no)), "{no}");
        }
    }

    #[test]
    fn hex_is_lowercase_pairs() {
        assert_eq!(hex(&[0, 0xab, 0xff]), "00abff");
    }
}
