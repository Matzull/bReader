use breader::*;
use std::time::Instant;

fn files(dir: &str) -> Vec<std::path::PathBuf> {
    let mut v = Vec::new();
    let mut stack = vec![std::path::PathBuf::from(dir)];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p)
            } else if matches!(
                p.extension()
                    .and_then(|x| x.to_str())
                    .map(|s| s.to_lowercase())
                    .as_deref(),
                Some("png" | "jpg" | "jpeg")
            ) {
                v.push(p)
            }
        }
    }
    v.sort();
    v
}

/// Best wall time in seconds over `RUNS` repetitions; the minimum is the least noisy estimate.
fn best_of(mut f: impl FnMut()) -> f64 {
    const RUNS: usize = 5;
    (0..RUNS)
        .map(|_| {
            let t = Instant::now();
            f();
            t.elapsed().as_secs_f64()
        })
        .fold(f64::INFINITY, f64::min)
}

/// Per-stage timing: whole-pipeline cases per effort and symbology family,
/// then the image kernels alone. Usage: stages <dataset-dir>
fn main() {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: stages <dataset-dir>");
        std::process::exit(2);
    };
    let imgs: Vec<_> = files(&dir)
        .into_iter()
        .filter_map(|p| image::open(&p).ok())
        .map(|i| i.into_luma8())
        .collect();
    let px: f64 = imgs
        .iter()
        .map(|i| (i.width() * i.height()) as f64)
        .sum::<f64>()
        / imgs.len() as f64;
    println!("{} images, mean {:.1} MP", imgs.len(), px / 1e6);
    let only = |set: &[Symbology]| set.iter().copied().collect::<SymbologySet>();
    let linear = only(&[
        Symbology::Ean13,
        Symbology::Ean8,
        Symbology::UpcA,
        Symbology::UpcE,
        Symbology::Code128,
        Symbology::Code39,
        Symbology::Itf,
    ]);
    let qr = only(&[Symbology::Qr]);
    let cases: Vec<(&str, ScanOptions)> = vec![
        ("balanced all", ScanOptions::default()),
        (
            "balanced linear",
            ScanOptions {
                symbologies: linear,
                ..Default::default()
            },
        ),
        (
            "balanced qr",
            ScanOptions {
                symbologies: qr,
                ..Default::default()
            },
        ),
        ("fast all", ScanOptions::with_effort(Effort::Fast)),
        (
            "fast linear",
            ScanOptions {
                symbologies: linear,
                ..ScanOptions::with_effort(Effort::Fast)
            },
        ),
        (
            "fast qr",
            ScanOptions {
                symbologies: qr,
                ..ScanOptions::with_effort(Effort::Fast)
            },
        ),
    ];
    for (name, opts) in cases {
        let mut r = Reader::with_options(opts);
        let best = best_of(|| {
            for i in &imgs {
                let v = ImageView::from_packed(i.as_raw(), i.width() as usize, i.height() as usize)
                    .unwrap();
                std::hint::black_box(r.scan(v));
            }
        });
        println!("{name:16} {:7.2} ms/img", best * 1e3 / imgs.len() as f64);
    }
    let mut out = BitMatrix::default();
    let mut buf = ImageBuffer::default();
    for (name, f) in [
        ("local binarize", 0),
        ("otsu binarize", 1),
        ("downscale x2", 2),
    ] {
        let best = best_of(|| {
            for i in &imgs {
                let v = ImageView::from_packed(i.as_raw(), i.width() as usize, i.height() as usize)
                    .unwrap();
                match f {
                    0 => breader::modules::LocalBinarizer::default().binarize(v, &mut out),
                    1 => breader::modules::OtsuBinarizer.binarize(v, &mut out),
                    _ => breader::modules::Downscale::new(2).apply(v, &mut buf),
                }
            }
        });
        println!("{name:16} {:7.2} ms/img", best * 1e3 / imgs.len() as f64);
    }
}
