//! Entry points for fuzz targets. Not part of the public API.

use breader_core::{
    Code39Options, EanAddonPolicy, ItfOptions, ScanOptions, Symbology, SymbologySet,
};

use crate::codabar::CodabarDecoder;
use crate::code39::{Code39Decoder, STAR_MASK};
use crate::code39_template;
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
    // Probes (empty text) only steer the refinement of the reader.
    out.extend(
        hits.into_iter()
            .filter(|h| !h.text.is_empty())
            .map(|h| (h.symbology, h.text)),
    );
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

/// Runs edge detection (both parameter sets), every decoder and, as the
/// reader does at Balanced and Exhaustive, the Code 39 template fallback
/// (D-058) over a gray-level profile.
pub fn scan_profile(profile: &[u8], seed: u16) -> Vec<FuzzHit> {
    scan_with(profile, &options(seed))
}

fn scan_with(profile: &[u8], opts: &LineOptions) -> Vec<FuzzHit> {
    let decoders = (
        EanDecoder,
        Code128Decoder,
        Code39Decoder,
        ItfDecoder,
        CodabarDecoder,
        DataBarDecoder,
        Code93Decoder,
    );
    let mut detector = EdgeDetector::default();
    let mut edges = Edges::default();
    let mut template = code39_template::Scratch::default();
    template.new_block();
    template.clear();
    let mut hits: Vec<LineHit> = Vec::new();
    let mut runs = Runs::default();
    let mut reversed = Runs::default();
    for params in [EdgeParams::fine(), EdgeParams::smoothed()] {
        detector.detect(profile, &params, &mut edges);
        Runs::from_edges(&edges, &mut runs);
        if runs.is_empty() {
            continue;
        }
        Runs::reverse_from(&runs, &mut reversed);
        if opts.normal {
            decoders.decode(&runs, opts, &mut hits);
            decoders.decode(&reversed, opts, &mut hits);
        }
        if opts.invert {
            runs.flip_roles();
            reversed.flip_roles();
            decoders.decode(&runs, opts, &mut hits);
            decoders.decode(&reversed, opts, &mut hits);
        }
        template.note(&runs, false, profile.len());
        template.note(&reversed, true, profile.len());
    }
    if !hits.iter().any(|h| !h.text.is_empty())
        && !opts.single_variant
        && opts.symbologies.contains(Symbology::Code39)
    {
        template.note_stretches(&runs, false, profile.len());
        template.note_stretches(&reversed, true, profile.len());
        hits.clear();
        code39_template::read_line(profile, opts, &mut template, &mut hits);
    }
    // Probes (empty text) only steer the refinement of the reader.
    hits.into_iter()
        .filter(|h| !h.text.is_empty())
        .map(|h| (h.symbology, h.text))
        .collect()
}

/// A thin Code 39 symbol drawn from `data`, then read the way a reader line is
/// (width decoders, then the template fallback). Layout: module in samples
/// (`b0` maps to 0.8..2.4), blur sigma (`b1`, 0.2..1.2), print gain (`b2`,
/// -0.5..0.5), offset (`b3`, 0..2), wide-to-narrow ratio (`b4`, 2.0..3.2),
/// noise amplitude (`b5`, 0..16 grey levels), flags (`b6`: bit 0 reversed,
/// bit 1 light on dark, bit 2 Full ASCII, bit 3 check character, bits 4 to 5
/// minimum length), the number of characters (`b7` % 16) and then that many
/// characters (each byte picks one of the 43 plain characters), the rest being
/// noise samples. Returns the text drawn (`None` when a read is not
/// comparable with it) and what was read.
pub fn thin_code39(data: &[u8]) -> (Option<Vec<u8>>, Vec<FuzzHit>) {
    let [m, sg, g, o, r, n, flags, count, rest @ ..] = data else {
        return (None, Vec::new());
    };
    let count = usize::from(count % 16).min(rest.len());
    let (body, noise) = rest.split_at(count);
    let unit = |b: u8| f32::from(b) / 255.0;
    let module = 0.8 + 1.6 * unit(*m);
    let sigma = 0.2 + unit(*sg);
    let gain = unit(*g) - 0.5;
    let offset = 2.0 * unit(*o);
    let ratio = 2.0 + 1.2 * unit(*r);
    let amp = 16.0 * unit(*n);
    let plain: Vec<u8> = code39_template::TEMPLATES
        .iter()
        .map(|&(c, _)| c)
        .filter(|&c| c != b'*')
        .collect();
    let text: Vec<u8> = body
        .iter()
        .map(|&b| plain[usize::from(b) % plain.len()])
        .collect();
    let mask_of = |c: u8| {
        code39_template::TEMPLATES
            .iter()
            .find(|(t, _)| *t == c)
            .map_or(0, |&(_, mask)| mask)
    };
    // Dark intervals, in samples.
    let mut dark: Vec<(f32, f32)> = Vec::new();
    let mut x = 12.0 * module + offset;
    for (i, c) in std::iter::once(b'*')
        .chain(text.iter().copied())
        .chain(std::iter::once(b'*'))
        .enumerate()
    {
        if i > 0 {
            x += module;
        }
        let mask = if c == b'*' { STAR_MASK } else { mask_of(c) };
        for e in 0..9 {
            let w = if mask >> e & 1 == 1 { ratio } else { 1.0 } * module;
            if e % 2 == 0 {
                dark.push((x - gain * module, x + w + gain * module));
            }
            x += w;
        }
    }
    let len = (x + 12.0 * module).ceil() as usize + 1;
    let (lo, hi) = (25.0f32, 225.0f32);
    let mut profile: Vec<u8> = (0..len)
        .map(|i| {
            let mut cover = 0.0f32;
            for k in 0..8 {
                let t = i as f32 - 0.5 + (k as f32 + 0.5) / 8.0;
                for &(a, b) in &dark {
                    if t < a - 5.0 * sigma || t > b + 5.0 * sigma {
                        continue;
                    }
                    let blur = |z: f32| 0.5 * (1.0 + libm_erf(z / std::f32::consts::SQRT_2));
                    cover += blur((t - a) / sigma) - blur((t - b) / sigma);
                }
            }
            (hi - (hi - lo) * cover / 8.0).round().clamp(0.0, 255.0) as u8
        })
        .collect();
    if !noise.is_empty() {
        for (i, v) in profile.iter_mut().enumerate() {
            let z = f32::from(noise[i % noise.len()] as i8) / 128.0;
            *v = (f32::from(*v) + z * amp).round().clamp(0.0, 255.0) as u8;
        }
    }
    if flags & 2 != 0 {
        for v in &mut profile {
            *v = 255 - *v;
        }
    }
    if flags & 1 != 0 {
        profile.reverse();
    }
    let mut opts = options(0);
    opts.symbologies = SymbologySet::EMPTY.with(Symbology::Code39);
    opts.normal = true;
    opts.invert = true;
    opts.single_variant = false;
    opts.code39_full_ascii = flags & 4 != 0;
    opts.code39_checksum = flags & 8 != 0;
    opts.code39_min_length = usize::from(flags >> 4 & 3);
    let comparable = !opts.code39_full_ascii
        && !opts.code39_checksum
        && amp <= 2.0
        && sigma <= 0.7
        && module >= 1.1
        && gain.abs() <= 0.3
        && !text.is_empty();
    (comparable.then_some(text), scan_with(&profile, &opts))
}

