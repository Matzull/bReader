//! Pseudo-random input must never panic and never produce a symbol.

use breader_core::{
    Binarizer, Effort, FrameContext, ImageBuffer, Pass, Preprocessor, ScanOptions, Scratch, Symbol,
    SymbologyReader,
};

use crate::codabar::CodabarDecoder;
use crate::code39::Code39Decoder;
use crate::code128::Code128Decoder;
use crate::databar::DataBarDecoder;
use crate::decoder::{LineOptions, LinearDecoder};
use crate::ean::EanDecoder;
use crate::edges::{EdgeDetector, EdgeParams, Edges};
use crate::itf::ItfDecoder;
use crate::reader::LinearReader;
use crate::runs::Runs;
use crate::testutil::Rng;

fn all_params() -> [EdgeParams; 4] {
    [
        EdgeParams::fine(),
        EdgeParams::smoothed(),
        EdgeParams::fine().raw(),
        EdgeParams::smoothed().raw(),
    ]
}

const ALL: (
    EanDecoder,
    Code128Decoder,
    Code39Decoder,
    ItfDecoder,
    CodabarDecoder,
    DataBarDecoder,
) = (
    EanDecoder,
    Code128Decoder,
    Code39Decoder,
    ItfDecoder,
    CodabarDecoder,
    DataBarDecoder,
);

/// The most permissive configuration: every length of ITF, Code 39 without a
/// minimum length, and optional add-ons.
fn options() -> LineOptions {
    let mut o = ScanOptions {
        ean_addon: breader_core::EanAddonPolicy::Optional,
        ..ScanOptions::default()
    };
    o.itf.allowed_lengths.clear();
    o.itf.min_length = 2;
    o.code39.min_length = 1;
    LineOptions::from(&o)
}

fn decode_everything(runs: &Runs, opts: &LineOptions) -> usize {
    let mut rev = Runs::default();
    Runs::reverse_from(runs, &mut rev);
    let mut out = Vec::new();
    ALL.decode(runs, opts, &mut out);
    ALL.decode(&rev, opts, &mut out);
    // Light bars on a dark background: same runs, roles swapped.
    let (mut a, mut b) = (runs.clone(), rev);
    a.flip_roles();
    b.flip_roles();
    ALL.decode(&a, opts, &mut out);
    ALL.decode(&b, opts, &mut out);
    out.len()
}

#[test]
fn random_profiles_yield_no_edges_crash_and_no_hits() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let opts = options();
    let mut detector = EdgeDetector::default();
    let mut edges = Edges::default();
    let mut runs = Runs::default();
    let mut hits = 0;
    for i in 0..2000 {
        let len = match i % 5 {
            0 => rng.below(40) as usize,
            1 => 100 + rng.below(400) as usize,
            _ => 400 + rng.below(1600) as usize,
        };
        let profile: Vec<u8> = match i % 7 {
            0 => (0..len).map(|_| rng.next_u64() as u8).collect(),
            1 => vec![rng.next_u64() as u8; len],
            2 => (0..len).map(|k| if k % 2 == 0 { 0 } else { 255 }).collect(),
            3 => {
                // Random bar widths, a stripe pattern like an unreadable barcode.
                let mut v = Vec::with_capacity(len);
                let mut dark = rng.below(2) == 0;
                while v.len() < len {
                    let w = 1 + rng.below(6) as usize;
                    v.extend(std::iter::repeat_n(if dark { 20 } else { 230 }, w));
                    dark = !dark;
                }
                v.truncate(len);
                v
            }
            4 => (0..len).map(|k| (k * 7 % 256) as u8).collect(),
            5 => {
                let base = rng.below(200) as i32;
                (0..len)
                    .map(|_| (base + rng.below(40) as i32).clamp(0, 255) as u8)
                    .collect()
            }
            _ => (0..len)
                .map(|k| if (k / (1 + i % 11)) % 2 == 0 { 10 } else { 245 })
                .collect(),
        };
        for params in all_params() {
            detector.detect(&profile, &params, &mut edges);
            assert!(edges.as_slice().windows(2).all(|w| w[0].pos < w[1].pos));
            assert!(
                edges
                    .as_slice()
                    .windows(2)
                    .all(|w| w[0].rising != w[1].rising)
            );
            Runs::from_edges(&edges, &mut runs);
            hits += decode_everything(&runs, &opts);
        }
    }
    assert_eq!(hits, 0);
}

