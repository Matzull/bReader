//! Start and stop patterns of PDF417 on scanline runs. The linear scan
//! records them for the PDF417 reader, which then needs no scan of its own.

use breader_core::{FrameCache, Point};

use crate::plan::Scanline;
use crate::runs::Runs;

const START: [f32; 8] = [8.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 3.0];
const STOP: [f32; 9] = [7.0, 1.0, 1.0, 3.0, 1.0, 1.0, 1.0, 2.0, 1.0];

/// Narrowest module, in samples, that is trusted.
const MIN_MODULE: f32 = 0.8;
/// Quiet zone next to a pattern, in modules (the specification asks for 2).
const QUIET: f32 = 1.4;
/// Smallest margin to the profile border that counts as a quiet zone that
/// the border cuts off.
const OPEN_MARGIN: f32 = 1.0;
/// The first element must be this many times wider than the next two.
const WIDE: f32 = 2.5;

/// Which end of a symbol's rows a pattern marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkKind {
    Start,
    Stop,
}

/// A start or stop pattern found on a profile, in profile positions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RawMark {
    pub kind: MarkKind,
    /// The first element of the pattern is light: a light-on-dark symbol.
    pub inverted: bool,
    /// Edge where the pattern begins in pattern order, and where it ends.
    /// `to < from` when the profile runs against the symbol's row direction.
    pub from: f32,
    pub to: f32,
    /// Samples per module along the profile.
    pub module: f32,
    /// Index of the pattern's lowest run.
    pub run: usize,
}

/// A pattern in frame coordinates. `to - from` points along the symbol's
/// rows, from the start side to the stop side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    pub kind: MarkKind,
    pub inverted: bool,
    pub from: Point,
    pub to: Point,
    /// Samples per module along the scanline that saw the pattern.
    pub module: f32,
}

/// Marks of one frame, shared through the frame context.
#[derive(Debug, Default)]
pub struct MarkSet {
    pub marks: Vec<Mark>,
    /// A scan that records marks ran over this frame.
    pub scanned: bool,
}

impl FrameCache for MarkSet {
    fn reset(&mut self) {
        self.marks.clear();
        self.scanned = false;
    }
}

fn tolerance(nominal: f32) -> f32 {
    0.4 + 0.08 * nominal
}

/// Module width when `w` matches `pattern` (or its reverse), else `None`.
fn fits(w: &[f32], pattern: &[f32], reversed: bool) -> Option<f32> {
    let n = pattern.len();
    let total: f32 = w.iter().sum();
    let module = total / pattern.iter().sum::<f32>();
    if module < MIN_MODULE {
        return None;
    }
    for (k, &width) in w.iter().enumerate() {
        let nominal = if reversed {
            pattern[n - 1 - k]
        } else {
            pattern[k]
        };
        if (width - nominal * module).abs() > tolerance(nominal) * module {
            return None;
        }
    }
    Some(module)
}

/// Finds the patterns in `runs`, read in both directions and in both
/// polarities.
pub fn find(runs: &Runs, out: &mut Vec<RawMark>) {
    let w = runs.widths();
    let n = w.len();
    if n < START.len() {
        return;
    }
    at_borders(runs, out);
    for i in 0..n {
        let wi = w[i];
        if i + 2 < n && wi > WIDE * w[i + 1] && wi > WIDE * w[i + 2] {
            forward(runs, i, out);
        }
        if i >= 2 && wi > WIDE * w[i - 1] && wi > WIDE * w[i - 2] {
            backward(runs, i, out);
        }
    }
}

