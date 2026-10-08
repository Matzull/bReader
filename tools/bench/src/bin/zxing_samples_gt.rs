//! Writes `annotations.json` into every directory of a zxing-cpp `test/samples` checkout.
//!
//! Usage: zxing-samples-gt <samples-dir>
//!
//! An image is labeled from `<stem>.txt` or `<stem>.bin`; `falsepositives*` directories get an
//! empty label list. Images without an expected value (multi-symbol and must-fail cases whose
//! expectations live in zxing-cpp's test code) and parts of structured-append symbols are left
//! out, as are files the `image` crate cannot decode.

use std::path::Path;
use std::process::ExitCode;

use benchlib::groundtruth::map_symbology;
use benchlib::zxing_samples::{
    IMAGE_EXTENSIONS, Label, barcode, dir_format, is_negative_dir, read_label, result_format,
    strip_addon,
};
use serde_json::{Value, json};

fn is_decodable(path: &Path) -> Result<(), String> {
    let reader = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| e.to_string())?;
    reader.decode().map(|_| ()).map_err(|e| e.to_string())
}

fn convert_dir(dir: &Path) -> Result<(usize, usize, usize), String> {
    let name = dir.file_name().unwrap_or_default().to_string_lossy();
    let dir_fmt = dir_format(&name);
    let negative = is_negative_dir(&name);
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        })
        .collect();
    files.sort();

    let (mut supported, mut unsupported) = (0, 0);
    let mut images: Vec<Value> = Vec::new();
    for path in files {
        let file = path.file_name().unwrap_or_default().to_string_lossy();
        let stem = path.file_stem().unwrap_or_default().to_string_lossy();
        if let Err(e) = is_decodable(&path) {
            println!("skip {name}/{file}: not loadable ({e})");
            continue;
        }
        let barcodes = if negative {
            Vec::new()
        } else {
            match read_label(dir, &stem) {
                Label::Expected(data) => {
                    let format = std::fs::read_to_string(dir.join(format!("{stem}.result.txt")))
                        .ok()
                        .and_then(|t| result_format(&t))
                        .unwrap_or_else(|| dir_fmt.clone());
                    if map_symbology(&format).is_some() {
                        supported += 1;
                    } else {
                        unsupported += 1;
                    }
                    let data = if name.contains("extension") {
                        strip_addon(&format, &data)
                    } else {
                        data
                    };
                    vec![barcode(&format, &data)]
                }
                Label::StructuredAppendPart => {
                    println!(
                        "skip {name}/{file}: structured-append part, only the merged text is known"
                    );
                    continue;
                }
                Label::Missing => {
                    println!("skip {name}/{file}: no expected value");
                    continue;
                }
            }
        };
        images.push(json!({ "file": file, "barcodes": barcodes }));
    }
    let total: usize = images
        .iter()
        .map(|i| i["barcodes"].as_array().map_or(0, Vec::len))
        .sum();
    let doc = json!({
        "format": "barcode-benchmark/1.0",
        "dataset": format!("zxing-cpp test/samples/{name}"),
        "total_images": images.len(),
        "total_barcodes": total,
        "images": images,
    });
    let text = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("annotations.json"), text).map_err(|e| e.to_string())?;
    Ok((images.len(), supported, unsupported))
}

fn main() -> ExitCode {
    let Some(root) = std::env::args().nth(1) else {
        eprintln!("usage: zxing-samples-gt <samples-dir>");
        return ExitCode::from(2);
    };
    let Ok(entries) = std::fs::read_dir(&root) else {
        eprintln!("cannot read {root}");
        return ExitCode::FAILURE;
    };
    let mut dirs: Vec<_> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    let (mut n_dirs, mut n_images, mut n_sup, mut n_unsup) = (0, 0, 0, 0);
    for dir in &dirs {
        match convert_dir(dir) {
            Ok((images, sup, unsup)) => {
                n_dirs += 1;
                n_images += images;
                n_sup += sup;
                n_unsup += unsup;
            }
            Err(e) => {
                eprintln!("{}: {e}", dir.display());
                return ExitCode::FAILURE;
            }
        }
    }
    println!(
        "{n_dirs} directories, {n_images} images, {n_sup} supported and {n_unsup} unsupported labels"
    );
    ExitCode::SUCCESS
}
