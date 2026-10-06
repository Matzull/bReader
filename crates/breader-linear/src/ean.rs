//! EAN-13, EAN-8, UPC-A (as EAN-13 with a leading zero), UPC-E and the
//! EAN-2 / EAN-5 supplements, following ISO/IEC 15420.

use breader_core::{EanAddonPolicy, Symbology};

use crate::decoder::{LineHit, LineOptions, LinearDecoder};
use crate::runs::Runs;

/// L-code of each digit, 1 = bar. G-code is its complement read backwards,
/// R-code its complement.
const L_CODE: [u8; 10] = [
    0b0001101, 0b0011001, 0b0010011, 0b0111101, 0b0100011, 0b0110001, 0b0101111, 0b0111011,
    0b0110111, 0b0001011,
];

/// Run widths of a 7-module digit; panics at compile time if a pattern does
/// not consist of exactly four runs.
const fn run_widths(bits: u8) -> [u8; 4] {
    let mut out = [0u8; 4];
    let mut run = 0;
    out[0] = 1;
    let mut prev = (bits >> 6) & 1;
    let mut i = 1;
    while i < 7 {
        let b = (bits >> (6 - i)) & 1;
        if b == prev {
            out[run] += 1;
        } else {
            run += 1;
            out[run] = 1;
            prev = b;
        }
        i += 1;
    }
    out
}

/// Widths of the L-patterns (index 0-9) followed by the G-patterns (10-19).
/// R-patterns have the same widths as L-patterns, starting with a bar.
static PATTERNS: [[f32; 4]; 20] = {
    let mut t = [[0.0; 4]; 20];
    let mut d = 0;
    while d < 10 {
        let w = run_widths(L_CODE[d]);
        let mut k = 0;
        while k < 4 {
            t[d][k] = w[k] as f32;
            t[d + 10][3 - k] = w[k] as f32;
            k += 1;
        }
        d += 1;
    }
    t
};

/// Bit i (MSB first of `n` positions) is set where `s` has `one`.
const fn mask(s: &str, one: u8) -> u8 {
    let b = s.as_bytes();
    let mut m = 0;
    let mut i = 0;
    while i < b.len() {
        m = (m << 1) | (b[i] == one) as u8;
        i += 1;
    }
    m
}

const fn masks(table: [&str; 10], one: u8) -> [u8; 10] {
    let mut out = [0; 10];
    let mut i = 0;
    while i < 10 {
        out[i] = mask(table[i], one);
        i += 1;
    }
    out
}

/// G-parity masks of the six left digits, indexed by the first digit.
const EAN13_FIRST: [u8; 10] = masks(
    [
        "LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL",
        "LGGLGL",
    ],
    b'G',
);

/// Even-parity (G) masks of UPC-E number system 0, indexed by the check digit.
/// Number system 1 uses the complement.
const UPCE_NS0: [u8; 10] = masks(
    [
        "EEEOOO", "EEOEOO", "EEOOEO", "EEOOOE", "EOEEOO", "EOOEEO", "EOOOEE", "EOEOEO", "EOEOOE",
        "EOOEOE",
    ],
    b'E',
);

/// G-parity masks of the EAN-5 digits, indexed by the supplement checksum.
const EAN5_PARITY: [u8; 10] = masks(
    [
        "GGLLL", "GLGLL", "GLLGL", "GLLLG", "LGGLL", "LLGGL", "LLLGG", "LGLGL", "LGLLG", "LLGLG",
    ],
    b'G',
);

/// Minimum quiet zone next to the symbol, in modules; the specification asks
/// for 7 to 11, photographed symbols are often tighter. The short symbols are
/// easier to imitate and get a larger minimum.
const QUIET_EAN13: f32 = 4.5;
const QUIET_EAN8: f32 = 5.5;
const QUIET_UPCE: f32 = 6.5;
const ADDON_QUIET: f32 = 3.5;
/// Gap between symbol and supplement, in modules (specified: 7 to 12).
const ADDON_GAP: (f32, f32) = (5.0, 14.0);
/// Largest total width deviation of a digit from its pattern, in modules.
const MAX_DIGIT_ERROR: f32 = 1.3;
/// Required distance between the best and second best pattern.
const MIN_DIGIT_MARGIN: f32 = 0.45;
/// One digit per symbol may fall short of `MIN_DIGIT_MARGIN` down to this
/// margin. A single wrong digit always breaks the check digit, so such a
/// digit cannot make a wrong symbol pass by itself.
const WEAK_DIGIT_MARGIN: f32 = 0.12;
/// Allowed ratio between a digit's module width and the guard module width.
const MODULE_RATIO: (f32, f32) = (0.6, 1.7);
/// Allowed deviation of guard runs from the estimated module width.
const GUARD_TOLERANCE: f32 = 0.55;
/// Largest ratio between the module widths measured at different guards.
const MODULE_DRIFT: f32 = 1.8;
const MIN_MODULE: f32 = 0.75;

#[derive(Debug, Clone, Copy, Default)]
pub struct EanDecoder;

#[derive(Debug, Clone, Copy)]
struct Calibration {
    /// Mean module width over all guards.
    module: f32,
    /// Bars measure `module + bias`, spaces `module - bias` (print gain, blur).
    bias: f32,
    /// Run offset of each guard's center and the module width measured there.
    /// Perspective and curved surfaces make the module drift along a symbol.
    anchors: [(f32, f32); 3],
    anchor_count: usize,
}

