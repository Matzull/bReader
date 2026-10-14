//! GS1 DataBar (ISO/IEC 24724): Omnidirectional and Truncated, Limited and
//! Expanded in one row.
//!
//! The symbols are width-modulated at module level: every element is a
//! whole number of modules and the pieces of a symbol (data characters,
//! finder patterns, guards) have fixed module counts. The decoders measure a
//! module per piece, quantize all elements, and accept a symbol only when
//! every character is a member of its combination set, the checksum
//! (mod 79, 89 or 211) holds and, for Omnidirectional and Limited, the
//! finder or check pattern agrees with it.

mod expanded;
mod limited;
mod message;
mod omni;
mod quantize;
mod rank;
mod rows;
pub(crate) mod stacked;
#[cfg(test)]
mod stacked_tests;

use breader_core::Symbology;

use crate::common::{quiet_after_open, quiet_before_open};
use crate::decoder::{LineOptions, LinearDecoder};
use crate::runs::Runs;

/// Elements between the quiet zones, from the first bar guard on, of
/// Omnidirectional and Limited symbols.
const ROW_RUNS: usize = 45;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DataBarDecoder;

impl LinearDecoder for DataBarDecoder {
    fn decode(
        &self,
        runs: &Runs,
        opts: &LineOptions,
        out: &mut Vec<crate::decoder::LineHit>,
    ) -> bool {
        let omni = opts.symbologies.contains(Symbology::DataBar);
        let limited = opts.symbologies.contains(Symbology::DataBarLimited);
        let expanded = opts.symbologies.contains(Symbology::DataBarExpanded);
        if !(omni || limited || expanded) || runs.is_empty() {
            return false;
        }
        let n = runs.len() as isize;
        let mut started = expanded && expanded::decode(runs, out);
        // Rows of the stacked symbols; the aggregator joins them.
        started |= expanded && rows::decode(runs, out);
        started |= omni && halves(runs, out);
        // A bar guard cut by the start of the profile is a run of its own
        // that the edges do not show: window -1.
        let mut j: isize = if runs.is_bar(0) { 0 } else { -1 };
        while (omni || limited) && j + ROW_RUNS as isize <= n + 1 {
            let (mut maybe_omni, mut maybe_limited) = screen(runs.widths(), j);
            if j >= 0 && j as usize + ROW_RUNS <= runs.len() {
                // Whole windows: the piece test of the decoders on positions
                // alone drops nearly every window of a busy line.
                let at = j as usize;
                maybe_omni = maybe_omni && omni && omni::pieces_plausible(runs, at);
                // With a single edge variant (Fast) nothing reads whether a
                // shape looked like a symbol, so windows without finders skip.
                maybe_omni =
                    maybe_omni && (!opts.single_variant || omni::finders_plausible(runs, at));
                maybe_limited = maybe_limited && limited && limited::pieces_plausible(runs, at);
            }
            if (omni && maybe_omni || limited && maybe_limited)
                && let Some(row) = Row::at(runs, j)
            {
                if omni && maybe_omni {
                    started |= omni::decode(&row, out);
                }
                if limited && maybe_limited {
                    started |= limited::decode(&row, out);
                }
            }
            j += 2;
        }
        started
    }
}

/// Top and bottom rows of Stacked and Stacked Omnidirectional symbols. A
/// bar guard cut by a border of the profile is run -1 or run `runs.len()`.
fn halves(runs: &Runs, out: &mut Vec<crate::decoder::LineHit>) -> bool {
    let n = runs.len() as isize;
    let mut started = false;
    let mut j: isize = if runs.is_bar(0) { 0 } else { -1 };
    while j + omni::TOP_RUNS as isize <= n + 1 {
        for top in [true, false] {
            if omni::half_plausible(runs, j, top) {
                started |= omni::decode_half(runs, j, top, out);
            }
        }
        j += 2;
    }
    started
}

/// Cheap tests on the runs at `j` that reject most windows of a busy line:
/// every finder of Omnidirectional ends, and the right one starts, with two
/// narrow elements; the check pattern of Limited has no element wider than
/// three modules. Windows with a cut guard are not screened.
fn screen(w: &[f32], j: isize) -> (bool, bool) {
    if j < 0 || j as usize + ROW_RUNS > w.len() {
        return (true, true);
    }
    let w = &w[j as usize..j as usize + ROW_RUNS];
    let g = w[0];
    let narrow = |v: f32| (0.4 * g..=2.5 * g).contains(&v);
    let omni = narrow(w[12]) && narrow(w[13]) && narrow(w[30]) && narrow(w[31]);
    let limited = w[15..29].iter().all(|&v| (0.4 * g..=4.0 * g).contains(&v));
    (omni, limited)
}

/// The checks that open `omni::decode` and `limited::decode`, on a whole
/// window of runs at `j` without building its [`Row`]: the module from the
/// span, and the light areas before and after as `Row::at` measures them.
fn opening_ok(runs: &Runs, j: usize, span_modules: f32, quiet: (f32, f32)) -> bool {
    opening_ok_n(runs, j, ROW_RUNS, span_modules, quiet)
}

