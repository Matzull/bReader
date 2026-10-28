//! Command line parsing, by hand: the grammar is small and fixed.

use std::time::Duration;

use breader::{EanAddonPolicy, Effort, ScanOptions, Symbology, SymbologySet};

pub const MAIN_HELP: &str = "\
breader: read barcodes and QR codes from images

Usage:
  breader scan [OPTIONS] <PATH>...
  breader --version
  breader --help

Commands:
  scan    Read the symbols in image files (PNG, JPEG), directories or stdin

Run `breader scan --help` for the options.
";

pub const SCAN_HELP: &str = "\
Read the symbols in images

Usage:
  breader scan [OPTIONS] <PATH>...

Each PATH is an image file (PNG or JPEG), a directory (searched recursively for
.png, .jpg and .jpeg files, in sorted order) or `-` for an image read from stdin.

Options:
      --effort <LEVEL>         fast, balanced (default) or exhaustive
      --symbologies <LIST>     Comma separated: qr, ean13, ean8, upca, upce, code128,
                               code39, itf, datamatrix, pdf417, aztec, codabar,
                               micro-qr, rmqr, databar, databar-limited,
                               databar-expanded, code93 (default: all)
      --try-invert             Also look for light-on-dark symbols (default: by effort)
      --max-symbols <N>        Stop after N symbols per image (default: unlimited)
      --time-budget-ms <N>     Time budget per image, checked between passes
      --ean-addon <POLICY>     ignore (default), optional or required
      --code39-full-ascii      Decode Code 39 Full ASCII
      --code39-checksum        Require a valid Code 39 mod-43 check character
      --format <FORMAT>        text (default) or json (JSON Lines)
  -j, --jobs <N>               Images scanned in parallel; 0 = one per CPU (default: 1).
                               Output order is always the input order
  -h, --help                   Print this help

Output:
  text   One line per symbol: path, symbology and payload, separated by tabs.
         Backslash, tab, newline and other control characters in the payload are
         written as \\\\, \\t, \\n, \\r and \\xNN; a payload that is not UTF-8 is
         written as hex: followed by its bytes in hex.
  json   One object per symbol and line with the keys path, symbology, text (null
         unless UTF-8), data_hex, content, gs1_hri, corners, orientation, aim_id and
         quality.

Exit status: 0 if a symbol was found, 1 if none was, 2 on a usage error or when
an input could not be read (the other inputs are still scanned).
";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Text,
    Json,
}

#[derive(Debug)]
pub struct ScanArgs {
    pub paths: Vec<String>,
    pub options: ScanOptions,
    pub format: Format,
    pub jobs: usize,
}

#[derive(Debug)]
pub enum Command {
    MainHelp,
    ScanHelp,
    Version,
    Scan(ScanArgs),
}

/// The id shown to users, as in the Python bindings.
pub fn symbology_id(s: Symbology) -> &'static str {
    match s {
        Symbology::Qr => "qr",
        Symbology::Ean13 => "ean13",
        Symbology::Ean8 => "ean8",
        Symbology::UpcA => "upca",
        Symbology::UpcE => "upce",
        Symbology::Code128 => "code128",
        Symbology::Code39 => "code39",
        Symbology::Itf => "itf",
        Symbology::DataMatrix => "datamatrix",
        Symbology::Pdf417 => "pdf417",
        Symbology::Aztec => "aztec",
        Symbology::Codabar => "codabar",
        Symbology::MicroQr => "micro-qr",
        Symbology::Rmqr => "rmqr",
        Symbology::DataBar => "databar",
        Symbology::DataBarLimited => "databar-limited",
        Symbology::DataBarExpanded => "databar-expanded",
        Symbology::Code93 => "code93",
        _ => s.name(),
    }
}

fn parse_symbology(name: &str) -> Result<Symbology, String> {
    let key = name.to_ascii_lowercase().replace(['-', '_', ' '], "");
    Symbology::ALL
        .iter()
        .copied()
        .find(|s| {
            symbology_id(*s).replace('-', "") == key || (key == "qrcode" && *s == Symbology::Qr)
        })
        .ok_or_else(|| {
            let known: Vec<_> = Symbology::ALL.iter().map(|s| symbology_id(*s)).collect();
            format!(
                "unknown symbology {name:?}, expected one of {}",
                known.join(", ")
            )
        })
}

fn parse_symbologies(list: &str) -> Result<SymbologySet, String> {
    let mut set = SymbologySet::EMPTY;
    for name in list.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        set = set.with(parse_symbology(name)?);
    }
    if set.is_empty() {
        return Err("--symbologies needs at least one name".into());
    }
    Ok(set)
}

fn parse_effort(name: &str) -> Result<Effort, String> {
    match name {
        "fast" => Ok(Effort::Fast),
        "balanced" => Ok(Effort::Balanced),
        "exhaustive" => Ok(Effort::Exhaustive),
        _ => Err(format!(
            "unknown effort {name:?}, expected fast, balanced or exhaustive"
        )),
    }
}

fn parse_addon(name: &str) -> Result<EanAddonPolicy, String> {
    match name {
        "ignore" => Ok(EanAddonPolicy::Ignore),
        "optional" => Ok(EanAddonPolicy::Optional),
        "required" => Ok(EanAddonPolicy::Required),
        _ => Err(format!(
            "unknown ean-addon {name:?}, expected ignore, optional or required"
        )),
    }
}

fn parse_format(name: &str) -> Result<Format, String> {
    match name {
        "text" => Ok(Format::Text),
        "json" => Ok(Format::Json),
        _ => Err(format!("unknown format {name:?}, expected text or json")),
    }
}

fn parse_count(option: &str, value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("{option} needs a non-negative integer, got {value:?}"))
}