impl Calibration {
    fn adjusted(&self, runs: &Runs, idx: usize) -> f32 {
        let w = runs.widths()[idx];
        (if runs.is_bar(idx) {
            w - self.bias
        } else {
            w + self.bias
        })
        .max(0.0)
    }

    fn start(&self) -> f32 {
        self.anchors[0].1
    }

    fn end(&self) -> f32 {
        self.anchors[self.anchor_count - 1].1
    }

    /// Module width interpolated between the guards at run offset `offset`
    /// from the start guard.
    fn module_at(&self, offset: f32) -> f32 {
        let anchors = &self.anchors[..self.anchor_count];
        for pair in anchors.windows(2) {
            if offset <= pair[1].0 {
                let t = ((offset - pair[0].0) / (pair[1].0 - pair[0].0)).clamp(0.0, 1.0);
                return pair[0].1 + t * (pair[1].1 - pair[0].1);
            }
        }
        self.end()
    }
}

#[derive(Debug, Clone, Copy)]
struct Layout {
    quiet: f32,
    total: usize,
    guards: &'static [(usize, usize)],
    left: usize,
    right_at: usize,
    right: usize,
}

const EAN13: Layout = Layout {
    quiet: QUIET_EAN13,
    total: 59,
    guards: &[(0, 3), (27, 32), (56, 59)],
    left: 6,
    right_at: 32,
    right: 6,
};
const EAN8: Layout = Layout {
    quiet: QUIET_EAN8,
    total: 43,
    guards: &[(0, 3), (19, 24), (40, 43)],
    left: 4,
    right_at: 24,
    right: 4,
};
const UPCE: Layout = Layout {
    quiet: QUIET_UPCE,
    total: 33,
    guards: &[(0, 3), (27, 33)],
    left: 6,
    right_at: 0,
    right: 0,
};

