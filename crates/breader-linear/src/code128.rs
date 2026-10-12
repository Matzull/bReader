//! Code 128 (code sets A, B and C, FNC1 for GS1-128), following
//! ISO/IEC 15417.

use breader_core::Symbology;

use crate::common::{
    MIN_MODULE, adjusted_width, adjusted_widths, quiet_after_open, quiet_before_open,
};
use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::runs::Runs;

/// Bar and space widths of symbol characters 0 to 105 and of the stop
/// character without its termination bar, as printed in the specification.
pub(crate) const PATTERNS: [&str; 107] = [
    "212222", "222122", "222221", "121223", "121322", "131222", "122213", "122312", "132212",
    "221213", "221312", "231212", "112232", "122132", "122231", "113222", "123122", "123221",
    "223211", "221132", "221231", "213212", "223112", "312131", "311222", "321122", "321221",
    "312212", "322112", "322211", "212123", "212321", "232121", "111323", "131123", "131321",
    "112313", "132113", "132311", "211313", "231113", "231311", "112133", "112331", "132131",
    "113123", "113321", "133121", "313121", "211331", "231131", "213113", "213311", "213131",
    "311123", "311321", "331121", "312113", "312311", "332111", "314111", "221411", "431111",
    "111224", "111422", "121124", "121421", "141122", "141221", "112214", "112412", "122114",
    "122411", "142112", "142211", "241211", "221114", "413111", "241112", "134111", "111242",
    "121142", "121241", "114212", "124112", "124211", "411212", "421112", "421211", "212141",
    "214121", "412121", "111143", "111341", "131141", "114113", "114311", "411113", "411311",
    "113141", "114131", "311141", "411131", "211412", "211214", "211232", "233111",
];

const START_A: u8 = 103;
const START_B: u8 = 104;
const START_C: u8 = 105;
const STOP: u8 = 106;
const CODE_A: u8 = 101;
const CODE_B: u8 = 100;
const CODE_C: u8 = 99;
const SHIFT: u8 = 98;
const FNC1: u8 = 102;

const fn widths_of(s: &str) -> [f32; 6] {
    let b = s.as_bytes();
    let mut out = [0.0; 6];
    let mut i = 0;
    while i < 6 {
        out[i] = (b[i] - b'0') as f32;
        i += 1;
    }
    out
}

static WIDTHS: [[f32; 6]; 107] = {
    let mut t = [[0.0; 6]; 107];
    let mut i = 0;
    while i < 107 {
        t[i] = widths_of(PATTERNS[i]);
        i += 1;
    }
    t
};

/// Minimum quiet zone on each side, in modules (specified: 10).
const QUIET: f32 = 5.0;
/// Longest symbol read, in characters between start and stop.
const MAX_CHARS: usize = 256;
/// Largest total deviation of a character from its pattern, in modules.
const MAX_CHAR_ERROR: f32 = 2.2;
/// Required distance between the best and the second best pattern.
const MIN_MARGIN: f32 = 0.7;
/// One character per symbol may fall short of `MIN_MARGIN` down to this
/// margin. A single wrong character always breaks the mod-103 checksum.
const WEAK_MARGIN: f32 = 0.25;
/// Allowed ratio between the module widths of neighboring characters.
const MODULE_DRIFT: f32 = 1.7;
/// Local print-gain deviation searched per character, in modules.
const LOCAL_BIAS_MAX: f32 = 0.5;
const LOCAL_BIAS_STEP: f32 = 0.125;
const LOCAL_BIAS_COST: f32 = 1.5;
/// Largest deviation of the start character from its pattern when it is only
/// screened, before the exact ranking.
const START_SCREEN: f32 = 3.6;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Code128Decoder;

#[derive(Debug, Clone, Copy)]
struct CharMatch {
    value: u8,
    margin: f32,
    weak: bool,
    module: f32,
}

/// Largest reduction of a pattern's cost the local bias can buy.
const MAX_BIAS_GAIN: f32 = (6.0 - LOCAL_BIAS_COST) * LOCAL_BIAS_MAX;