/// Patterns that begin or end at the border of the profile, where the margin
/// is the first or last element: a symbol cropped without a quiet zone. The
/// border counts as the quiet zone. `run` is 0 for these.
fn at_borders(runs: &Runs, out: &mut Vec<RawMark>) {
    let w = runs.widths();
    let n = w.len();
    let (lead, trail) = (runs.lead(), runs.trail());
    // The margin before the first run is a run of the opposite colour.
    let first_is_light_margin = runs.is_bar(0);
    let mut head = [0f32; 9];
    head[0] = lead;
    head[1..].copy_from_slice(&w[..8]);
    let head_start = runs.start(0) - lead;
    let mut push = |kind, module: f32, from: f32, to: f32, inverted: bool| {
        out.push(RawMark {
            kind,
            inverted,
            from,
            to,
            module,
            run: 0,
        });
    };
    // A start pattern forward, or a stop pattern reversed, from the margin.
    let inverted = first_is_light_margin;
    if lead >= OPEN_MARGIN {
        if let Some(m) = fits(&head[..8], &START, false) {
            push(MarkKind::Start, m, head_start, runs.end(6), inverted);
        }
        if let Some(m) = fits(&head, &STOP, true) {
            push(MarkKind::Stop, m, runs.end(7), head_start, inverted);
        }
    }
    // A start pattern reversed, or a stop pattern forward, up to the margin.
    let mut tail = [0f32; 9];
    tail[8] = trail;
    tail[..8].copy_from_slice(&w[n - 8..]);
    let tail_end = runs.end(n - 1) + trail;
    let inverted = runs.is_bar(n - 1);
    if trail >= OPEN_MARGIN {
        if let Some(m) = fits(&tail[1..], &START, true) {
            push(MarkKind::Start, m, tail_end, runs.start(n - 7), inverted);
        }
        if let Some(m) = fits(&tail, &STOP, false) {
            push(MarkKind::Stop, m, runs.start(n - 8), tail_end, inverted);
        }
    }
}

fn quiet_before(runs: &Runs, i: usize, module: f32) -> bool {
    if i == 0 {
        runs.lead() >= OPEN_MARGIN
    } else {
        runs.widths()[i - 1] >= QUIET * module
    }
}

fn quiet_after(runs: &Runs, i: usize, module: f32) -> bool {
    if i + 1 == runs.len() {
        runs.trail() >= OPEN_MARGIN
    } else {
        runs.widths()[i + 1] >= QUIET * module
    }
}

fn forward(runs: &Runs, i: usize, out: &mut Vec<RawMark>) {
    let w = runs.widths();
    for (kind, pattern) in [(MarkKind::Start, &START[..]), (MarkKind::Stop, &STOP[..])] {
        let last = i + pattern.len() - 1;
        if last >= w.len() {
            continue;
        }
        let Some(module) = fits(&w[i..=last], pattern, false) else {
            continue;
        };
        let outer_ok = match kind {
            MarkKind::Start => quiet_before(runs, i, module),
            MarkKind::Stop => quiet_after(runs, last, module),
        };
        if outer_ok {
            out.push(RawMark {
                kind,
                inverted: !runs.is_bar(i),
                from: runs.start(i),
                to: runs.end(last),
                module,
                run: i,
            });
        }
    }
}

fn backward(runs: &Runs, i: usize, out: &mut Vec<RawMark>) {
    let w = runs.widths();
    for (kind, pattern) in [(MarkKind::Start, &START[..]), (MarkKind::Stop, &STOP[..])] {
        let Some(first) = (i + 1).checked_sub(pattern.len()) else {
            continue;
        };
        let Some(module) = fits(&w[first..=i], pattern, true) else {
            continue;
        };
        let outer_ok = match kind {
            MarkKind::Start => quiet_after(runs, i, module),
            MarkKind::Stop => quiet_before(runs, first, module),
        };
        if outer_ok {
            out.push(RawMark {
                kind,
                inverted: !runs.is_bar(i),
                from: runs.end(i),
                to: runs.start(first),
                module,
                run: first,
            });
        }
    }
}

