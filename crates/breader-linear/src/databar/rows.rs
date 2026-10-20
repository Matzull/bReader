//! One row of GS1 DataBar Expanded Stacked.
//!
//! A row is a bar guard, up to 11 blocks of 21 elements (character,
//! finder, character: 49 modules) and a guard; the row of an odd-sized
//! symbol that ends the symbol has a character and a finder (13 elements)
//! as its last block. Rows are read in the direction in which a block always
//! starts with a character as printed and its second character is printed
//! reversed. The finder of a block is one of the six patterns, as printed
//! for even finder positions of the whole symbol and reversed for odd ones,
//! so the finders of a row alternate. Which patterns, the checksum and the
//! symbol size are only known from all rows: the aggregator joins them
//! ([`super::stacked`]).
//!
//! The guard bars at the ends of a row may be cut by the border of the
//! profile (symbols cropped to their modules): run -1 and run `runs.len()`
//! stand for them, with the width of a module.

use breader_core::Symbology;

use super::expanded::{FINDERS, char_value, spec_order};
use super::quantize::{MAX_ELEMENTS, Segment, quantize};
use crate::common::{MIN_MODULE, quiet_after_open, quiet_before_open};
use crate::decoder::LineHit;
use crate::runs::Runs;

/// Tag of the part text: `[PART_ROW, o, elements...]`; bit 0 of `o` is the
/// orientation (0 as printed, 1 reversed) of the first finder, bit 1 tells
/// that the region starts with a bar (a guard space precedes it).
pub(super) const PART_ROW: u8 = 3;
/// Blocks of a row (columns of the symbol).
pub(super) const MAX_BLOCKS: usize = 11;
/// Elements of a whole block and of the half block that ends an odd symbol.
pub(super) const BLOCK: usize = 21;
pub(super) const HALF_BLOCK: usize = 13;

const TOLERANCE: f32 = 0.5;
const PIECE_SLACK: f32 = 0.25;
const QUIET: f32 = 0.8;

/// The runs with the virtual guard bars of a cut profile.
pub(super) struct View<'a> {
    pub runs: &'a Runs,
    /// Width of a virtual guard bar: the module of the row.
    pub guard: f32,
    /// Positions decrease along reversed runs.
    sign: f32,
}

impl<'a> View<'a> {
    pub fn new(runs: &'a Runs, guard: f32) -> Self {
        let sign = if runs.end(0) >= runs.start(0) {
            1.0
        } else {
            -1.0
        };
        Self { runs, guard, sign }
    }

    pub fn width(&self, k: isize) -> f32 {
        self.runs
            .widths()
            .get(usize::try_from(k).unwrap_or(usize::MAX))
            .copied()
            .unwrap_or(self.guard)
    }

    pub fn start(&self, k: isize) -> f32 {
        match usize::try_from(k) {
            Ok(k) => self.runs.start(k),
            Err(_) => self.runs.start(0) - self.sign * self.guard,
        }
    }

    pub fn end(&self, k: isize) -> f32 {
        if k >= self.runs.len() as isize {
            self.runs.end(self.runs.len() - 1) + self.sign * self.guard
        } else {
            self.runs.end(k as usize)
        }
    }
}

/// Scans `runs` for rows; true when a row was read.
pub(super) fn decode(runs: &Runs, out: &mut Vec<LineHit>) -> bool {
    let n = runs.len() as isize;
    if n == 0 {
        return false;
    }
    let mut started = false;
    // A bar guard cut by the start of the profile is run -1.
    let mut j: isize = if runs.is_bar(0) { 0 } else { -1 };
    while j + 15 <= n {
        for lead in [1, 2] {
            started |= window(runs, j, lead, out);
        }
        j += 2;
    }
    started
}