/// Parses the arguments after the program name.
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        None => Err("missing command, expected `scan`".into()),
        Some("-h" | "--help" | "help") => Ok(Command::MainHelp),
        Some("-V" | "--version") => Ok(Command::Version),
        Some("scan") => parse_scan(args),
        Some(other) => Err(format!("unknown command {other:?}, expected `scan`")),
    }
}

fn parse_scan(args: impl Iterator<Item = String>) -> Result<Command, String> {
    let mut args = args;
    let mut options = ScanOptions::default();
    let mut format = Format::Text;
    let mut jobs = 1;
    let mut paths = Vec::new();
    let mut only_paths = false;
    while let Some(arg) = args.next() {
        if only_paths || arg == "-" || !arg.starts_with('-') {
            paths.push(arg);
            continue;
        }
        if arg == "--" {
            only_paths = true;
            continue;
        }
        // `--name=value` and `--name value` are the same.
        let (name, inline) = match arg.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match name.as_str() {
            "-h" | "--help" => return Ok(Command::ScanHelp),
            "--effort" => options.effort = parse_effort(&value(&name)?)?,
            "--symbologies" => options.symbologies = parse_symbologies(&value(&name)?)?,
            "--max-symbols" => options.max_symbols = parse_count(&name, &value(&name)?)?,
            "--time-budget-ms" => {
                let ms = parse_count(&name, &value(&name)?)?;
                options.time_budget = Some(Duration::from_millis(ms as u64));
            }
            "--ean-addon" => options.ean_addon = parse_addon(&value(&name)?)?,
            "--format" => format = parse_format(&value(&name)?)?,
            "-j" | "--jobs" => {
                jobs = parse_count(&name, &value(&name)?)?;
                if jobs == 0 {
                    jobs = std::thread::available_parallelism().map_or(1, |n| n.get());
                }
            }
            "--try-invert" | "--code39-full-ascii" | "--code39-checksum" => {
                if inline.is_some() {
                    return Err(format!("{name} takes no value"));
                }
                match name.as_str() {
                    "--try-invert" => options.try_invert = Some(true),
                    "--code39-full-ascii" => options.code39.full_ascii = true,
                    _ => options.code39.require_checksum = true,
                }
            }
            _ => return Err(format!("unknown option {arg:?}")),
        }
    }
    if paths.is_empty() {
        return Err("scan needs at least one PATH (an image, a directory or `-`)".into());
    }
    Ok(Command::Scan(ScanArgs {
        paths,
        options,
        format,
        jobs,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(|a| a.to_string()))
    }

    fn scan(args: &[&str]) -> ScanArgs {
        match run(args) {
            Ok(Command::Scan(s)) => s,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn options_map_to_scan_options() {
        let s = scan(&[
            "scan",
            "--effort",
            "exhaustive",
            "--symbologies=qr, EAN-13,micro_qr",
            "--try-invert",
            "--max-symbols",
            "3",
            "--time-budget-ms=250",
            "--ean-addon",
            "optional",
            "--code39-full-ascii",
            "--code39-checksum",
            "--format",
            "json",
            "-j",
            "4",
            "a.png",
            "-",
        ]);
        let o = &s.options;
        assert_eq!(o.effort, Effort::Exhaustive);
        assert_eq!(o.try_invert, Some(true));
        assert_eq!(o.max_symbols, 3);
        assert_eq!(o.time_budget, Some(Duration::from_millis(250)));
        assert_eq!(o.ean_addon, EanAddonPolicy::Optional);
        assert!(o.code39.full_ascii && o.code39.require_checksum);
        let set: Vec<_> = o.symbologies.iter().collect();
        assert_eq!(set, [Symbology::Qr, Symbology::Ean13, Symbology::MicroQr]);
        assert_eq!((s.format, s.jobs), (Format::Json, 4));
        assert_eq!(s.paths, ["a.png", "-"]);
    }

    #[test]
    fn defaults_leave_the_options_untouched() {
        let s = scan(&["scan", "x.png"]);
        assert_eq!(s.options, ScanOptions::default());
        assert_eq!((s.format, s.jobs), (Format::Text, 1));
    }

    #[test]
    fn every_symbology_id_parses_back() {
        for s in Symbology::ALL {
            assert_eq!(parse_symbology(symbology_id(*s)), Ok(*s));
        }
    }

    #[test]
    fn jobs_zero_means_every_cpu() {
        assert!(scan(&["scan", "-j", "0", "x"]).jobs >= 1);
    }

    #[test]
    fn double_dash_ends_the_options() {
        assert_eq!(scan(&["scan", "--", "-odd.png"]).paths, ["-odd.png"]);
    }

    #[test]
    fn help_and_version() {
        assert!(matches!(run(&["--help"]), Ok(Command::MainHelp)));
        assert!(matches!(run(&["--version"]), Ok(Command::Version)));
        assert!(matches!(run(&["scan", "--help"]), Ok(Command::ScanHelp)));
        assert!(matches!(run(&["scan", "x", "-h"]), Ok(Command::ScanHelp)));
    }

    #[test]
    fn usage_errors() {
        for bad in [
            &[][..],
            &["frobnicate"],
            &["scan"],
            &["scan", "--effort", "slow", "x"],
            &["scan", "--effort"],
            &["scan", "--symbologies", "qr,nope", "x"],
            &["scan", "--symbologies", ",", "x"],
            &["scan", "--max-symbols", "-1", "x"],
            &["scan", "--format", "xml", "x"],
            &["scan", "--try-invert=1", "x"],
            &["scan", "--bogus", "x"],
        ] {
            assert!(run(bad).is_err(), "{bad:?}");
        }
    }
}
