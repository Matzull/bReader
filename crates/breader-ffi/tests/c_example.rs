//! Compiles `examples/c/scan.c` against the built library with the system C compiler and
//! scans a QR code with it. Skipped (with a note) when `cc` is missing or on Windows.

use std::path::{Path, PathBuf};
use std::process::Command;

use qrcode::{Color, EcLevel, QrCode};

const TEXT: &str = "HELLO FROM C 42";

fn have(tool: &str) -> bool {
    Command::new(tool)
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A binary PGM of a QR code.
fn pgm(text: &str) -> Vec<u8> {
    const PX: usize = 6;
    const QUIET: usize = 4;
    let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::M).unwrap();
    let dim = code.width();
    let side = (dim + 2 * QUIET) * PX;
    let colors = code.to_colors();
    let mut out = format!("P5\n# a QR code\n{side} {side}\n255\n").into_bytes();
    for y in 0..side {
        for x in 0..side {
            let (mx, my) = (x / PX, y / PX);
            let inside = mx >= QUIET && my >= QUIET && mx < QUIET + dim && my < QUIET + dim;
            let dark = inside && colors[(my - QUIET) * dim + (mx - QUIET)] == Color::Dark;
            out.push(if dark { 20 } else { 240 });
        }
    }
    out
}

/// `target/<profile>`, found from the test executable (`target/<profile>/deps/<name>`).
fn profile_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().to_path_buf()
}

fn find_library(dir: &Path) -> bool {
    ["libbreader.so", "libbreader.dylib"]
        .iter()
        .any(|n| dir.join(n).exists())
}

#[test]
fn c_example_builds_and_reads_a_qr() {
    if cfg!(windows) || !have("cc") {
        eprintln!("skipped: no C compiler (cc) or Windows");
        return;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let lib_dir = profile_dir();
    if !find_library(&lib_dir) {
        // `cargo test` builds the rlib only; build the shared library into the same profile.
        let mut cmd = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
        cmd.args(["build", "-p", "breader-ffi", "--manifest-path"])
            .arg(manifest.join("Cargo.toml"));
        if lib_dir.file_name().is_some_and(|n| n == "release") {
            cmd.arg("--release");
        }
        let st = cmd.status().expect("cargo build");
        assert!(st.success(), "cargo build -p breader-ffi failed");
    }
    assert!(find_library(&lib_dir), "no shared library in {lib_dir:?}");

    let work = std::env::temp_dir().join(format!("breader-ffi-c-{}", std::process::id()));
    std::fs::create_dir_all(&work).unwrap();
    let exe = work.join("scan");
    let image = work.join("qr.pgm");
    std::fs::write(&image, pgm(TEXT)).unwrap();

    let out = Command::new("cc")
        .args([
            "-std=c99",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-pedantic",
            "-O1",
        ])
        .arg("-I")
        .arg(manifest.join("include"))
        .arg(manifest.join("examples/c/scan.c"))
        .arg("-L")
        .arg(&lib_dir)
        .arg("-lbreader")
        .arg(format!("-Wl,-rpath,{}", lib_dir.display()))
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("cc");
    assert!(
        out.status.success(),
        "cc failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let run = Command::new(&exe).arg(&image).output().expect("run scan");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(stdout.contains("1 symbol(s)"), "{stdout}");
    assert!(stdout.contains(&format!("text: {TEXT}")), "{stdout}");
    assert!(stdout.contains("QR Code"), "{stdout}");

    // A file that is not a PGM fails cleanly with a non-zero status, no crash.
    let junk = work.join("junk.pgm");
    std::fs::write(&junk, b"hello").unwrap();
    let bad = Command::new(&exe).arg(&junk).output().expect("run scan");
    assert_eq!(bad.status.code(), Some(1));

    // The header is valid C++ too.
    if have("c++") {
        let hpp = work.join("h.cpp");
        std::fs::write(
            &hpp,
            "#include \"breader.h\"\nint main() { return breader_results_count(nullptr) != 0; }\n",
        )
        .unwrap();
        let st = Command::new("c++")
            .args([
                "-std=c++11",
                "-Wall",
                "-Wextra",
                "-Werror",
                "-fsyntax-only",
                "-I",
            ])
            .arg(manifest.join("include"))
            .arg(&hpp)
            .status()
            .unwrap();
        assert!(st.success(), "header does not compile as C++");
    }
    std::fs::remove_dir_all(&work).unwrap();
}