#[test]
fn random_run_sequences_yield_no_hits() {
    let mut rng = Rng(0xdead_beef_cafe_f00d);
    let opts = options();
    let mut hits = 0;
    for i in 0..6000 {
        let n = 20 + rng.below(100) as usize;
        let module = 1.0 + rng.unit() * 5.0;
        let widths: Vec<f32> = (0..n)
            .map(|_| match i % 3 {
                0 => module * (1 + rng.below(4)) as f32,
                1 => module * (0.5 + rng.unit() * 4.0),
                _ => module * (1 + rng.below(2)) as f32 + rng.gauss() * 0.2,
            })
            .map(|w| w.max(0.01))
            .collect();
        let runs = Runs::from_widths(
            rng.below(2) == 0,
            &widths,
            rng.unit() * 100.0,
            rng.unit() * 100.0,
        );
        hits += decode_everything(&runs, &opts);
    }
    assert_eq!(hits, 0);
}

#[test]
fn hostile_run_values_are_handled() {
    let opts = options();
    let specials = [0.0, -1.0, f32::NAN, f32::INFINITY, f32::MAX, 1e-30, 3.0];
    let mut rng = Rng(77);
    for _ in 0..500 {
        let n = 33 + rng.below(40) as usize;
        let widths: Vec<f32> = (0..n)
            .map(|_| {
                if rng.below(4) == 0 {
                    specials[rng.below(specials.len() as u64) as usize]
                } else {
                    3.0
                }
            })
            .collect();
        let runs = Runs::from_widths(rng.below(2) == 0, &widths, f32::NAN, f32::INFINITY);
        decode_everything(&runs, &opts);
    }
}

/// Alternating guard-like runs with random digit groups: the structure is
/// plausible, the digits are not valid L/G/R codes in aggregate.
#[test]
fn guard_like_sequences_with_random_digits_are_rejected_or_valid() {
    let mut rng = Rng(4242);
    let opts = options();
    let mut valid = 0;
    for _ in 0..3000 {
        let m = 2.0 + rng.unit() * 2.0;
        let mut widths = vec![m, m, m];
        for _ in 0..6 {
            widths.extend(random_digit(&mut rng, m));
        }
        widths.extend([m; 5]);
        for _ in 0..6 {
            widths.extend(random_digit(&mut rng, m));
        }
        widths.extend([m; 3]);
        let runs = Runs::from_widths(true, &widths, 12.0 * m, 12.0 * m);
        // Random digits pass the check digit one time in ten, so some
        // sequences are legitimately valid; they must then decode cleanly.
        let mut out = Vec::new();
        EanDecoder.decode(&runs, &opts, &mut out);
        for hit in &out {
            assert!(hit.text.iter().all(u8::is_ascii_digit));
            assert!(matches!(hit.text.len(), 12 | 13));
        }
        valid += out.len();
    }
    assert!(valid < 1000, "{valid}");
}

fn random_digit(rng: &mut Rng, m: f32) -> [f32; 4] {
    loop {
        let w: [u64; 4] = [1 + rng.below(4), 1 + rng.below(4), 1 + rng.below(4), 0];
        let rest = 7i64 - (w[0] + w[1] + w[2]) as i64;
        if (1..=4).contains(&rest) {
            return [
                w[0] as f32 * m,
                w[1] as f32 * m,
                w[2] as f32 * m,
                rest as f32 * m,
            ];
        }
    }
}

fn run_decode<D: LinearDecoder>(
    d: &D,
    runs: &Runs,
    opts: &LineOptions,
) -> Vec<crate::decoder::LineHit> {
    let mut out = Vec::new();
    d.decode(runs, opts, &mut out);
    out
}

fn to_runs(modules: &[bool], px: f32) -> Runs {
    let mut widths = Vec::new();
    let mut i = 0;
    while i < modules.len() {
        let start = i;
        while i < modules.len() && modules[i] == modules[start] {
            i += 1;
        }
        widths.push((i - start) as f32 * px);
    }
    Runs::from_widths(modules[0], &widths, 20.0 * px, 20.0 * px)
}