/// Best and second best pattern cost of widths normalized to 11 modules,
/// allowing a local print-gain bias that widens bars and narrows spaces.
/// `None` when no pattern can come within `MAX_CHAR_ERROR`. Patterns that
/// cannot be within `MIN_MARGIN` of the best are not evaluated exactly, so
/// the second best cost is only exact below that distance.
fn rank(n: &[f32; 6], first_is_bar: bool) -> Option<(usize, f32, f32)> {
    let sign = if first_is_bar {
        [1.0, -1.0, 1.0, -1.0, 1.0, -1.0]
    } else {
        [-1.0, 1.0, -1.0, 1.0, -1.0, 1.0]
    };
    let mut plain = [0.0f32; 107];
    let mut min_plain = f32::MAX;
    for (c, p) in plain.iter_mut().zip(&WIDTHS) {
        *c = (0..6).map(|r| (n[r] - p[r]).abs()).sum();
        min_plain = min_plain.min(*c);
    }
    if min_plain - MAX_BIAS_GAIN > MAX_CHAR_ERROR {
        return None;
    }
    let reach = min_plain + MAX_BIAS_GAIN + MIN_MARGIN;
    let steps = (LOCAL_BIAS_MAX / LOCAL_BIAS_STEP) as i32;
    let (mut best, mut best_i, mut second) = (f32::MAX, 0, f32::MAX);
    for (i, p) in WIDTHS.iter().enumerate() {
        if plain[i] > reach {
            continue;
        }
        let mut cost = f32::MAX;
        for t in -steps..=steps {
            let d = t as f32 * LOCAL_BIAS_STEP;
            let mut e = LOCAL_BIAS_COST * d.abs();
            for r in 0..6 {
                e += (n[r] - p[r] - sign[r] * d).abs();
            }
            cost = cost.min(e);
        }
        if cost < best {
            second = best;
            best = cost;
            best_i = i;
        } else if cost < second {
            second = cost;
        }
    }
    Some((best_i, best, second))
}

/// Reads the character whose six runs start at `at`. `bias` is the global
/// print gain in profile samples, `module` the module of the previous
/// character (0 for the first).
fn match_char(runs: &Runs, at: usize, bias: f32, prev_module: f32) -> Option<CharMatch> {
    let w = adjusted_widths::<6>(runs, at, bias)?;
    let total: f32 = w.iter().sum();
    let module = total / 11.0;
    if module < MIN_MODULE
        || (prev_module > 0.0
            && (module < prev_module / MODULE_DRIFT || module > prev_module * MODULE_DRIFT))
    {
        return None;
    }
    let k = 11.0 / total;
    let n = w.map(|x| x * k);
    let (best, cost, second) = rank(&n, runs.is_bar(at))?;
    let margin = (second - cost).min(2.0);
    (cost <= MAX_CHAR_ERROR && margin >= WEAK_MARGIN).then_some(CharMatch {
        value: best as u8,
        margin,
        weak: margin < MIN_MARGIN,
        module,
    })
}

/// Estimates the print gain from the start character, in profile samples.
fn start_bias(runs: &Runs, at: usize, pattern: usize) -> f32 {
    let raw = &runs.widths()[at..at + 6];
    let total: f32 = raw.iter().sum();
    let k = 11.0 / total;
    let sum: f32 = raw
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let e = x * k - WIDTHS[pattern][i];
            if runs.is_bar(at + i) { e } else { -e }
        })
        .sum();
    (sum / 6.0).clamp(-LOCAL_BIAS_MAX, LOCAL_BIAS_MAX) * total / 11.0
}

/// Cheap screen: could the six runs at `at` be a start character?
fn start_pattern(runs: &Runs, at: usize) -> Option<usize> {
    let raw = runs.widths().get(at..at + 6)?;
    let total: f32 = raw.iter().sum();
    if !(total.is_finite() && total / 11.0 >= MIN_MODULE)
        || runs.space_before_open(at) < 0.8 * QUIET * total / 11.0
    {
        return None;
    }
    let k = 11.0 / total;
    let mut best = (f32::MAX, 0);
    let first = usize::from(START_A);
    for (p, pattern) in WIDTHS[first..=usize::from(START_C)]
        .iter()
        .enumerate()
        .map(|(k, w)| (first + k, w))
    {
        let e: f32 = (0..6).map(|i| (raw[i] * k - pattern[i]).abs()).sum();
        if e < best.0 {
            best = (e, p);
        }
    }
    (best.0 <= START_SCREEN).then_some(best.1)
}

struct Read {
    values: [u8; MAX_CHARS + 2],
    len: usize,
    margin_sum: f32,
    module_sum: f32,
    last_bar: usize,
}

