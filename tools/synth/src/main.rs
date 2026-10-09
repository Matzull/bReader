mod encode;
mod negative;
mod render;
mod scene;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde::Serialize;

use scene::{Annotation, Class};

const USAGE: &str = "usage: synth <out-dir> [--count N per class] [--seed S]";

#[derive(Debug)]
struct Args {
    out: PathBuf,
    count: usize,
    seed: u64,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut out = None;
    let mut count = 20;
    let mut seed = 1;
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--count" => {
                count = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--count needs a number")?;
            }
            "--seed" => {
                seed = args
                    .next()
                    .and_then(|v| v.parse().ok())
                    .ok_or("--seed needs a number")?;
            }
            s if s.starts_with("--") => return Err(format!("unknown option {s}")),
            _ if out.is_none() => out = Some(PathBuf::from(arg)),
            _ => return Err("multiple output directories given".into()),
        }
    }
    Ok(Args {
        out: out.ok_or("missing <out-dir>")?,
        count,
        seed,
    })
}

#[derive(Serialize)]
struct Barcode {
    text: String,
    format: String,
    points: [[f64; 2]; 4],
}

#[derive(Serialize)]
struct ImageEntry {
    file: String,
    barcodes: Vec<Barcode>,
}

#[derive(Serialize)]
struct Annotations {
    format: &'static str,
    dataset: String,
    total_images: usize,
    total_barcodes: usize,
    images: Vec<ImageEntry>,
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn to_barcode(a: &Annotation) -> Barcode {
    Barcode {
        text: a.text.clone(),
        format: a.format.to_string(),
        points: a.points.map(|(x, y)| [round1(x), round1(y)]),
    }
}

fn generate_class(
    class: Class,
    out: &Path,
    count: usize,
    seed: u64,
) -> Result<(usize, usize), String> {
    let dir = out.join(class.name());
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut images = Vec::new();
    let extra = if class == Class::Datamatrix {
        0
    } else {
        count.div_ceil(9)
    };
    let pdf417 = if matches!(class, Class::Datamatrix | Class::Negative) {
        0
    } else {
        count.div_ceil(9)
    };
    let aztec = pdf417;
    for i in 0..count + extra + pdf417 + aztec {
        let (img, annotations) = if i < count {
            scene::generate(class, i, seed)
        } else if i < count + extra {
            match scene::generate_extra(class, i - count, seed) {
                Some(g) => g,
                None => break,
            }
        } else if i < count + extra + pdf417 {
            match scene::generate_pdf417(class, i - count - extra, seed) {
                Some(g) => g,
                None => break,
            }
        } else {
            match scene::generate_aztec(class, i - count - extra - pdf417, seed) {
                Some(g) => g,
                None => break,
            }
        };
        let file = format!("{}_{:04}.png", class.name().replace('-', "_"), i + 1);
        img.save(dir.join(&file))
            .map_err(|e| format!("{file}: {e}"))?;
        images.push(ImageEntry {
            file,
            barcodes: annotations.iter().map(to_barcode).collect(),
        });
    }
    let total_barcodes = images.iter().map(|e| e.barcodes.len()).sum();
    let ann = Annotations {
        format: "barcode-benchmark/1.0",
        dataset: format!("breader synthetic / {}", class.name()),
        total_images: images.len(),
        total_barcodes,
        images,
    };
    let json = serde_json::to_string_pretty(&ann).map_err(|e| e.to_string())?;
    let path = dir.join("annotations.json");
    std::fs::write(&path, json).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok((ann.total_images, total_barcodes))
}

fn main() -> ExitCode {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    let (mut images, mut codes) = (0, 0);
    for class in Class::ALL {
        match generate_class(class, &args.out, args.count, args.seed) {
            Ok((i, c)) => {
                println!("{:<13} images {i:>4}  barcodes {c:>4}", class.name());
                images += i;
                codes += c;
            }
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    println!("{:<13} images {images:>4}  barcodes {codes:>4}", "total");
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_args() {
        let a = parse_args(
            ["out", "--count", "3", "--seed", "9"]
                .map(String::from)
                .into_iter(),
        )
        .unwrap();
        assert_eq!((a.count, a.seed), (3, 9));
        assert!(parse_args(["--count", "x"].map(String::from).into_iter()).is_err());
        assert!(parse_args(std::iter::empty()).is_err());
    }

    #[test]
    fn generation_is_deterministic_and_annotated() {
        for class in Class::ALL {
            let (a, ann_a) = scene::generate(class, 2, 5);
            let (b, ann_b) = scene::generate(class, 2, 5);
            assert_eq!(a.as_raw(), b.as_raw(), "{}", class.name());
            assert_eq!(ann_a.len(), ann_b.len());
            assert!(
                (640..=1600).contains(&a.width()),
                "{} {}",
                class.name(),
                a.width()
            );
            match class {
                Class::Negative => assert!(ann_a.is_empty()),
                Class::Multi => assert!((2..=5).contains(&ann_a.len())),
                _ => assert_eq!(ann_a.len(), 1),
            }
            for ann in &ann_a {
                for (x, y) in ann.points {
                    assert!(
                        x >= 0.0
                            && y >= 0.0
                            && x <= f64::from(a.width())
                            && y <= f64::from(a.height())
                    );
                }
            }
        }
    }
}
