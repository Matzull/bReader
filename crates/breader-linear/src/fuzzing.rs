//! Entry points for fuzz targets. Not part of the public API.

use breader_core::{
    Code39Options, EanAddonPolicy, ItfOptions, ScanOptions, Symbology, SymbologySet,
};

use crate::codabar::CodabarDecoder;
use crate::code39::Code39Decoder;
use crate::code93::Code93Decoder;
use crate::code128::Code128Decoder;
use crate::databar::DataBarDecoder;
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::ean::EanDecoder;
use crate::edges::{EdgeDetector, EdgeParams, Edges};
use crate::itf::ItfDecoder;
use crate::runs::Runs;

/// A symbol decoded from a fuzz input.
pub type FuzzHit = (Symbology, Vec<u8>);

/// Bit `i % 16` of `seed` selects symbology `i`: there are more symbologies than bits.
fn options(seed: u16) -> LineOptions {
    let mut set = SymbologySet::EMPTY;
    for (i, s) in Symbology::ALL.iter().enumerate() {
        if seed >> (i % 16) & 1 == 1 {
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
    let decoders = (
        EanDecoder,
        Code128Decoder,
        Code39Decoder,
        ItfDecoder,
        CodabarDecoder,
        DataBarDecoder,
        Code93Decoder,
    );
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

/// Code 93 only, on arbitrary run widths (in eighths of a sample): both
/// directions and polarities. `seed` bit 0 sets the polarity of the first run.
pub fn decode_code93_runs(widths: &[u16], seed: u16) -> Vec<FuzzHit> {
    let w: Vec<f32> = widths.iter().map(|&v| f32::from(v) / 8.0).collect();
    let mut runs = Runs::from_widths(seed & 1 == 1, &w, 10.0, 10.0);
    let mut reversed = Runs::default();
    let mut opts = options(seed >> 1);
    opts.symbologies = SymbologySet::EMPTY.with(Symbology::Code93);
    let mut hits: Vec<LineHit> = Vec::new();
    Runs::reverse_from(&runs, &mut reversed);
    Code93Decoder.decode(&runs, &opts, &mut hits);
    Code93Decoder.decode(&reversed, &opts, &mut hits);
    runs.flip_roles();
    reversed.flip_roles();
    Code93Decoder.decode(&runs, &opts, &mut hits);
    Code93Decoder.decode(&reversed, &opts, &mut hits);
    hits.into_iter().map(|h| (h.symbology, h.text)).collect()
}

/// A well-formed Code 93 symbol around arbitrary data: every byte of `data`
/// (at most 128) becomes a character value 0 to 46, the check characters are
/// computed, and the symbol is drawn with modules of `seed % 24 + 8` eighths
/// of a sample. Returns the text the symbol stands for (`None` when it holds
/// an invalid shift sequence or fewer than two characters) and what the
/// decoder read; both must agree.
pub fn code93_symbol(data: &[u8], seed: u16) -> (Option<Vec<u8>>, Vec<FuzzHit>) {
    let values: Vec<u8> = data.iter().take(128).map(|b| b % 47).collect();
    let (expected, widths) = crate::code93::symbol_runs(&values);
    let module = f32::from(seed % 24 + 8) / 8.0;
    let w: Vec<f32> = widths.iter().map(|&k| f32::from(k) * module).collect();
    let runs = Runs::from_widths(true, &w, 12.0 * module, 12.0 * module);
    let mut opts = options(0);
    opts.symbologies = SymbologySet::EMPTY.with(Symbology::Code93);
    let mut hits: Vec<LineHit> = Vec::new();
    Code93Decoder.decode(&runs, &opts, &mut hits);
    (
        expected,
        hits.into_iter().map(|h| (h.symbology, h.text)).collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Rng;

    #[test]
    fn code93_entry_points_agree_with_the_decoder() {
        let mut rng = Rng(0xc093);
        let (mut read, mut none) = (0, 0);
        for i in 0..2000u32 {
            let n = rng.below(40) as usize;
            let data: Vec<u8> = (0..n).map(|_| rng.below(256) as u8).collect();
            let (expected, hits) = code93_symbol(&data, i as u16);
            match expected {
                Some(text) => {
                    assert_eq!(hits.iter().map(|h| &h.1).collect::<Vec<_>>(), [&text]);
                    read += 1;
                }
                None => {
                    assert!(hits.is_empty());
                    none += 1;
                }
            }
        }
        assert!(read > 300 && none > 300, "{read} {none}");
        for i in 0..500u32 {
            let widths: Vec<u16> = (0..rng.below(300))
                .map(|_| rng.below(1024) as u16)
                .collect();
            for (symbology, text) in decode_code93_runs(&widths, i as u16) {
                assert_eq!(symbology, Symbology::Code93);
                assert!(!text.is_empty());
            }
        }
    }
}