/// [`opening_ok`] for a window of `len` runs.
fn opening_ok_n(runs: &Runs, j: usize, len: usize, span_modules: f32, quiet: (f32, f32)) -> bool {
    let last = j + len - 1;
    let span = (runs.end(last) - runs.start(j)).abs();
    let module = span / span_modules;
    if module < crate::common::MIN_MODULE {
        return false;
    }
    let before = if j == 0 {
        f32::INFINITY
    } else {
        quiet_before_open(runs, j, 0.0)
    };
    let after = if last + 1 == runs.len() {
        f32::INFINITY
    } else {
        quiet_after_open(runs, last, 0.0)
    };
    !(before < quiet.0 * module || after < quiet.1 * module)
}

/// The 45 runs of a symbol of 46 elements, without the guard space that
/// merges with the quiet zone.
struct Row {
    w: [f32; ROW_RUNS],
    start: f32,
    end: f32,
    /// Profile samples between the first and the last bar edge.
    span: f32,
    /// Light area before the first and after the last bar; infinite where
    /// the profile border cut a guard bar.
    quiet_before: f32,
    quiet_after: f32,
}

impl Row {
    /// Collects the runs from `j` on and screens the three runs that are one
    /// module wide in every variant: the bar guard at each end and the space
    /// guard before the last bar. They are alike within the range blur and
    /// print gain allow. Run -1 and run `runs.len()` stand for a guard bar
    /// that the border of the profile cut: its width is the distance from
    /// the border, which is only trusted as an upper bound, so it takes the
    /// width of the other guard.
    fn at(runs: &Runs, j: isize) -> Option<Self> {
        let n = runs.len() as isize;
        let last = j + ROW_RUNS as isize - 1;
        let first_cut = j < 0;
        let last_cut = last >= n;
        if (first_cut && runs.lead() < 1.0)
            || (last_cut && (runs.trail() < 1.0 || runs.is_bar(runs.len() - 1)))
        {
            return None;
        }
        let real = |k: isize| runs.widths()[k as usize];
        let (a, b) = (if first_cut { None } else { Some(real(j)) }, real(last - 1));
        let c = if last_cut { None } else { Some(real(last)) };
        let (a, c) = match (a, c) {
            (Some(a), Some(c)) => (a, c),
            (Some(a), None) => (a, a),
            (None, Some(c)) => (c, c),
            (None, None) => (b, b),
        };
        let (lo, hi) = (a.min(b).min(c), a.max(b).max(c));
        if !(lo > 0.0 && hi <= 2.2 * lo) {
            return None;
        }
        // No element of either symbology is narrower than a third or wider
        // than ten guard widths; most windows of a busy line fail here.
        let g = (a + b + c) / 3.0;
        let first_real = j.max(0) as usize;
        let last_real = (last.min(n - 1)) as usize;
        if runs.widths()[first_real..=last_real]
            .iter()
            .any(|&w| !(0.33 * g..=10.0 * g).contains(&w))
        {
            return None;
        }
        // Positions decrease along reversed runs.
        let sign = if runs.end(0) >= runs.start(0) {
            1.0
        } else {
            -1.0
        };
        let start = if first_cut {
            runs.start(0) - sign * a
        } else {
            runs.start(j as usize)
        };
        let end = if last_cut {
            runs.end(runs.len() - 1) + sign * c
        } else {
            runs.end(last as usize)
        };
        let span = (end - start).abs();
        if !(span.is_finite() && span > 0.0) {
            return None;
        }
        let mut w = [0f32; ROW_RUNS];
        for (i, v) in w.iter_mut().enumerate() {
            let k = j + i as isize;
            *v = if k < 0 {
                a
            } else if k >= n {
                c
            } else {
                real(k)
            };
        }
        // A symbol that starts or ends at the border of the profile has no
        // room for the guard's own space.
        let quiet_before = if first_cut || j == 0 {
            f32::INFINITY
        } else {
            quiet_before_open(runs, j as usize, 0.0)
        };
        let quiet_after = if last_cut || last + 1 == n {
            f32::INFINITY
        } else {
            quiet_after_open(runs, last as usize, 0.0)
        };
        Some(Self {
            w,
            start,
            end,
            span,
            quiet_before,
            quiet_after,
        })
    }
}

pub(super) fn reversed<const N: usize>(src: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    for (o, &s) in out.iter_mut().zip(src.iter().rev()) {
        *o = s;
    }
    out
}

pub(super) fn array<const N: usize>(src: &[u8]) -> [u8; N] {
    let mut out = [0u8; N];
    out.copy_from_slice(src);
    out
}

/// GTIN-14 element string `01...` from the 13-digit body.
pub(super) fn element_string(body: u64) -> Vec<u8> {
    let mut digits = [b'0'; 14];
    let mut v = body;
    for d in digits[..13].iter_mut().rev() {
        *d = b'0' + (v % 10) as u8;
        v /= 10;
    }
    let sum: u32 = digits[..13]
        .iter()
        .enumerate()
        .map(|(i, &d)| u32::from(d - b'0') * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    digits[13] = b'0' + ((10 - sum % 10) % 10) as u8;
    let mut text = Vec::with_capacity(16);
    text.extend_from_slice(b"01");
    text.extend_from_slice(&digits);
    text
}