/// Valid structure, random content: whatever is accepted must be exactly what
/// was drawn, and random Code 128 characters rarely pass the checksum.
#[test]
fn structured_random_symbols_never_decode_to_something_else() {
    use crate::testutil::{codabar_modules, code39_modules, code128_modules_raw, itf_modules};
    let mut rng = Rng(0xfeed_f00d);
    let opts = options();
    let (mut c128_hits, mut c39_hits, mut itf_hits, mut cb_hits) = (0, 0, 0, 0);
    for _ in 0..3000 {
        let px = 1.5 + rng.unit() * 3.0;
        let n = 1 + rng.below(12) as usize;
        let mut v = vec![103 + rng.below(3) as u8];
        v.extend((0..n + 1).map(|_| rng.below(103) as u8));
        let modules = code128_modules_raw(&v);
        let hits = run_decode(&Code128Decoder, &to_runs(&modules, px), &opts);
        for h in &hits {
            assert_eq!(h.symbology, breader_core::Symbology::Code128);
        }
        c128_hits += hits.len();

        const CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";
        let text: String = (0..1 + rng.below(10))
            .map(|_| char::from(CHARS[rng.below(43) as usize]))
            .collect();
        let hits = run_decode(
            &Code39Decoder,
            &to_runs(&code39_modules(&text, 2 + rng.below(2) as usize, 1), px),
            &opts,
        );
        assert!(
            hits.iter().all(|h| h.text == text.as_bytes()),
            "{text}: {hits:?}"
        );
        c39_hits += hits.len();

        let pairs = 1 + rng.below(8) as usize;
        let digits = rng.digits(2 * pairs);
        let hits = run_decode(
            &ItfDecoder,
            &to_runs(&itf_modules(&digits, 2 + rng.below(2) as usize), px),
            &opts,
        );
        assert!(
            hits.iter().all(|h| h.text == digits.as_bytes()),
            "{digits}: {hits:?}"
        );
        itf_hits += hits.len();

        const CB: &[u8] = b"0123456789-$:/.+";
        const GUARDS: &[u8] = b"ABCD";
        let body: String = (0..4 + rng.below(8))
            .map(|_| char::from(CB[rng.below(16) as usize]))
            .collect();
        let (a, z) = (
            char::from(GUARDS[rng.below(4) as usize]),
            char::from(GUARDS[rng.below(4) as usize]),
        );
        let hits = run_decode(
            &CodabarDecoder,
            &to_runs(
                &codabar_modules(a, &body, z, 2 + rng.below(2) as usize, 1),
                px,
            ),
            &opts,
        );
        assert!(
            hits.iter().all(|h| h.text == body.as_bytes()),
            "{a}{body}{z}: {hits:?}"
        );
        cb_hits += hits.len();
    }
    // Random check characters pass one time in 103.
    assert!(c128_hits < 120, "{c128_hits}");
    assert!(c39_hits > 2000 && itf_hits > 2000, "{c39_hits} {itf_hits}");
    assert!(cb_hits > 1500, "{cb_hits}");
}

struct Half;

impl Preprocessor for Half {
    fn id(&self) -> &'static str {
        "half"
    }

    fn apply(&self, src: breader_core::ImageView<'_>, dst: &mut ImageBuffer) {
        dst.resize(src.width().div_ceil(2), src.height().div_ceil(2));
        for y in 0..dst.height() {
            for x in 0..dst.width() {
                dst.row_mut(y)[x] = src.get(2 * x, 2 * y);
            }
        }
    }
}

#[test]
fn random_images_yield_no_symbols() {
    let mut rng = Rng(0x1234_5678_9abc_def0);
    let pre: Vec<Box<dyn Preprocessor>> = vec![Box::new(Half)];
    let binarizers: Vec<Box<dyn Binarizer>> = Vec::new();
    let mut scratch = Scratch::new();
    let mut found: Vec<Symbol> = Vec::new();
    for i in 0..120 {
        let (w, h) = (1 + rng.below(220) as usize, 1 + rng.below(140) as usize);
        let mut img = ImageBuffer::new(w, h);
        match i % 4 {
            0 => img
                .as_mut_slice()
                .iter_mut()
                .for_each(|v| *v = rng.next_u64() as u8),
            1 => {
                // Random vertical stripes: every scanline sees the same
                // barcode-like run sequence.
                let mut stripes = Vec::new();
                let mut dark = false;
                while stripes.len() < w {
                    let width = 1 + rng.below(5) as usize;
                    stripes.extend(std::iter::repeat_n(dark, width));
                    dark = !dark;
                }
                for y in 0..h {
                    for (px, &on) in img.row_mut(y).iter_mut().zip(&stripes) {
                        *px = if on { 20 } else { 235 };
                    }
                }
            }
            2 => {
                // Horizontal and vertical stripes mixed.
                for y in 0..h {
                    for x in 0..w {
                        let on = (x / (1 + i % 5) + y / (1 + i % 7)) % 2 == 0;
                        img.row_mut(y)[x] = if on { 30 } else { 220 };
                    }
                }
            }
            _ => {
                let mut x = 128i32;
                for v in img.as_mut_slice() {
                    x = (x + rng.below(41) as i32 - 20).clamp(0, 255);
                    *v = x as u8;
                }
            }
        }
        let efforts: &[Effort] = if i % 4 == 0 {
            &[Effort::Fast, Effort::Balanced, Effort::Exhaustive]
        } else {
            &[Effort::Fast, Effort::Balanced]
        };
        for &effort in efforts {
            let opts = ScanOptions::with_effort(effort);
            for inverted in [false, true] {
                for preprocessor in [None, Some(0)] {
                    let mut ctx =
                        FrameContext::new(img.view(), &opts, &pre, &binarizers, &mut scratch);
                    ctx.set_pass(Pass {
                        preprocessor,
                        binarizer: 0,
                        inverted,
                    });
                    LinearReader.scan(&mut ctx, &mut found);
                }
            }
        }
    }
    assert!(found.is_empty(), "{found:?}");
}