/// The row that starts with the bar guard at run `j`. The region of blocks
/// starts `lead` runs on: right after the guard bar with a space, as the
/// first row of a symbol is printed, or after a guard space with a bar (the
/// even rows of a symbol of an odd number of columns, which are printed in
/// the opposite colours).
fn window(runs: &Runs, j: isize, lead: isize, out: &mut Vec<LineHit>) -> bool {
    let w = runs.widths();
    let first = (j + lead) as usize;
    if j < 0 && runs.lead() < 1.0 {
        return false;
    }
    // Every finder has two narrow elements at one end,
    let g = if j < 0 {
        w[first..first + 13].iter().sum::<f32>() / 32.0
    } else {
        w[j as usize]
    };
    let narrow = |v: f32| (0.4 * g..=2.5 * g).contains(&v);
    // and the element before them is at least four modules wide.
    if !(w[first + 10] >= 2.8 * g
        && ((narrow(w[first + 11]) && narrow(w[first + 12]))
            || (narrow(w[first + 8]) && narrow(w[first + 9]))))
    {
        return false;
    }
    // Pieces measured on run positions (the first block: character and
    // finder, 32 modules).
    let span = |a: usize, b: usize| (runs.start(b) - runs.start(a)).abs();
    let module = span(first, first + 13) / 32.0;
    let within = |a: usize, b: usize, modules: f32| {
        (span(a, b) - modules * module).abs() <= PIECE_SLACK * modules * module
    };
    let view = View::new(runs, module);
    let guard_ok = |k: isize| (0.5 * module..=2.2 * module).contains(&view.width(k));
    if !(module >= MIN_MODULE
        && guard_ok(j)
        && (lead == 1 || guard_ok(j + 1))
        && within(first, first + 8, 17.0)
        && within(first + 8, first + 13, 15.0)
        && (j <= 0 || quiet_before_open(runs, j as usize, 0.0) >= QUIET * module))
    {
        return false;
    }
    // The first finder must be near one of the patterns, printed or reversed.
    if !finder_plausible(&w[first + 8..first + 13]) {
        return false;
    }
    // A row ends where its last block does. Narrow bars inside the row also
    // pass for a guard, so only the longest end that has a guard and a light
    // area after it is tried.
    let mut end: Option<usize> = None;
    let mut pos = first;
    for _ in 0..MAX_BLOCKS {
        if pos + 13 > w.len()
            || !within(pos, pos + 8, 17.0)
            || !within(pos + 8, pos + 13, 15.0)
            || !finder_plausible(&w[pos + 8..pos + 13])
        {
            break;
        }
        if end_fits(&view, j, lead, pos + 13, module) {
            end = Some(pos + 13);
        }
        if pos + 21 > w.len() || !within(pos + 13, pos + 21, 17.0) {
            break;
        }
        pos += 21;
        if end_fits(&view, j, lead, pos, module) {
            end = Some(pos);
        }
    }
    end.is_some_and(|e| read(&view, j, lead, e, out))
}

/// Five elements scaled to 15 modules lie near one of the six finder
/// patterns, printed or reversed (tolerance as for the Omnidirectional
/// finders: quantization limit, print gain and rescaling).
fn finder_plausible(e: &[f32]) -> bool {
    let sum: f32 = e.iter().sum();
    if !(sum.is_finite() && sum > 0.0) {
        return false;
    }
    let scale = 15.0 / sum;
    let near = |x: f32, m: u8| (x * scale - f32::from(m)).abs() <= 1.0 + 0.12 * f32::from(m);
    FINDERS
        .iter()
        .any(|f| (0..5).all(|i| near(e[i], f[i])) || (0..5).all(|i| near(e[i], f[4 - i])))
}

/// Runs of the tail guards of a row whose region ends before run `e`: the
/// guard bar alone when run `e` is a bar (its space merges with the quiet
/// zone), otherwise a space and the bar.
fn tail_runs(j: isize, e: usize) -> usize {
    if (e as isize - j) % 2 == 0 { 1 } else { 2 }
}

/// Guards and light area of the row from the guard bar at `j` to the region
/// that ends before run `e`; the module must be the one of the row's head.
fn end_fits(v: &View<'_>, j: isize, lead: isize, e: usize, head_module: f32) -> bool {
    let n = v.runs.len();
    let tail = tail_runs(j, e);
    let last = e + tail - 1;
    // The last bar may be cut by the end of the profile: run `n`.
    let cut = last == n;
    if last > n || (cut && (v.runs.trail() < 1.0 || v.runs.is_bar(n - 1))) {
        return false;
    }
    let m = (e as isize - j - lead) as usize;
    let module = (v.end(last as isize) - v.start(j)).abs() / row_modules(m, lead as usize, tail);
    if !(module >= MIN_MODULE && (module / head_module - 1.0).abs() <= 0.25) {
        return false;
    }
    let guard_ok = [j, e as isize, last as isize]
        .iter()
        .all(|&k| (0.5 * module..=2.2 * module).contains(&v.width(k)));
    let quiet_after = if cut || last + 1 == n {
        f32::INFINITY
    } else {
        quiet_after_open(v.runs, last, 0.0)
    };
    guard_ok && quiet_after >= QUIET * module
}