/// Checks the start character at run `j` and the quiet zone before it. A
/// weak start character is returned as such and counts as the one weak
/// character a symbol may have.
fn start_of(runs: &Runs, j: usize) -> Option<(CharMatch, f32)> {
    start_pattern(runs, j)?;
    let start = match_char(runs, j, 0.0, 0.0)?;
    if !(START_A..=START_C).contains(&start.value) {
        return None;
    }
    let bias = start_bias(runs, j, usize::from(start.value));
    if quiet_before_open(runs, j, bias) < QUIET * start.module {
        return None;
    }
    Some((start, bias))
}

fn read_symbol(runs: &Runs, j: usize, started: &mut bool) -> Option<Read> {
    let (start, bias) = start_of(runs, j)?;
    *started = true;
    let mut values = [0u8; MAX_CHARS + 2];
    values[0] = start.value;
    let mut read = Read {
        values,
        len: 1,
        margin_sum: start.margin,
        module_sum: start.module,
        last_bar: 0,
    };
    let mut weak = usize::from(start.weak);
    let mut module = start.module;
    for c in 1..=MAX_CHARS {
        let at = j + 6 * c;
        let m = match_char(runs, at, bias, module)?;
        weak += usize::from(m.weak);
        if weak > 1 {
            return None;
        }
        read.margin_sum += m.margin;
        read.module_sum += m.module;
        module = m.module;
        if m.value == STOP {
            if read.len < 3 {
                return None;
            }
            let final_bar = adjusted_width(runs, at + 6, bias)?;
            let ratio = final_bar / module;
            if !(1.3..=3.0).contains(&ratio)
                || quiet_after_open(runs, at + 6, bias) < QUIET * module
            {
                return None;
            }
            read.last_bar = at + 6;
            return Some(read);
        }
        if m.value >= START_A {
            return None;
        }
        read.values[read.len] = m.value;
        read.len += 1;
    }
    None
}