#[derive(Debug, Clone, Copy, Default)]
struct DigitMatch {
    digit: u8,
    g: bool,
    margin: f32,
    /// Margin below `MIN_DIGIT_MARGIN`.
    weak: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Set {
    LAndG,
    LOnly,
}

/// Local print-gain deviation searched per digit, in modules.
const LOCAL_BIAS_MAX: f32 = 0.5;
const LOCAL_BIAS_STEP: f32 = 0.125;
/// Cost per module of local bias, which keeps the search from explaining
/// away genuine pattern differences.
const LOCAL_BIAS_COST: f32 = 1.5;

/// Best and second best pattern cost of normalized digit widths.
fn rank_patterns(n: [f32; 4], set: Set, starts_with_bar: bool) -> (usize, f32, f32) {
    let sign = if starts_with_bar {
        [1.0, -1.0, 1.0, -1.0]
    } else {
        [-1.0, 1.0, -1.0, 1.0]
    };
    let count = if set == Set::LAndG { 20 } else { 10 };
    let steps = (LOCAL_BIAS_MAX / LOCAL_BIAS_STEP) as i32;
    let (mut best, mut best_i, mut second) = (f32::MAX, 0, f32::MAX);
    for (i, p) in PATTERNS[..count].iter().enumerate() {
        let mut cost = f32::MAX;
        for t in -steps..=steps {
            let d = t as f32 * LOCAL_BIAS_STEP;
            let e: f32 = (0..4)
                .map(|r| (n[r] - p[r] - sign[r] * d).abs())
                .sum::<f32>()
                + LOCAL_BIAS_COST * d.abs();
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
    (best_i, best, second)
}

/// `widths` are corrected for the global bias; `starts_with_bar` tells which
/// runs a local bias widens.
fn match_digit(widths: [f32; 4], set: Set, starts_with_bar: bool) -> Option<DigitMatch> {
    let total: f32 = widths.iter().sum();
    if total.partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return None;
    }
    let k = 7.0 / total;
    let (best_i, best, second) = rank_patterns(widths.map(|w| w * k), set, starts_with_bar);
    let margin = second - best;
    (best <= MAX_DIGIT_ERROR && margin >= WEAK_DIGIT_MARGIN).then_some(DigitMatch {
        digit: (best_i % 10) as u8,
        g: best_i >= 10,
        margin,
        weak: margin < MIN_DIGIT_MARGIN,
    })
}

/// Reads the digit whose four runs start at `at`, `offset` runs from the
/// start guard (use `f32::MAX` past the end guard).
fn read_digit(
    runs: &Runs,
    at: usize,
    offset: f32,
    cal: Calibration,
    set: Set,
) -> Option<DigitMatch> {
    let w = [
        cal.adjusted(runs, at),
        cal.adjusted(runs, at + 1),
        cal.adjusted(runs, at + 2),
        cal.adjusted(runs, at + 3),
    ];
    let module = w.iter().sum::<f32>() / 7.0;
    let expected = cal.module_at(offset + 1.5);
    if module < expected * MODULE_RATIO.0 || module > expected * MODULE_RATIO.1 {
        return None;
    }
    match_digit(w, set, runs.is_bar(at))
}

fn calibrate(runs: &Runs, j: usize, guards: &[(usize, usize)]) -> Option<Calibration> {
    let (mut bars, mut spaces) = ((0.0f32, 0u32), (0.0f32, 0u32));
    for &(a, b) in guards {
        for o in a..b {
            let w = runs.widths()[j + o];
            if o % 2 == 0 {
                bars = (bars.0 + w, bars.1 + 1);
            } else {
                spaces = (spaces.0 + w, spaces.1 + 1);
            }
        }
    }
    if bars.1 == 0 || spaces.1 == 0 || guards.len() > 3 {
        return None;
    }
    let (mb, ms) = (bars.0 / bars.1 as f32, spaces.0 / spaces.1 as f32);
    let mut cal = Calibration {
        module: (mb + ms) / 2.0,
        bias: (mb - ms) / 2.0,
        anchors: [(0.0, 0.0); 3],
        anchor_count: guards.len(),
    };
    if cal.module < MIN_MODULE || cal.bias.abs() > cal.module * 0.5 {
        return None;
    }
    let (mut lo, mut hi) = (f32::MAX, 0.0f32);
    let mut anchors = [(0.0, 0.0); 3];
    for (slot, &(a, b)) in anchors.iter_mut().zip(guards) {
        let sum: f32 = (a..b).map(|o| cal.adjusted(runs, j + o)).sum();
        let module = sum / (b - a) as f32;
        *slot = ((a + b - 1) as f32 / 2.0, module);
        lo = lo.min(module);
        hi = hi.max(module);
    }
    cal.anchors = anchors;
    if lo < MIN_MODULE || hi > lo * MODULE_DRIFT {
        return None;
    }
    for (slot, &(a, b)) in cal.anchors.iter().zip(guards) {
        for o in a..b {
            if (cal.adjusted(runs, j + o) - slot.1).abs() > GUARD_TOLERANCE * slot.1 {
                return None;
            }
        }
    }
    Some(cal)
}

/// Cheap test of the start guard and the space in front of it, using the
/// smallest quiet zone of any layout.
fn plausible_start(runs: &Runs, j: usize) -> bool {
    let w = &runs.widths()[j..j + 3];
    let mean = (w[0] + w[1] + w[2]) / 3.0;
    if mean < MIN_MODULE {
        return false;
    }
    w.iter().all(|&x| x > 0.45 * mean && x < 1.9 * mean)
        && runs.space_before(j) >= QUIET_EAN13 * mean * 0.8
}

struct Read {
    cal: Calibration,
    left: [DigitMatch; 6],
    right: [DigitMatch; 6],
    margin_sum: f32,
    digits: usize,
}

fn read_layout(runs: &Runs, j: usize, layout: &Layout) -> Option<Read> {
    let cal = calibrate(runs, j, layout.guards)?;
    let last = j + layout.total - 1;
    if runs.space_before(j) < layout.quiet * cal.start()
        || runs.space_after(last) < layout.quiet * cal.end()
    {
        return None;
    }
    let mut read = Read {
        cal,
        left: [DigitMatch::default(); 6],
        right: [DigitMatch::default(); 6],
        margin_sum: 0.0,
        digits: layout.left + layout.right,
    };
    let mut weak = 0;
    for i in 0..layout.left {
        let m = read_digit(runs, j + 3 + 4 * i, (3 + 4 * i) as f32, cal, Set::LAndG)?;
        read.margin_sum += m.margin;
        weak += usize::from(m.weak);
        read.left[i] = m;
    }
    for i in 0..layout.right {
        let m = read_digit(
            runs,
            j + layout.right_at + 4 * i,
            (layout.right_at + 4 * i) as f32,
            cal,
            Set::LOnly,
        )?;
        read.margin_sum += m.margin;
        weak += usize::from(m.weak);
        read.right[i] = m;
    }
    (weak <= 1).then_some(read)
}

fn parity_mask(m: &[DigitMatch]) -> u8 {
    m.iter().fold(0, |acc, d| (acc << 1) | u8::from(d.g))
}

/// GS1 mod-10: the last digit is the check digit.
pub(crate) fn check_digit_ok(digits: &[u8]) -> bool {
    let Some((&check, body)) = digits.split_last() else {
        return false;
    };
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| u32::from(d) * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    (10 - sum % 10) % 10 == u32::from(check)
}

fn ascii(digits: &[u8]) -> Vec<u8> {
    digits.iter().map(|d| b'0' + d).collect()
}

/// Expands zero-suppressed UPC-E digits into the 12-digit UPC-A form.
fn upce_to_upca(ns: u8, d: &[u8; 6], check: u8) -> [u8; 12] {
    let mut a = [0u8; 12];
    a[0] = ns;
    let tail = &mut a[1..11];
    match d[5] {
        0..=2 => {
            tail[..2].copy_from_slice(&d[..2]);
            tail[2] = d[5];
            tail[7..].copy_from_slice(&d[2..5]);
        }
        3 => {
            tail[..3].copy_from_slice(&d[..3]);
            tail[8..].copy_from_slice(&d[3..5]);
        }
        4 => {
            tail[..4].copy_from_slice(&d[..4]);
            tail[9] = d[4];
        }
        _ => {
            tail[..5].copy_from_slice(&d[..5]);
            tail[9] = d[5];
        }
    }
    a[11] = check;
    a
}

impl EanDecoder {
    fn hit(
        runs: &Runs,
        j: usize,
        layout: &Layout,
        read: &Read,
        symbology: Symbology,
        text: Vec<u8>,
        opts: &LineOptions,
    ) -> LineHit {
        let addon = if opts.ean_addon == EanAddonPolicy::Ignore {
            None
        } else {
            read_addon(runs, j + layout.total + 1, read.cal)
        };
        LineHit {
            symbology,
            text,
            addon,
            start: runs.start(j),
            end: runs.end(j + layout.total - 1),
            module: read.cal.module,
            quality: (read.margin_sum / read.digits as f32 / 2.0).clamp(0.0, 1.0),
            aim: if symbology == Symbology::Ean8 {
                *b"]E4"
            } else {
                *b"]E0"
            },
            checksum_verified: true,
            inverted: false,
        }
    }

    fn ean13(runs: &Runs, j: usize, opts: &LineOptions) -> Option<LineHit> {
        let upca = opts.symbologies.contains(Symbology::UpcA);
        let ean13 = opts.symbologies.contains(Symbology::Ean13);
        if !upca && !ean13 {
            return None;
        }
        let read = read_layout(runs, j, &EAN13)?;
        let first = EAN13_FIRST
            .iter()
            .position(|&m| m == parity_mask(&read.left))? as u8;
        let mut digits = [0u8; 13];
        digits[0] = first;
        for i in 0..6 {
            digits[1 + i] = read.left[i].digit;
            digits[7 + i] = read.right[i].digit;
        }
        if !check_digit_ok(&digits) {
            return None;
        }
        let (symbology, text) = if first == 0 && upca {
            (Symbology::UpcA, ascii(&digits[1..]))
        } else if ean13 {
            (Symbology::Ean13, ascii(&digits))
        } else {
            return None;
        };
        Some(Self::hit(runs, j, &EAN13, &read, symbology, text, opts))
    }

    fn ean8(runs: &Runs, j: usize, opts: &LineOptions) -> Option<LineHit> {
        if !opts.symbologies.contains(Symbology::Ean8) {
            return None;
        }
        let read = read_layout(runs, j, &EAN8)?;
        if parity_mask(&read.left[..4]) != 0 {
            return None;
        }
        let mut digits = [0u8; 8];
        for i in 0..4 {
            digits[i] = read.left[i].digit;
            digits[4 + i] = read.right[i].digit;
        }
        if !check_digit_ok(&digits) {
            return None;
        }
        Some(Self::hit(
            runs,
            j,
            &EAN8,
            &read,
            Symbology::Ean8,
            ascii(&digits),
            opts,
        ))
    }

    fn upce(runs: &Runs, j: usize, opts: &LineOptions) -> Option<LineHit> {
        if !opts.symbologies.contains(Symbology::UpcE) {
            return None;
        }
        let read = read_layout(runs, j, &UPCE)?;
        let parity = parity_mask(&read.left);
        let (ns, check) = (0..2u8).find_map(|ns| {
            let want = |c: usize| {
                if ns == 0 {
                    UPCE_NS0[c]
                } else {
                    !UPCE_NS0[c] & 0x3f
                }
            };
            (0..10).find(|&c| want(c) == parity).map(|c| (ns, c as u8))
        })?;
        let mut d = [0u8; 6];
        for (slot, m) in d.iter_mut().zip(&read.left) {
            *slot = m.digit;
        }
        let upca = upce_to_upca(ns, &d, check);
        if !check_digit_ok(&upca) {
            return None;
        }
        let mut digits = [0u8; 8];
        digits[0] = ns;
        digits[1..7].copy_from_slice(&d);
        digits[7] = check;
        Some(Self::hit(
            runs,
            j,
            &UPCE,
            &read,
            Symbology::UpcE,
            ascii(&digits),
            opts,
        ))
    }
}

impl LinearDecoder for EanDecoder {
    fn decode(&self, runs: &Runs, opts: &LineOptions, out: &mut Vec<LineHit>) {
        let set = opts.symbologies;
        let want13 = set.contains(Symbology::Ean13) || set.contains(Symbology::UpcA);
        let want8 = set.contains(Symbology::Ean8);
        let want_e = set.contains(Symbology::UpcE);
        if !(want13 || want8 || want_e) {
            return;
        }
        let n = runs.len();
        let mut j = usize::from(n > 0 && !runs.is_bar(0));
        while j + UPCE.total <= n {
            if plausible_start(runs, j) {
                if want13 && j + EAN13.total <= n {
                    out.extend(Self::ean13(runs, j, opts));
                }
                if want8 && j + EAN8.total <= n {
                    out.extend(Self::ean8(runs, j, opts));
                }
                if want_e {
                    out.extend(Self::upce(runs, j, opts));
                }
            }
            j += 2;
        }
    }
}

/// Reads an EAN-5 or EAN-2 supplement whose start guard begins at run `a`.
fn read_addon(runs: &Runs, a: usize, cal: Calibration) -> Option<Vec<u8>> {
    if a == 0 || a + 13 > runs.len() {
        return None;
    }
    let m = cal.end();
    let gap = cal.adjusted(runs, a - 1);
    if gap < ADDON_GAP.0 * m || gap > ADDON_GAP.1 * m {
        return None;
    }
    let near = |idx: usize, modules: f32| {
        (cal.adjusted(runs, idx) - modules * m).abs() <= GUARD_TOLERANCE * m * modules.max(1.0)
    };
    if !(near(a, 1.0) && near(a + 1, 1.0) && near(a + 2, 2.0)) {
        return None;
    }
    [5usize, 2]
        .into_iter()
        .filter(|&count| a + 3 + 6 * count - 2 <= runs.len())
        .find_map(|count| read_addon_digits(runs, a, count, cal))
}

fn read_addon_digits(runs: &Runs, a: usize, count: usize, cal: Calibration) -> Option<Vec<u8>> {
    let mut digits = [0u8; 5];
    let mut g = [false; 5];
    for i in 0..count {
        let at = a + 3 + 6 * i;
        let m = read_digit(runs, at, f32::MAX, cal, Set::LAndG)?;
        if m.weak {
            return None;
        }
        digits[i] = m.digit;
        g[i] = m.g;
        if i + 1 < count {
            for sep in [at + 4, at + 5] {
                if (cal.adjusted(runs, sep) - cal.end()).abs() > GUARD_TOLERANCE * cal.end() {
                    return None;
                }
            }
        }
    }
    let last_bar = a + 3 + 6 * (count - 1) + 3;
    if runs.space_after(last_bar) < ADDON_QUIET * cal.end() {
        return None;
    }
    let g_mask = parity_mask_bool(&g[..count]);
    let valid = if count == 2 {
        let value = u32::from(digits[0]) * 10 + u32::from(digits[1]);
        u32::from(g_mask) == value % 4
    } else {
        let sum =
            3 * u32::from(digits[0] + digits[2] + digits[4]) + 9 * u32::from(digits[1] + digits[3]);
        EAN5_PARITY[(sum % 10) as usize] == g_mask
    };
    valid.then(|| ascii(&digits[..count]))
}

fn parity_mask_bool(g: &[bool]) -> u8 {
    g.iter().fold(0, |acc, &b| (acc << 1) | u8::from(b))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeDetector, EdgeParams, Edges};
    use crate::testutil::*;
    use breader_core::{ScanOptions, SymbologySet};

    fn runs_of(profile: &[u8]) -> Runs {
        runs_with(profile, &EdgeParams::fine())
    }

    fn runs_with(profile: &[u8], params: &EdgeParams) -> Runs {
        let mut edges = Edges::default();
        EdgeDetector::default().detect(profile, params, &mut edges);
        let mut runs = Runs::default();
        Runs::from_edges(&edges, &mut runs);
        runs
    }

    /// Decodes in both reading directions, as the reader does.
    fn decode_with(profile: &[u8], opts: &LineOptions) -> Vec<LineHit> {
        let runs = runs_of(profile);
        let mut rev = Runs::default();
        Runs::reverse_from(&runs, &mut rev);
        let mut out = Vec::new();
        EanDecoder.decode(&runs, opts, &mut out);
        EanDecoder.decode(&rev, opts, &mut out);
        out
    }

    fn opts() -> LineOptions {
        LineOptions::from(&ScanOptions {
            ean_addon: EanAddonPolicy::Optional,
            ..ScanOptions::default()
        })
    }

    fn decode(profile: &[u8]) -> Vec<LineHit> {
        decode_with(profile, &opts())
    }

    fn only(hits: Vec<LineHit>) -> LineHit {
        assert_eq!(hits.len(), 1, "{hits:?}");
        hits.into_iter().next().unwrap()
    }

    fn reversed(p: &[u8]) -> Vec<u8> {
        p.iter().rev().copied().collect()
    }

    #[test]
    fn pattern_tables_are_consistent() {
        for (d, bits) in L_CODE.iter().enumerate() {
            assert_eq!(bits >> 6, 0, "L-codes start with a space");
            assert_eq!(bits & 1, 1, "L-codes end with a bar");
            assert_eq!(bits.count_ones() % 2, 1, "L-codes have odd parity");
            let w = PATTERNS[d];
            assert_eq!(w.iter().sum::<f32>(), 7.0);
            let g = PATTERNS[d + 10];
            assert_eq!(g, [w[3], w[2], w[1], w[0]]);
        }
        for (a, p) in PATTERNS.iter().enumerate() {
            for (b, q) in PATTERNS.iter().enumerate().skip(a + 1) {
                assert_ne!(p, q, "{a} {b}");
            }
        }
        let mut firsts: Vec<_> = EAN13_FIRST.to_vec();
        firsts.sort_unstable();
        firsts.dedup();
        assert_eq!(firsts.len(), 10);
        assert_eq!(UPCE_NS0[0], 0b111000);
        assert_eq!(EAN5_PARITY[0], 0b11000);
    }

    #[test]
    fn check_digit_known_values() {
        for ok in [
            "5901234123457",
            "4006381333931",
            "036000291452",
            "73513537",
            "96385074",
        ] {
            let d: Vec<u8> = ok.bytes().map(|b| b - b'0').collect();
            assert!(check_digit_ok(&d), "{ok}");
        }
        let d: Vec<u8> = "5901234123458".bytes().map(|b| b - b'0').collect();
        assert!(!check_digit_ok(&d));
        assert!(!check_digit_ok(&[]));
    }

    #[test]
    fn upce_expansion_follows_the_suppression_rules() {
        assert_eq!(
            upce_to_upca(0, &[1, 2, 3, 4, 5, 6], 5),
            [0, 1, 2, 3, 4, 5, 0, 0, 0, 0, 6, 5]
        );
        assert_eq!(
            upce_to_upca(0, &[1, 2, 3, 4, 5, 0], 9),
            [0, 1, 2, 0, 0, 0, 0, 0, 3, 4, 5, 9]
        );
        assert_eq!(
            upce_to_upca(1, &[1, 2, 3, 4, 5, 3], 0),
            [1, 1, 2, 3, 0, 0, 0, 0, 0, 4, 5, 0]
        );
        assert_eq!(
            upce_to_upca(0, &[1, 2, 3, 4, 5, 4], 0),
            [0, 1, 2, 3, 4, 0, 0, 0, 0, 0, 5, 0]
        );
    }

    #[test]
    fn ean13_decodes_across_scales_and_directions() {
        let modules = ean13_modules("5901234123457");
        for px in [2.0, 2.5, 3.0, 4.0] {
            for offset in [0.0, 0.3, 0.7] {
                let p = Render {
                    offset,
                    ..Render::px(px)
                }
                .profile(&modules);
                for profile in [p.clone(), reversed(&p)] {
                    let hit = only(decode(&profile));
                    assert_eq!(hit.symbology, Symbology::Ean13, "px {px}");
                    assert_eq!(hit.text, b"5901234123457", "px {px}");
                    assert!(hit.addon.is_none());
                    assert!(
                        (hit.module - px).abs() < 0.35 * px,
                        "{} vs {px}",
                        hit.module
                    );
                }
            }
        }
    }

    #[test]
    fn narrow_modules_decode_when_the_image_is_crisp() {
        let modules = ean13_modules("5901234123457");
        for px in [1.5, 1.7] {
            for offset in [0.0, 0.4, 0.8] {
                let p = Render {
                    offset,
                    sigma: 0.35,
                    ..Render::px(px)
                }
                .profile(&modules);
                let hit = only(decode(&p));
                assert_eq!(hit.text, b"5901234123457", "px {px} offset {offset}");
            }
        }
    }

    #[test]
    fn smoothed_variant_decodes_heavy_noise() {
        let modules = ean13_modules("9780201379624");
        let mut rng = Rng(31);
        let mut ok = 0;
        for _ in 0..20 {
            let mut p = Render {
                offset: rng.unit(),
                ..Render::px(3.5)
            }
            .profile(&modules);
            add_noise(&mut p, 24.0, &mut rng);
            let runs = runs_with(&p, &EdgeParams::smoothed());
            let mut out = Vec::new();
            EanDecoder.decode(&runs, &opts(), &mut out);
            let mut rev = Runs::default();
            Runs::reverse_from(&runs, &mut rev);
            EanDecoder.decode(&rev, &opts(), &mut out);
            assert!(out.iter().all(|h| h.text == b"9780201379624"));
            ok += out.len();
        }
        assert!(ok >= 16, "{ok}");
    }

    #[test]
    fn module_drift_across_the_symbol_is_tolerated() {
        // Perspective: the module grows 40% from the left end to the right.
        let modules = ean13_modules("4006381333931");
        let mut widths = Vec::new();
        let (mut run, mut bar) = (1usize, modules[0]);
        for &m in &modules[1..] {
            if m == bar {
                run += 1;
            } else {
                widths.push((run, bar));
                run = 1;
                bar = m;
            }
        }
        widths.push((run, bar));
        let total = modules.len() as f32;
        let mut pos = 0.0f32;
        let mut dark = Vec::new();
        let mut x = 0.0f32;
        for (len, is_bar) in widths {
            let start = x;
            for _ in 0..len {
                let t = pos / total;
                x += 3.0 * (1.0 + 0.4 * t);
                pos += 1.0;
            }
            if is_bar {
                dark.push((36.0 + start, 36.0 + x));
            }
        }
        let p = render_intervals(&dark, (x + 72.0) as usize, 25.0, 225.0, 0.6);
        let hit = only(decode(&p));
        assert_eq!(hit.text, b"4006381333931");
    }

    /// Run widths (in modules) of an EAN-13 given as module string.
    fn module_widths(modules: &[bool]) -> Vec<f32> {
        let mut widths = Vec::new();
        let mut run = 1.0;
        for pair in modules.windows(2) {
            if pair[0] == pair[1] {
                run += 1.0;
            } else {
                widths.push(run);
                run = 1.0;
            }
        }
        widths.push(run);
        widths
    }

    #[test]
    fn digit_margins_classify_strong_weak_and_ambiguous() {
        // Morph L1 (2,2,2,1) into L2 (2,1,2,2).
        let classify = |t: f32| match_digit([2.0, 2.0 - t, 2.0, 1.0 + t], Set::LAndG, false);
        let strong = classify(0.0).unwrap();
        assert_eq!((strong.digit, strong.g, strong.weak), (1, false, false));
        let weak_t = (0..50)
            .map(|k| k as f32 * 0.01)
            .find(|&t| classify(t).is_some_and(|m| m.weak))
            .expect("some morph state is weak");
        assert!(weak_t < 0.5);
        assert!(classify(0.5).is_none(), "halfway is ambiguous");
        assert_eq!(classify(1.0).unwrap().digit, 2);
    }

    #[test]
    fn only_one_uncertain_digit_is_accepted_per_symbol() {
        let body = "011111122222";
        let text = format!("{body}{}", check_digit(body));
        let widths = module_widths(&ean13_modules(&text));
        assert_eq!(widths.len(), 59);
        let weak_t = (0..50)
            .map(|k| k as f32 * 0.01)
            .find(|&t| {
                match_digit([2.0, 2.0 - t, 2.0, 1.0 + t], Set::LAndG, false).is_some_and(|m| m.weak)
            })
            .unwrap();
        let decode_with_weak = |digits: &[usize]| {
            let mut w: Vec<f32> = widths.iter().map(|m| m * 3.0).collect();
            for &d in digits {
                // Left digit d (0-based) is a '1': runs 3 + 4d.. = (2,2,2,1).
                w[3 + 4 * d + 1] -= weak_t * 3.0;
                w[3 + 4 * d + 3] += weak_t * 3.0;
            }
            let runs = Runs::from_widths(true, &w, 60.0, 60.0);
            let mut out = Vec::new();
            EanDecoder.decode(&runs, &opts(), &mut out);
            out
        };
        assert_eq!(decode_with_weak(&[]).len(), 1);
        let one = decode_with_weak(&[2]);
        assert_eq!(one.len(), 1, "{one:?}");
        assert_eq!(one[0].text, &text.as_bytes()[1..]);
        assert!(decode_with_weak(&[1, 3]).is_empty());
    }

    #[test]
    fn hit_endpoints_match_the_symbol_extent() {
        let modules = ean13_modules("4006381333931");
        let r = Render::px(3.0);
        let p = r.profile(&modules);
        let hit = only(decode(&p));
        let left = r.quiet_left * 3.0;
        assert!((hit.start - left).abs() < 1.0, "{}", hit.start);
        assert!((hit.end - (left + 95.0 * 3.0)).abs() < 1.0, "{}", hit.end);
        let hit = only(decode(&reversed(&p)));
        assert!(hit.start > hit.end, "reading starts at the right end");
    }

    #[test]
    fn upca_is_reported_for_a_leading_zero() {
        let modules = upca_modules("036000291452");
        let p = Render::px(2.5).profile(&modules);
        let hit = only(decode(&p));
        assert_eq!(hit.symbology, Symbology::UpcA);
        assert_eq!(hit.text, b"036000291452");

        let mut o = opts();
        o.symbologies = SymbologySet::EMPTY.with(Symbology::Ean13);
        let hit = only(decode_with(&p, &o));
        assert_eq!(hit.symbology, Symbology::Ean13);
        assert_eq!(hit.text, b"0036000291452");

        o.symbologies = SymbologySet::EMPTY.with(Symbology::UpcA);
        assert!(
            decode_with(
                &Render::px(2.5).profile(&ean13_modules("5901234123457")),
                &o
            )
            .is_empty()
        );
        o.symbologies = SymbologySet::EMPTY.with(Symbology::Ean8);
        assert!(decode_with(&p, &o).is_empty());
    }

    #[test]
    fn ean8_decodes_in_both_directions() {
        let modules = ean8_modules("73513537");
        for px in [2.0, 2.5, 4.0] {
            let p = Render::px(px).profile(&modules);
            for profile in [p.clone(), reversed(&p)] {
                let hit = only(decode(&profile));
                assert_eq!(hit.symbology, Symbology::Ean8);
                assert_eq!(hit.text, b"73513537");
            }
        }
    }

    #[test]
    fn upce_decodes_in_both_number_systems() {
        for text in ["01234565", "12345670"] {
            let body = &text[..7];
            let upca = {
                let d: Vec<u8> = text.bytes().map(|b| b - b'0').collect();
                let a = upce_to_upca(d[0], &d[1..7].try_into().unwrap(), d[7]);
                a.map(|x| b'0' + x)
            };
            let _ = body;
            let upca = String::from_utf8(upca.to_vec()).unwrap();
            let expected_check = check_digit(&upca[..11]);
            let text = format!("{}{}", &text[..7], expected_check);
            let modules = upce_modules(&text);
            let p = Render::px(3.0).profile(&modules);
            for profile in [p.clone(), reversed(&p)] {
                let hit = only(decode(&profile));
                assert_eq!(hit.symbology, Symbology::UpcE);
                assert_eq!(hit.text, text.as_bytes());
            }
        }
    }

    #[test]
    fn check_digit_failures_are_rejected() {
        for bad in ["5901234123458", "5901234123450", "4006381333930"] {
            let p = Render::px(3.0).profile(&ean13_modules(bad));
            assert!(decode(&p).is_empty(), "{bad}");
        }
        let p = Render::px(3.0).profile(&ean8_modules("73513538"));
        assert!(decode(&p).is_empty());
    }

    #[test]
    fn quiet_zones_are_required() {
        let modules = ean13_modules("5901234123457");
        let ok = Render::px(3.0);
        assert_eq!(decode(&ok.profile(&modules)).len(), 1);
        for (l, r) in [(1.0, 12.0), (12.0, 1.0), (2.0, 2.0)] {
            let p = Render {
                quiet_left: l,
                quiet_right: r,
                ..ok
            }
            .profile(&modules);
            assert!(decode(&p).is_empty(), "quiet {l}/{r}");
        }
    }

    #[test]
    fn symbol_embedded_in_bars_has_no_quiet_zone() {
        let mut modules = vec![true, true, false, true, false, false, true];
        modules.extend(ean13_modules("5901234123457"));
        modules.extend([false, true, true]);
        let p = Render::px(3.0).profile(&modules);
        assert!(decode(&p).is_empty());
    }

    #[test]
    fn ean13_inside_has_no_upce_or_ean8() {
        for text in [
            "5901234123457",
            "4006381333931",
            "9780201379624",
            "1234567890128",
        ] {
            let hits = decode(&Render::px(3.0).profile(&ean13_modules(text)));
            assert_eq!(hits.len(), 1, "{text}: {hits:?}");
            assert_eq!(hits[0].symbology, Symbology::Ean13);
        }
    }

    #[test]
    fn modest_blur_gain_and_noise_are_tolerated() {
        let modules = ean13_modules("9780201379624");
        let mut ok = 0;
        let mut total = 0;
        for (sigma, gain, noise) in [
            (0.8, 0.0, 0.0),
            (1.2, 0.0, 0.0),
            (0.6, 0.3, 0.0),
            (0.6, -0.3, 0.0),
            (0.8, 0.0, 6.0),
            (1.0, 0.2, 8.0),
        ] {
            let mut rng = Rng(77);
            for px in [2.5, 3.0, 4.0] {
                for _ in 0..5 {
                    let r = Render {
                        sigma,
                        gain,
                        offset: rng.unit(),
                        ..Render::px(px)
                    };
                    let mut p = r.profile(&modules);
                    add_noise(&mut p, noise, &mut rng);
                    total += 1;
                    let hits = decode(&p);
                    assert!(hits.iter().all(|h| h.text == b"9780201379624"), "{hits:?}");
                    ok += hits.len();
                }
            }
        }
        assert!(ok * 10 >= total * 8, "{ok}/{total}");
    }

    #[test]
    fn addons_are_decoded_and_policy_is_respected() {
        let main = ean13_modules("9780735200449");
        for (addon, gap) in [
            ("12", 9),
            ("52495", 9),
            ("05", 7),
            ("52495", 12),
            ("99", 11),
        ] {
            let modules = with_addon(main.clone(), gap, addon_modules(addon));
            let p = Render::px(3.0).profile(&modules);
            for profile in [p.clone(), reversed(&p)] {
                let hit = only(decode(&profile));
                assert_eq!(hit.text, b"9780735200449");
                assert_eq!(hit.addon.as_deref(), Some(addon.as_bytes()), "{addon}");
            }
            let mut o = opts();
            o.ean_addon = EanAddonPolicy::Ignore;
            let hit = only(decode_with(&p, &o));
            assert_eq!(hit.addon, None);
            assert_eq!(hit.text, b"9780735200449");
        }
    }

    #[test]
    fn addon_on_ean8_and_upce() {
        let modules = with_addon(ean8_modules("73513537"), 9, addon_modules("34"));
        let hit = only(decode(&Render::px(3.0).profile(&modules)));
        assert_eq!(hit.addon.as_deref(), Some(&b"34"[..]));
        let modules = with_addon(upce_modules("01234565"), 9, addon_modules("77777"));
        let hit = only(decode(&Render::px(3.0).profile(&modules)));
        assert_eq!(hit.symbology, Symbology::UpcE);
        assert_eq!(hit.addon.as_deref(), Some(&b"77777"[..]));
    }

    #[test]
    fn invalid_addons_are_ignored() {
        let main = ean13_modules("9780735200449");
        let good = addon_modules("52495");
        let gap = |n: usize, addon: &[bool]| with_addon(main.clone(), n, addon.to_vec());
        let cases: Vec<(&str, Vec<bool>)> = vec![
            (
                "checksum parity",
                gap(9, &addon_modules_with_parity("52495", "GLLGL")),
            ),
            (
                "ean2 parity",
                gap(9, &addon_modules_with_parity("12", "LG")),
            ),
            ("gap too short", gap(3, &good)),
            ("gap too long", gap(25, &good)),
            ("truncated", gap(9, &good[..good.len() - 14])),
            (
                "no quiet zone after",
                gap(9, &[good.clone(), vec![true, false, true]].concat()),
            ),
        ];
        for (name, modules) in cases {
            let hits = decode(&Render::px(3.0).profile(&modules));
            assert!(hits.iter().all(|h| h.addon.is_none()), "{name}: {hits:?}");
        }
        let hits = decode(&Render::px(3.0).profile(&gap(9, &good)));
        assert_eq!(only(hits).addon.as_deref(), Some(&b"52495"[..]));
    }

    #[test]
    fn ambiguous_or_garbage_patterns_are_rejected() {
        let modules = ean13_modules("5901234123457");
        let p = Render {
            sigma: 3.5,
            ..Render::px(2.0)
        }
        .profile(&modules);
        assert!(decode(&p).is_empty());
        let mut rng = Rng(5);
        let mut noisy = Render::px(3.0).profile(&modules);
        add_noise(&mut noisy, 70.0, &mut rng);
        assert!(decode(&noisy).is_empty());
    }

    #[test]
    fn disabled_symbologies_are_not_decoded() {
        let p = Render::px(3.0).profile(&ean13_modules("5901234123457"));
        let mut o = opts();
        o.symbologies = SymbologySet::EMPTY.with(Symbology::Code128);
        assert!(decode_with(&p, &o).is_empty());
        o.symbologies = SymbologySet::EMPTY.with(Symbology::Ean13);
        assert_eq!(decode_with(&p, &o).len(), 1);
    }

    #[test]
    fn many_random_valid_symbols_round_trip() {
        let mut rng = Rng(2024);
        for i in 0..150 {
            let px = 2.2 + rng.unit() * 1.8;
            let r = Render {
                offset: rng.unit(),
                sigma: 0.4 + 0.4 * rng.unit(),
                ..Render::px(px)
            };
            let (modules, expected, sym) = match i % 3 {
                0 => {
                    let body = rng.digits(12);
                    let text = format!("{body}{}", check_digit(&body));
                    match text.strip_prefix('0') {
                        Some(upca) => (ean13_modules(&text), upca.to_string(), Symbology::UpcA),
                        None => (ean13_modules(&text), text, Symbology::Ean13),
                    }
                }
                1 => {
                    let body = rng.digits(7);
                    let text = format!("{body}{}", check_digit(&body));
                    (ean8_modules(&text), text, Symbology::Ean8)
                }
                _ => {
                    let body = format!("0{}", rng.digits(10));
                    let text = format!("{body}{}", check_digit(&body));
                    (upca_modules(&text), text, Symbology::UpcA)
                }
            };
            let mut p = r.profile(&modules);
            if i % 2 == 1 {
                p.reverse();
            }
            let hits = decode(&p);
            assert_eq!(hits.len(), 1, "{expected} px {px}: {hits:?}");
            assert_eq!(hits[0].text, expected.as_bytes());
            assert_eq!(hits[0].symbology, sym);
        }
    }
}