/// Appends the patterns of `runs` (the profile of `line`) to `out`.
pub(crate) fn record(
    runs: &Runs,
    line: &Scanline,
    scratch: &mut Vec<RawMark>,
    out: &mut Vec<Mark>,
) {
    scratch.clear();
    find(runs, scratch);
    out.extend(scratch.iter().map(|m| Mark {
        kind: m.kind,
        inverted: m.inverted,
        from: line.point_at(m.from),
        to: line.point_at(m.to),
        module: m.module,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs_of(widths: &[f32], first_is_bar: bool) -> Runs {
        Runs::from_widths(first_is_bar, widths, 12.0, 12.0)
    }

    fn scaled(pattern: &[f32], m: f32) -> Vec<f32> {
        pattern.iter().map(|p| p * m).collect()
    }

    #[test]
    fn forward_start_after_a_quiet_zone() {
        // A light quiet zone, the pattern from a bar, then the first codeword.
        let mut w = vec![18.0];
        w.extend(scaled(&START, 2.0));
        w.extend([10.0, 4.0, 6.0]);
        let runs = runs_of(&w, false);
        let mut out = Vec::new();
        find(&runs, &mut out);
        assert_eq!(out.len(), 1);
        let m = out[0];
        assert_eq!(m.kind, MarkKind::Start);
        assert!(!m.inverted);
        assert!((m.module - 2.0).abs() < 0.05);
        assert!(m.to > m.from);
        assert!((m.to - m.from - 34.0).abs() < 0.1);
    }

    #[test]
    fn reversed_stop_reads_against_the_profile() {
        let mut w = vec![30.0];
        let mut stop = scaled(&STOP, 1.5);
        stop.reverse();
        w.extend(stop);
        w.extend([30.0, 3.0]);
        let runs = runs_of(&w, false);
        let mut out = Vec::new();
        find(&runs, &mut out);
        let stops: Vec<_> = out.iter().filter(|m| m.kind == MarkKind::Stop).collect();
        assert_eq!(stops.len(), 1);
        assert!(stops[0].to < stops[0].from);
        assert!((stops[0].module - 1.5).abs() < 0.05);
    }

    #[test]
    fn light_on_dark_is_flagged() {
        // The first run is a bar, so the wide run of the pattern is light.
        let mut w = vec![20.0];
        w.extend(scaled(&START, 2.0));
        w.extend([8.0, 3.0]);
        let runs = runs_of(&w, true);
        let mut out = Vec::new();
        find(&runs, &mut out);
        let inverted: Vec<_> = out.iter().filter(|m| m.inverted).collect();
        assert_eq!(inverted.len(), 1);
    }

    #[test]
    fn missing_quiet_zone_or_wrong_widths_are_rejected() {
        let mut w = vec![1.0];
        w.extend(scaled(&START, 2.0));
        w.extend([8.0, 3.0]);
        let mut out = Vec::new();
        find(&runs_of(&w, false), &mut out);
        assert!(out.is_empty());
        let mut w = vec![40.0];
        let mut bad = scaled(&START, 2.0);
        bad[3] = 6.0;
        w.extend(bad);
        w.extend([8.0, 3.0]);
        find(&runs_of(&w, false), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn patterns_cut_by_the_border_use_the_margin_as_an_element() {
        // The profile starts inside the 8-module bar: lead is its width.
        let mut w = scaled(&START[1..], 2.0);
        w.extend([6.0, 4.0, 6.0, 4.0]);
        let runs = Runs::from_widths(false, &w, 16.0, 12.0);
        let mut out = Vec::new();
        find(&runs, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].kind, MarkKind::Start);
        assert!(!out[0].inverted);
        assert!((out[0].module - 2.0).abs() < 0.05);
        assert!(out[0].from.abs() < 0.01 && (out[0].to - 34.0).abs() < 0.1);
        // The stop pattern ends at the other border, bar cut at the margin.
        let mut w = vec![4.0, 6.0, 4.0, 6.0];
        w.extend(scaled(&STOP[..8], 2.0));
        let runs = Runs::from_widths(true, &w, 12.0, 2.0);
        let mut out = Vec::new();
        find(&runs, &mut out);
        assert_eq!(out.len(), 1, "{out:?}");
        assert_eq!(out[0].kind, MarkKind::Stop);
        assert!(out[0].to > out[0].from);
    }

    #[test]
    fn ordinary_barcodes_have_no_marks() {
        let w: Vec<f32> = (0..200).map(|i| 2.0 + (i * 7 % 5) as f32).collect();
        let mut out = Vec::new();
        find(&runs_of(&w, true), &mut out);
        assert!(out.is_empty());
    }
}