/// Reads the row from the guard at `j` to the region that ends before run
/// `e`, followed by the right guard.
fn read(v: &View<'_>, j: isize, lead: isize, e: usize, out: &mut Vec<LineHit>) -> bool {
    let m = (e as isize - j - lead) as usize;
    let tail = tail_runs(j, e);
    let last = e + tail - 1;
    let (start, end) = (v.start(j), v.end(last as isize));
    let module = (end - start).abs() / row_modules(m, lead as usize, tail);
    let len = (last as isize - j + 1) as usize;
    let lead = lead as usize;
    let mut widths = [0f32; MAX_ELEMENTS];
    for (i, x) in widths[..len].iter_mut().enumerate() {
        *x = v.width(j + i as isize);
    }
    let mut segments = [Segment::new(0, 0, 0); 2 + 3 * MAX_BLOCKS + 2];
    let mut count = 0;
    for g in 0..lead {
        segments[count] = Segment::new(g, 1, 1);
        count += 1;
    }
    let mut at = lead;
    while at < lead + m {
        segments[count] = Segment::new(at, 8, 17);
        segments[count + 1] = Segment::new(at + 8, 5, 15);
        count += 2;
        at += 13;
        if at < lead + m {
            segments[count] = Segment::new(at, 8, 17);
            count += 1;
            at += 8;
        }
    }
    for g in at..len {
        segments[count] = Segment::new(g, 1, 1);
        count += 1;
    }
    let mut mods = [0u8; MAX_ELEMENTS];
    let Some(q) = quantize(
        &widths[..len],
        true,
        &segments[..count],
        module,
        TOLERANCE,
        &mut mods[..len],
    ) else {
        return false;
    };
    let region = &mods[lead..lead + m];
    let Some(orientation) = check_region(region) else {
        return true;
    };
    let mut text = crate::pool::take(2 + m);
    text.push(PART_ROW);
    text.push(orientation | (((lead - 1) as u8) << 1));
    text.extend_from_slice(region);
    out.push(LineHit {
        symbology: Symbology::DataBarExpanded,
        text,
        addon: None,
        start,
        end,
        module,
        quality: (1.0 - q.deviation / 0.5).clamp(0.0, 1.0),
        aim: *b"]e0",
        checksum_verified: false,
        short_quiet: false,
        inverted: false,
        start_stop: None,
        segments: 0,
        thin: false,
        part: true,
    });
    true
}

/// Modules from the first to the last bar edge: the guard bar (and space),
/// the region of `m` elements (17 per character, 15 per finder) and the tail
/// guards.
fn row_modules(m: usize, lead: usize, tail: usize) -> f32 {
    let blocks = m / BLOCK;
    let (chars, finders) = match m % BLOCK {
        0 => (2 * blocks, blocks),
        _ => (2 * blocks + 1, blocks + 1),
    };
    (lead + 17 * chars + 15 * finders + tail) as f32
}

/// Every character of the region is a member of its set and the finders are
/// among the six patterns, alternating between printed and reversed; the
/// orientation of the first finder (0 printed).
fn check_region(region: &[u8]) -> Option<u8> {
    let mut orientation: Option<u8> = None;
    let mut at = 0;
    let mut p = 0usize;
    while at < region.len() {
        let c: [u8; 8] = region[at..at + 8].try_into().ok()?;
        char_value(&spec_order(&c, 0))?;
        let f: [u8; 5] = region[at + 8..at + 13].try_into().ok()?;
        let o = finder_orientation(&f)?;
        let first = *orientation.get_or_insert(o);
        if o != first ^ (p % 2) as u8 {
            return None;
        }
        at += 13;
        if at < region.len() {
            let c: [u8; 8] = region[at..at + 8].try_into().ok()?;
            char_value(&spec_order(&c, 1))?;
            at += 8;
        }
        p += 1;
    }
    orientation
}

/// 0 when `f` is one of the six patterns as printed, 1 when reversed.
pub(super) fn finder_orientation(f: &[u8; 5]) -> Option<u8> {
    if FINDERS.contains(f) {
        return Some(0);
    }
    let mut r = *f;
    r.reverse();
    FINDERS.contains(&r).then_some(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_widths() {
        // One block: guard, 17 + 15 + 17, tail of one bar.
        assert_eq!(row_modules(21, 1, 1), (1 + 49 + 1) as f32);
        // Half block: 17 + 15.
        assert_eq!(row_modules(13, 1, 1), (1 + 32 + 1) as f32);
        assert_eq!(row_modules(42, 1, 2), (1 + 98 + 2) as f32);
        // A guard space before the region.
        assert_eq!(row_modules(21, 2, 2), (2 + 49 + 2) as f32);
    }
}
