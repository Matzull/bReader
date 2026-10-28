//! Runs the built `breader` binary on images generated here.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use qrcode::{Color, EcLevel, QrCode};

/// A fresh directory under the system temp dir, removed on drop.
struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("breader-cli-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Encoded PNG of a QR code of `text`, 6 pixels per module, quiet zone of 4 modules.
fn qr_png(text: &str) -> Vec<u8> {
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).unwrap();
    let (dim, px) = (code.width(), 6);
    let side = (dim + 8) * px;
    let colors = code.to_colors();
    let mut pixels = vec![255u8; side * side];
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / px, y / px);
            if (4..4 + dim).contains(&mx)
                && (4..4 + dim).contains(&my)
                && colors[(my - 4) * dim + mx - 4] == Color::Dark
            {
                pixels[y * side + x] = 0;
            }
        }
    }
    let img = image::GrayImage::from_raw(side as u32, side as u32, pixels).unwrap();
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn write_qr(dir: &TempDir, name: &str, text: &str) -> PathBuf {
    let path = dir.path(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, qr_png(text)).unwrap();
    path
}

fn breader(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_breader"))
        .args(args)
        .output()
        .unwrap()
}

fn run(args: &[&std::ffi::OsStr]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_breader"))
        .args(args)
        .output()
        .unwrap()
}

fn scan(args: &[&str], paths: &[&Path]) -> Output {
    let mut all: Vec<&std::ffi::OsStr> = vec!["scan".as_ref()];
    all.extend(args.iter().map(|a| std::ffi::OsStr::new(*a)));
    all.extend(paths.iter().map(|p| p.as_os_str()));
    run(&all)
}

fn stdout(o: &Output) -> String {
    String::from_utf8(o.stdout.clone()).unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8(o.stderr.clone()).unwrap()
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap()
}

#[test]
fn text_output_is_one_tab_separated_line() {
    let dir = TempDir::new();
    let path = write_qr(&dir, "a.png", "hello world");
    let o = scan(&[], &[&path]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), format!("{}\tqr\thello world\n", path.display()));
    assert!(stderr(&o).is_empty());
}

#[test]
fn control_characters_stay_on_one_line() {
    let dir = TempDir::new();
    let path = write_qr(&dir, "a.png", "a\tb\nc\\d");
    let o = scan(&[], &[&path]);
    assert_eq!(code(&o), 0);
    assert_eq!(
        stdout(&o),
        format!("{}\tqr\ta\\tb\\nc\\\\d\n", path.display())
    );
}