/// Mod-103 check over `values` = start, data..., check.
fn checksum_ok(values: &[u8]) -> bool {
    let Some((&check, body)) = values.split_last() else {
        return false;
    };
    let sum: u32 = body
        .iter()
        .enumerate()
        .map(|(i, &v)| u32::from(v) * (i.max(1) as u32))
        .sum();
    sum % 103 == u32::from(check)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Set {
    A,
    B,
    C,
}

/// Interprets the symbol characters between start and check character.
/// Returns the payload bytes and the symbology identifier.
fn interpret(start: u8, data: &[u8]) -> Option<(Vec<u8>, [u8; 3])> {
    let mut set = match start {
        START_A => Set::A,
        START_B => Set::B,
        START_C => Set::C,
        _ => return None,
    };
    let mut out = Vec::with_capacity(data.len() * 2);
    let mut aim = *b"]C0";
    let mut shift = false;
    let mut fnc4_next = false;
    let mut fnc4_latched = false;
    for (i, &v) in data.iter().enumerate() {
        let eff = match (set, shift) {
            (Set::A, true) => Set::B,
            (Set::B, true) => Set::A,
            (s, _) => s,
        };
        if eff == Set::C {
            match v {
                0..=99 => {
                    if fnc4_next {
                        return None;
                    }
                    out.push(b'0' + v / 10);
                    out.push(b'0' + v % 10);
                }
                CODE_B => set = Set::B,
                CODE_A => set = Set::A,
                FNC1 => push_fnc1(i, data, set, &mut out, &mut aim, fnc4_next)?,
                _ => return None,
            }
            continue;
        }
        match v {
            0..=95 => {
                let base = if eff == Set::A && v >= 64 {
                    v - 64
                } else {
                    v + 32
                };
                let extended = fnc4_latched != fnc4_next;
                out.push(if extended { base | 0x80 } else { base });
                fnc4_next = false;
                shift = false;
            }
            96 | 97 => return None,
            SHIFT => {
                if shift {
                    return None;
                }
                shift = true;
                continue;
            }
            CODE_C => {
                if fnc4_next || shift {
                    return None;
                }
                set = Set::C;
            }
            CODE_B if eff == Set::A => {
                if shift {
                    return None;
                }
                set = Set::B;
            }
            CODE_A if eff == Set::B => {
                if shift {
                    return None;
                }
                set = Set::A;
            }
            CODE_B | CODE_A => {
                // FNC4 in the set where its value is not a code switch.
                if shift {
                    return None;
                }
                if fnc4_next {
                    fnc4_latched = !fnc4_latched;
                    fnc4_next = false;
                } else {
                    fnc4_next = true;
                }
            }
            FNC1 => {
                if shift {
                    return None;
                }
                push_fnc1(i, data, set, &mut out, &mut aim, fnc4_next)?;
            }
            _ => return None,
        }
    }
    if shift || fnc4_next || out.is_empty() {
        return None;
    }
    Some((out, aim))
}

/// FNC1 in the first position selects GS1-128, in the second position an
/// application indicator; anywhere else it separates fields as GS.
fn push_fnc1(
    i: usize,
    data: &[u8],
    set: Set,
    out: &mut Vec<u8>,
    aim: &mut [u8; 3],
    fnc4_next: bool,
) -> Option<()> {
    if fnc4_next {
        return None;
    }
    if i == 0 {
        *aim = *b"]C1";
    } else if i == 1 && *aim == *b"]C0" && out.len() == if set == Set::C { 2 } else { 1 } {
        let first_is_data = data[0] <= if set == Set::C { 99 } else { 95 };
        if first_is_data {
            *aim = *b"]C2";
            return Some(());
        }
        out.push(0x1d);
    } else {
        out.push(0x1d);
    }
    Some(())
}

impl LinearDecoder for Code128Decoder {
    fn decode(&self, runs: &Runs, opts: &LineOptions, out: &mut Vec<LineHit>) -> bool {
        if !opts.symbologies.contains(Symbology::Code128) {
            return false;
        }
        let mut started = false;
        let n = runs.len();
        let mut j = usize::from(n > 0 && !runs.is_bar(0));
        while j + 6 * 4 < n {
            if let Some(read) = read_symbol(runs, j, &mut started)
                && let values = &read.values[..read.len]
                && checksum_ok(values)
                && let Some((text, aim)) = interpret(values[0], &values[1..values.len() - 1])
            {
                let chars = read.len as f32 + 1.0;
                out.push(LineHit {
                    symbology: Symbology::Code128,
                    text,
                    addon: None,
                    start: runs.start(j),
                    end: runs.end(read.last_bar),
                    module: read.module_sum / chars,
                    quality: (read.margin_sum / chars / 2.0).clamp(0.0, 1.0),
                    aim,
                    checksum_verified: true,
                    short_quiet: false,
                    inverted: false,
                    start_stop: None,
                    segments: 0,
                });
            }
            j += 2;
        }
        started
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    fn opts() -> LineOptions {
        line_options(|_| {})
    }

    fn profile(modules: &[bool], px: f32) -> Vec<u8> {
        Render::px(px).profile(modules)
    }

    fn texts(hits: &[LineHit]) -> Vec<Vec<u8>> {
        hits.iter().map(|h| h.text.clone()).collect()
    }

    fn encode(start: u8, text: &str) -> Vec<bool> {
        let mut v = vec![start];
        v.extend(text.chars().map(b));
        code128_modules(&v)
    }

    #[test]
    fn pattern_table_is_consistent() {
        let mut seen = std::collections::HashSet::new();
        for (i, p) in PATTERNS.iter().enumerate() {
            let w: Vec<u32> = p.bytes().map(|b| u32::from(b - b'0')).collect();
            assert_eq!(w.len(), 6);
            assert_eq!(w.iter().sum::<u32>(), 11, "pattern {i}");
            assert_eq!(
                (w[0] + w[2] + w[4]) % 2,
                0,
                "bars of pattern {i} must be even"
            );
            assert!(w.iter().all(|&x| (1..=4).contains(&x)));
            assert!(seen.insert(*p), "duplicate pattern {i}");
        }
    }

    #[test]
    fn reads_at_every_pixel_scale_in_both_directions_and_polarities() {
        let modules = encode(START_B, "Hello-128");
        for px in [1.5, 2.0, 2.5, 3.0, 4.0] {
            let p = profile(&modules, px);
            let hits = decode_all(&Code128Decoder, &p, &opts());
            assert_eq!(texts(&hits), [b"Hello-128".to_vec()], "px {px}");
            assert_eq!(&hits[0].aim, b"]C0");
            assert!(hits[0].checksum_verified && !hits[0].inverted);

            let mut rev = p.clone();
            rev.reverse();
            let hits = decode_all(&Code128Decoder, &rev, &opts());
            assert_eq!(texts(&hits), [b"Hello-128".to_vec()], "reversed px {px}");

            let r = Render {
                dark: 225.0,
                light: 25.0,
                ..Render::px(px)
            };
            let hits = decode_all(&Code128Decoder, &r.profile(&modules), &opts());
            assert_eq!(texts(&hits), [b"Hello-128".to_vec()], "inverted px {px}");
            assert!(hits[0].inverted);
        }
    }

    #[test]
    fn reads_through_noise_gain_and_blur() {
        let modules = encode(START_B, "Noisy 42");
        let mut rng = Rng(5);
        let mut ok = 0;
        for i in 0..40 {
            let r = Render {
                sigma: 0.5 + (i % 4) as f32 * 0.15,
                gain: (i % 5) as f32 * 0.12 - 0.2,
                offset: rng.unit(),
                ..Render::px(2.5)
            };
            let mut p = r.profile(&modules);
            add_noise(&mut p, 6.0, &mut rng);
            let hits = decode_all(&Code128Decoder, &p, &opts());
            assert!(hits.iter().all(|h| h.text == b"Noisy 42"), "{hits:?}");
            ok += usize::from(!hits.is_empty());
        }
        assert!(ok >= 36, "{ok}");
    }

    #[test]
    fn code_sets_switch_and_shift() {
        // Start C, 12 34, CODE B, "ab", CODE A, TAB (value 73), CODE C, 56.
        let v = [
            START_C,
            12,
            34,
            CODE_B,
            b('a'),
            b('b'),
            CODE_A,
            73,
            CODE_C,
            56,
        ];
        let hits = decode_all(
            &Code128Decoder,
            &profile(&code128_modules(&v), 3.0),
            &opts(),
        );
        assert_eq!(texts(&hits), [b"1234ab\t56".to_vec()]);
        // Start A, "A", SHIFT, 'b' (from set B), "C".
        let v = [START_A, 33, SHIFT, b('b'), 35];
        let hits = decode_all(
            &Code128Decoder,
            &profile(&code128_modules(&v), 3.0),
            &opts(),
        );
        assert_eq!(texts(&hits), [b"AbC".to_vec()]);
    }

    #[test]
    fn interpret_covers_sets_fnc1_and_fnc4() {
        let d = |start, data: &[u8]| interpret(start, data);
        assert_eq!(d(START_B, &[b('A')]), Some((b"A".to_vec(), *b"]C0")));
        assert_eq!(
            d(START_A, &[0, 63, 64, 95]),
            Some((vec![b' ', b'_', 0, 31], *b"]C0"))
        );
        assert_eq!(d(START_C, &[0, 99]), Some((b"0099".to_vec(), *b"]C0")));
        // GS1-128: FNC1 first, a later FNC1 becomes the GS separator.
        let gs1 = [FNC1, 1, 12, 34, 56, 78, 90, 12, 31, CODE_B, FNC1, b('1')];
        assert_eq!(
            d(START_C, &gs1),
            Some((b"0112345678901231\x1d1".to_vec(), *b"]C1"))
        );
        // FNC1 in second position selects the application indicator form.
        assert_eq!(
            d(START_B, &[b('a'), FNC1, b('1'), b('2')]),
            Some((b"a12".to_vec(), *b"]C2"))
        );
        assert_eq!(
            d(START_C, &[42, FNC1, 12]),
            Some((b"4212".to_vec(), *b"]C2"))
        );
        // FNC1 elsewhere stays a separator under ]C0.
        assert_eq!(
            d(START_B, &[b('a'), b('b'), FNC1, b('c')]),
            Some((b"ab\x1dc".to_vec(), *b"]C0"))
        );
        // FNC4: single, latched and toggled off again.
        assert_eq!(d(START_B, &[CODE_B, b('i')]).map(|r| r.0), Some(vec![0xe9]));
        assert_eq!(d(START_A, &[101, 33]).map(|r| r.0), Some(vec![b'A' | 0x80]));
        assert_eq!(
            d(START_B, &[100, 100, b('a'), b('b'), 100, 100, b('c')]).map(|r| r.0),
            Some(vec![b'a' | 0x80, b'b' | 0x80, b'c'])
        );
        assert_eq!(
            d(START_B, &[100, 100, 100, b('a')]).map(|r| r.0),
            Some(vec![b'a'])
        );
    }

    #[test]
    fn interpret_rejects_malformed_streams() {
        let d = |start, data: &[u8]| interpret(start, data);
        assert!(d(START_B, &[]).is_none(), "no data");
        assert!(d(START_B, &[FNC1]).is_none(), "nothing but FNC1");
        assert!(d(START_B, &[96, b('a')]).is_none(), "FNC3");
        assert!(d(START_B, &[97, b('a')]).is_none(), "FNC2");
        assert!(d(START_B, &[b('a'), SHIFT]).is_none(), "dangling shift");
        assert!(d(START_B, &[SHIFT, SHIFT, b('a')]).is_none());
        assert!(d(START_B, &[SHIFT, CODE_C, 12]).is_none());
        assert!(d(START_B, &[100, b('a'), 100]).is_none(), "dangling FNC4");
        assert!(d(START_B, &[100, CODE_C, 12]).is_none());
        assert!(d(START_C, &[100, 12]).is_some());
        assert!(d(START_C, &[104]).is_none(), "start code inside the symbol");
        assert!(d(START_C, &[100]).is_none(), "only a code switch");
        assert!(d(7, &[1]).is_none());
    }

    #[test]
    fn minimum_symbol_is_one_data_character() {
        let hits = decode_all(
            &Code128Decoder,
            &profile(&encode(START_B, "Z"), 3.0),
            &opts(),
        );
        assert_eq!(texts(&hits), [b"Z".to_vec()]);
    }

    #[test]
    fn checksum_is_mandatory() {
        let mut v = vec![START_B, b('A'), b('B'), b('C')];
        let good = code128_check(&v);
        v.push(good);
        assert_eq!(
            decode_all(
                &Code128Decoder,
                &profile(&code128_modules_raw(&v), 3.0),
                &opts()
            )
            .len(),
            1
        );
        for wrong in (0..103u8).filter(|&c| c != good) {
            *v.last_mut().unwrap() = wrong;
            let p = profile(&code128_modules_raw(&v), 3.0);
            assert!(
                decode_all(&Code128Decoder, &p, &opts()).is_empty(),
                "check {wrong}"
            );
        }
    }

    /// Element widths of a symbol in modules, starting with a bar.
    fn element_widths(modules: &[bool]) -> Vec<f32> {
        let mut out = vec![0.0f32];
        for i in 0..modules.len() {
            if i > 0 && modules[i] != modules[i - 1] {
                out.push(0.0);
            }
            *out.last_mut().unwrap() += 1.0;
        }
        out
    }

    #[test]
    fn a_weak_start_character_is_read_and_counts_as_the_weak_one() {
        let widths = element_widths(&encode(START_B, "Weak start"));
        let read = |w: &[f32]| {
            let mut out = Vec::new();
            let runs = Runs::from_widths(true, w, 30.0, 30.0);
            Code128Decoder.decode(&runs, &opts(), &mut out);
            out
        };
        assert_eq!(texts(&read(&widths)), [b"Weak start".to_vec()]);
        // Move width between the last bar and space of the start character
        // (toward start C) until the match to its pattern is ambiguous but still accepted.
        let weak_start = |w: &[f32]| {
            let runs = Runs::from_widths(true, w, 30.0, 30.0);
            start_of(&runs, 0).is_some_and(|(s, _)| s.weak)
        };
        let shifted = (1..40)
            .map(|k| {
                let mut w = widths.clone();
                w[4] += k as f32 * 0.04;
                w[5] -= k as f32 * 0.04;
                w
            })
            .find(|w| weak_start(w))
            .expect("a perturbation that makes the start weak");
        assert_eq!(texts(&read(&shifted)), [b"Weak start".to_vec()]);
        // A second weak character is one too many.
        let mut both = shifted.clone();
        let at = 6 * 3;
        let d = shifted[4] - widths[4];
        both[at + 4] += d;
        both[at + 5] -= d;
        let runs = Runs::from_widths(true, &both, 30.0, 30.0);
        let weak_second = match_char(&runs, at, 0.0, 1.0).is_some_and(|m| m.weak);
        assert!(weak_second);
        assert!(read(&both).is_empty());
    }

    #[test]
    fn quiet_zone_of_five_modules_is_required() {
        let modules = encode(START_B, "Quiet");
        let at = |left: f32, right: f32| {
            let r = Render {
                quiet_left: left,
                quiet_right: right,
                ..Render::px(3.0)
            };
            decode_all(&Code128Decoder, &r.profile(&modules), &opts()).len()
        };
        assert_eq!(at(6.0, 6.0), 1);
        // Content limits the quiet zone here; a border does not (below).
        let near = |left: usize, right: usize| {
            let p = Render::px(3.0).profile(&with_neighbours(&modules, left, right));
            decode_all(&Code128Decoder, &p, &opts()).len()
        };
        assert_eq!(near(12, 12), 1);
        assert_eq!(near(3, 12), 0);
        assert_eq!(near(12, 3), 0);
    }

    #[test]
    fn a_quiet_zone_ending_at_the_profile_border_is_open() {
        let modules = encode(START_B, "Quiet");
        for (left, right) in [(3.0, 12.0), (12.0, 3.0), (1.0, 1.0), (2.0, 4.0)] {
            let r = Render {
                quiet_left: left,
                quiet_right: right,
                ..Render::px(3.0)
            };
            let hits = decode_all(&Code128Decoder, &r.profile(&modules), &opts());
            assert_eq!(texts(&hits), [b"Quiet".to_vec()], "{left} {right}");
        }
    }

    #[test]
    fn a_symbol_cut_by_the_border_is_not_read_shorter() {
        let modules = encode(START_B, "Cut by the border");
        let r = Render {
            quiet_left: 4.0,
            quiet_right: 4.0,
            ..Render::px(3.0)
        };
        let p = r.profile(&modules);
        assert_eq!(decode_all(&Code128Decoder, &p, &opts()).len(), 1);
        for (depth, cut) in border_cuts(&p, 12.0, modules.len() as f32 * 3.0) {
            for hit in decode_all(&Code128Decoder, &cut, &opts()) {
                assert_eq!(hit.text, b"Cut by the border", "depth {depth}");
            }
            if depth > 9 {
                assert!(
                    decode_all(&Code128Decoder, &cut, &opts()).is_empty(),
                    "depth {depth}"
                );
            }
        }
    }

    #[test]
    fn perturbed_widths_are_read_correctly_or_not_at_all() {
        let mut rng = Rng(99);
        let text = "AmbiguityCheck";
        let modules = encode(START_B, text);
        let (mut right, mut none) = (0, 0);
        for i in 0..400 {
            let amount = 0.2 + (i % 8) as f32 * 0.12;
            let mut dark = Vec::new();
            let mut x = 40.0f32;
            let mut j = 0;
            while j < modules.len() {
                let start = j;
                while j < modules.len() && modules[j] == modules[start] {
                    j += 1;
                }
                let w = (j - start) as f32 * 3.0 + rng.gauss() * amount * 3.0 / 2.0;
                if modules[start] {
                    dark.push((x, x + w.max(1.0)));
                }
                x += w.max(1.0);
            }
            let p = render_intervals(&dark, x as usize + 60, 25.0, 225.0, 0.6);
            let hits = decode_all(&Code128Decoder, &p, &opts());
            for h in &hits {
                assert_eq!(h.text, text.as_bytes(), "misread at perturbation {amount}");
            }
            if hits.is_empty() {
                none += 1
            } else {
                right += 1
            }
        }
        assert!(right > 100 && none > 0, "{right} {none}");
    }

    #[test]
    fn symbols_cut_off_or_without_stop_are_not_read() {
        let modules = encode(START_B, "Truncated");
        let cut = &modules[..modules.len() - 20];
        let mut m = cut.to_vec();
        m.extend(std::iter::repeat_n(false, 20));
        assert!(decode_all(&Code128Decoder, &profile(&m, 3.0), &opts()).is_empty());
        let tail = &modules[30..];
        assert!(decode_all(&Code128Decoder, &profile(tail, 3.0), &opts()).is_empty());
    }

    #[test]
    fn disabled_symbology_is_skipped() {
        let o = line_options(|o| o.symbologies = breader_core::SymbologySet::EMPTY);
        assert!(decode_all(&Code128Decoder, &profile(&encode(START_B, "Off"), 3.0), &o).is_empty());
    }
}