/// Error function (Abramowitz and Stegun 7.1.26), accurate to 1.5e-7.
fn libm_erf(x: f32) -> f32 {
    let t = 1.0 / (1.0 + 0.327_591_1 * x.abs());
    let y = 1.0
        - (((((1.061_405_4 * t - 1.453_152_1) * t) + 1.421_413_7) * t - 0.284_496_74) * t
            + 0.254_829_6)
            * t
            * (-x * x).exp();
    y.copysign(x)
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

    /// The fuzz entry draws thin symbols that the width decoders miss and the
    /// template fallback reads, so the target reaches `code39_template`.
    #[test]
    fn thin_code39_reaches_the_template_fallback() {
        let mut reads = 0;
        for o in 0..=255u8 {
            let mut data = vec![88, 64, 51, o, 106, 0, 0, 7];
            data.extend((0..7u8).map(|k| o.wrapping_mul(7).wrapping_add(k * 5)));
            data.extend((0..64u8).map(|k| k.wrapping_mul(37) ^ o));
            let (expected, hits) = thin_code39(&data);
            for (_, text) in &hits {
                assert_eq!(Some(text), expected.as_ref());
                reads += 1;
            }
        }
        assert!(reads > 20, "{reads}");
    }

    /// A fuzz input that makes the decoders leave a probe (empty text): the
    /// entry must hand out symbols only.
    #[test]
    fn decode_runs_hands_out_no_probes() {
        let data: [u8; 148] = [
            0xff, 0xff, 0xa8, 0xfe, 0xef, 0x00, 0x18, 0x00, 0x24, 0x24, 0x24, 0x24, 0x24, 0x26,
            0x3d, 0x24, 0x24, 0x24, 0x26, 0x24, 0x64, 0x24, 0x20, 0x24, 0xdc, 0x24, 0x6b, 0x24,
            0x7b, 0x24, 0x8e, 0x24, 0x26, 0x24, 0x64, 0x24, 0x20, 0x24, 0x24, 0x24, 0x24, 0x10,
            0x68, 0x68, 0x68, 0x68, 0x68, 0x68, 0x68, 0x00, 0x5a, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x68, 0x68, 0x68, 0x68, 0x68, 0x68, 0x24, 0x20, 0x24, 0xdc, 0x24, 0x24,
            0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x46, 0x46, 0x46,
            0xff, 0x40, 0xef, 0x1b, 0x20, 0x24, 0x24, 0x24, 0x24, 0x24, 0x26, 0x24, 0x64, 0x24,
            0x20, 0x24, 0xdc, 0x00, 0x00, 0x00, 0x00, 0x68, 0x68, 0x68, 0x68, 0x68, 0x68, 0x68,
            0x68, 0x68, 0x68, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x03, 0x24, 0x24, 0x24,
            0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x18, 0x00,
            0x10, 0x00, 0x00, 0x10, 0x00, 0x10, 0xff, 0x24,
        ];
        let seed = u16::from_le_bytes([data[0], data[1]]);
        let widths: Vec<u16> = data[2..]
            .chunks(2)
            .map(|c| u16::from(c[0]) | u16::from(c.get(1).copied().unwrap_or(0) & 0x03) << 8)
            .collect();
        for (_, text) in decode_runs(&widths, seed) {
            assert!(!text.is_empty());
        }
    }

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