#[test]
fn json_output_is_one_object_per_line() {
    let dir = TempDir::new();
    let path = write_qr(&dir, "a.png", "say \"hi\"\t!");
    let o = scan(&["--format", "json"], &[&path]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let out = stdout(&o);
    assert_eq!(out.lines().count(), 1);
    let line = out.trim_end();
    assert!(line.starts_with("{\"path\":\""), "{line}");
    assert!(line.contains("\"symbology\":\"qr\""), "{line}");
    assert!(line.contains("\"text\":\"say \\\"hi\\\"\\t!\""), "{line}");
    assert!(
        line.contains(&format!(
            "\"data_hex\":\"{}\"",
            "say \"hi\"\t!"
                .bytes()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        )),
        "{line}"
    );
    assert!(line.contains("\"content\":\"text\""), "{line}");
    assert!(line.contains("\"gs1_hri\":null"), "{line}");
    assert!(line.contains("\"corners\":[["), "{line}");
    assert!(line.contains("\"orientation\":"), "{line}");
    assert!(line.contains("\"aim_id\":\"]Q"), "{line}");
    assert!(line.contains("\"quality\":"), "{line}");
    assert!(line.ends_with('}'));
    // Four corners of two numbers each.
    let corners = line.split("\"corners\":").nth(1).unwrap();
    let corners = &corners[..corners.find("],\"orientation\"").unwrap() + 1];
    assert_eq!(corners.matches('[').count(), 5, "{corners}");
}

#[test]
fn exit_code_one_when_nothing_is_found() {
    let dir = TempDir::new();
    let path = dir.path("blank.png");
    image::GrayImage::from_pixel(64, 64, image::Luma([255]))
        .save(&path)
        .unwrap();
    let o = scan(&[], &[&path]);
    assert_eq!(code(&o), 1);
    assert!(stdout(&o).is_empty() && stderr(&o).is_empty());
}

#[test]
fn exit_code_two_for_usage_errors() {
    for args in [
        &[][..],
        &["frobnicate"],
        &["scan"],
        &["scan", "--effort", "slow", "x.png"],
        &["scan", "--bogus", "x.png"],
    ] {
        let o = breader(args);
        assert_eq!(code(&o), 2, "{args:?}");
        assert!(stdout(&o).is_empty(), "{args:?}");
        assert!(!stderr(&o).is_empty(), "{args:?}");
    }
}

#[test]
fn unreadable_input_is_reported_and_the_rest_is_scanned() {
    let dir = TempDir::new();
    let good = write_qr(&dir, "good.png", "still here");
    let missing = dir.path("missing.png");
    let garbage = dir.path("garbage.png");
    std::fs::write(&garbage, b"not an image").unwrap();
    let o = scan(&[], &[&missing, &good, &garbage]);
    assert_eq!(code(&o), 2);
    assert_eq!(stdout(&o), format!("{}\tqr\tstill here\n", good.display()));
    let err = stderr(&o);
    assert!(
        err.contains("missing.png") && err.contains("garbage.png"),
        "{err}"
    );
}

#[test]
fn jobs_keep_the_input_order() {
    let dir = TempDir::new();
    let paths: Vec<PathBuf> = (0..12)
        .map(|i| write_qr(&dir, &format!("{:02}.png", 11 - i), &format!("item {i}")))
        .collect();
    let refs: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
    let serial = scan(&["--jobs", "1"], &refs);
    assert_eq!(code(&serial), 0, "{}", stderr(&serial));
    let expected: String = paths
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{}\tqr\titem {i}\n", p.display()))
        .collect();
    assert_eq!(stdout(&serial), expected);
    for jobs in ["2", "4", "0", "64"] {
        let o = scan(&["-j", jobs], &refs);
        assert_eq!(code(&o), 0, "{}", stderr(&o));
        assert_eq!(stdout(&o), expected, "jobs {jobs}");
    }
}

#[test]
fn symbologies_filter_the_result() {
    let dir = TempDir::new();
    let path = write_qr(&dir, "a.png", "filter me");
    let only_qr = scan(&["--symbologies", "qr,ean13"], &[&path]);
    assert_eq!(code(&only_qr), 0);
    let no_qr = scan(&["--symbologies", "ean13,code128"], &[&path]);
    assert_eq!(code(&no_qr), 1);
    assert!(stdout(&no_qr).is_empty());
    let unknown = scan(&["--symbologies", "qr,nonsense"], &[&path]);
    assert_eq!(code(&unknown), 2);
    assert!(stderr(&unknown).contains("nonsense"));
}

#[test]
fn a_directory_is_searched_recursively_in_sorted_order() {
    let dir = TempDir::new();
    write_qr(&dir, "b.png", "bravo");
    write_qr(&dir, "sub/a.PNG", "alpha");
    write_qr(&dir, "sub/deeper/c.jpg.png", "charlie");
    std::fs::write(dir.path("notes.txt"), b"ignored").unwrap();
    let o = scan(&[], &[&dir.0]);
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    let texts: Vec<String> = stdout(&o)
        .lines()
        .map(|l| l.split('\t').nth(2).unwrap().to_string())
        .collect();
    // Sorted by path: b.png, sub/a.PNG, sub/deeper/c.jpg.png.
    assert_eq!(texts, ["bravo", "alpha", "charlie"]);
    // An empty directory finds nothing, which is not an error.
    let empty = TempDir::new();
    assert_eq!(code(&scan(&[], &[&empty.0])), 1);
}

#[test]
fn stdin_is_decoded() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_breader"))
        .args(["scan", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&qr_png("from stdin"))
        .unwrap();
    let o = child.wait_with_output().unwrap();
    assert_eq!(code(&o), 0, "{}", stderr(&o));
    assert_eq!(stdout(&o), "-\tqr\tfrom stdin\n");
}

#[test]
fn version_and_help() {
    let v = breader(&["--version"]);
    assert_eq!(code(&v), 0);
    assert_eq!(
        stdout(&v),
        format!("breader {}\n", env!("CARGO_PKG_VERSION"))
    );
    let h = breader(&["--help"]);
    assert_eq!(code(&h), 0);
    assert!(stdout(&h).contains("breader scan"));
    let s = breader(&["scan", "--help"]);
    assert_eq!(code(&s), 0);
    let text = stdout(&s);
    for needle in [
        "--effort",
        "--symbologies",
        "--jobs",
        "--format",
        "Exit status",
    ] {
        assert!(text.contains(needle), "{needle}");
    }
}
