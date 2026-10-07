//! Entry points for fuzz targets. Not part of the public API.

use breader_core::{
    Code39Options, EanAddonPolicy, ItfOptions, ScanOptions, Symbology, SymbologySet,
};

use crate::code39::Code39Decoder;
use crate::code128::Code128Decoder;
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::ean::EanDecoder;
use crate::edges::{EdgeDetector, EdgeParams, Edges};
use crate::itf::ItfDecoder;
use crate::runs::Runs;

/// A symbol decoded from a fuzz input.
pub type FuzzHit = (Symbology, Vec<u8>);

fn options(seed: u16) -> LineOptions {
    let mut set = SymbologySet::EMPTY;
    for (i, s) in Symbology::ALL.iter().enumerate() {
        if seed >> i & 1 == 1 {
            set = set.with(*s);
        }
    }
    if set.is_empty() {
        set = SymbologySet::all();
    }
    let opts = ScanOptions {
        symbologies: set,
        ean_addon: match seed >> 8 & 3 {
            0 => EanAddonPolicy::Ignore,
            1 => EanAddonPolicy::Optional,
            _ => EanAddonPolicy::Required,
        },
        code39: Code39Options {
            full_ascii: seed >> 10 & 1 == 1,
            require_checksum: seed >> 11 & 1 == 1,
            min_length: usize::from(seed >> 12 & 3),
        },
        itf: ItfOptions {
            allowed_lengths: if seed >> 14 & 1 == 1 {
                vec![6, 14]
            } else {
                Vec::new()
            },
            min_length: 2,
        },
        try_invert: Some(seed >> 15 & 1 == 1),
        ..ScanOptions::default()
    };
    LineOptions::from(&opts)
}

fn decode_all(runs: &mut Runs, reversed: &mut Runs, opts: &LineOptions, out: &mut Vec<FuzzHit>) {
    let decoders = (EanDecoder, Code128Decoder, Code39Decoder, ItfDecoder);
    Runs::reverse_from(runs, reversed);
    let mut hits: Vec<LineHit> = Vec::new();
    decoders.decode(runs, opts, &mut hits);
    decoders.decode(reversed, opts, &mut hits);
    if opts.invert {
        runs.flip_roles();
        reversed.flip_roles();
        decoders.decode(runs, opts, &mut hits);
        decoders.decode(reversed, opts, &mut hits);
    }
    out.extend(hits.into_iter().map(|h| (h.symbology, h.text)));
}

/// Runs every decoder over a run sequence, in both directions and
/// polarities. `widths` are in eighths of a sample; `seed` selects options.
pub fn decode_runs(widths: &[u16], seed: u16) -> Vec<FuzzHit> {
    let w: Vec<f32> = widths.iter().map(|&v| f32::from(v) / 8.0).collect();
    let mut runs = Runs::from_widths(seed & 1 == 1, &w, 10.0, 10.0);
    let mut reversed = Runs::default();
    let mut out = Vec::new();
    decode_all(&mut runs, &mut reversed, &options(seed >> 1), &mut out);
    out
}

/// Runs edge detection (both parameter sets) and every decoder over a
/// gray-level profile.
pub fn scan_profile(profile: &[u8], seed: u16) -> Vec<FuzzHit> {
    let opts = options(seed);
    let mut detector = EdgeDetector::default();
    let mut edges = Edges::default();
    let mut out = Vec::new();
    for params in [EdgeParams::fine(), EdgeParams::smoothed()] {
        detector.detect(profile, &params, &mut edges);
        let mut runs = Runs::default();
        Runs::from_edges(&edges, &mut runs);
        if runs.is_empty() {
            continue;
        }
        let mut reversed = Runs::default();
        decode_all(&mut runs, &mut reversed, &opts, &mut out);
    }
    out
}
