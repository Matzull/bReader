//! Reading the codewords of one row line: element widths to codewords, with
//! the cluster check, and the sequential walk along the runs.

use breader_linear::stacked::{MarkKind, RawMark, Runs};

use crate::decode::ERASED;
use crate::spec;

/// Codewords kept per line: left indicator, 30 data columns, right indicator
/// and some slack for a stray read beyond the symbol.
pub(crate) const MAX_CW: usize = 36;

/// Largest relative deviation of a codeword's length from the running
/// module estimate that is still read as one codeword.
const LENGTH_TOLERANCE: f32 = 0.25;

/// A codeword candidate for the element widths of one position.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cand {
    /// Row number modulo 3 of the cluster the pattern belongs to.
    pub cluster: u8,
    pub value: u16,
    /// Squared distance of the rounded element boundaries from the measured
    /// ones, in modules.
    pub cost: f32,
}

/// Up to three candidates of one position, best first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cands {
    pub list: [Cand; 3],
    pub n: usize,
}

impl Cands {
    const EMPTY: Cand = Cand {
        cluster: 0,
        value: ERASED,
        cost: f32::MAX,
    };

    fn new() -> Self {
        Self {
            list: [Self::EMPTY; 3],
            n: 0,
        }
    }

    fn push(&mut self, c: Cand) {
        if self.list[..self.n]
            .iter()
            .any(|x| x.value == c.value && x.cluster == c.cluster)
        {
            return;
        }
        let at = self.list[..self.n]
            .iter()
            .position(|x| c.cost < x.cost)
            .unwrap_or(self.n);
        if at >= 3 {
            return;
        }
        let end = self.n.min(2);
        for k in (at..end).rev() {
            self.list[k + 1] = self.list[k];
        }
        self.list[at] = c;
        self.n = (self.n + 1).min(3);
    }

    pub(crate) fn as_slice(&self) -> &[Cand] {
        &self.list[..self.n]
    }
}

/// The codewords whose 17 modules are the 8 element widths `w` (bar first),
/// rounded to whole modules at the element boundaries. Neighbouring
/// roundings are tried when a boundary is close to a half module or when the
/// first rounding is not a codeword.
pub(crate) fn read_widths(w: &[f32; 8]) -> Cands {
    let mut out = Cands::new();
    let total: f32 = w.iter().sum();
    if total.is_nan() || total <= 0.0 {
        return out;
    }
    let scale = 17.0 / total;
    let mut c = [0f32; 9];
    for k in 0..8 {
        c[k + 1] = c[k] + w[k] * scale;
    }
    c[8] = 17.0;
    let mut base = [0i32; 9];
    for (b, &x) in base.iter_mut().zip(&c) {
        *b = x.round() as i32;
    }
    consider(&mut out, &c, &base);
    let ambiguous = (1..8).any(|j| (c[j] - c[j].floor() - 0.5).abs() < 0.3);
    if ambiguous || out.n == 0 {
        for j in 1..8 {
            let mut r = base;
            r[j] += if r[j] as f32 > c[j] { -1 } else { 1 };
            consider(&mut out, &c, &r);
        }
    }
    if out.n == 0 {
        for i in 1..8 {
            for j in i + 1..8 {
                let mut r = base;
                r[i] += if r[i] as f32 > c[i] { -1 } else { 1 };
                r[j] += if r[j] as f32 > c[j] { -1 } else { 1 };
                consider(&mut out, &c, &r);
            }
        }
    }
    out
}

/// Adds the codeword whose element boundaries are `r` (in modules), if any.
fn consider(out: &mut Cands, c: &[f32; 9], r: &[i32; 9]) {
    let mut widths = [0u8; 8];
    for k in 0..8 {
        let d = r[k + 1] - r[k];
        if !(1..=6).contains(&d) {
            return;
        }
        widths[k] = d as u8;
    }
    let Some(modules) = spec::pattern_of_widths(widths) else {
        return;
    };
    let Some((cluster, value)) = spec::lookup(modules) else {
        return;
    };
    let cost = (1..8).map(|j| (c[j] - r[j] as f32).powi(2)).sum();
    out.push(Cand {
        cluster: cluster as u8,
        value,
        cost,
    });
}

/// A codeword of a row line after the cluster check.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cw {
    /// Best candidate of the row's cluster, or [`ERASED`].
    pub value: u16,
    /// Second candidate of the same cluster, or [`ERASED`].
    pub alt: u16,
    /// The best candidate was a clean read.
    pub sure: bool,
}

impl Cw {
    const ERASED: Self = Self {
        value: ERASED,
        alt: ERASED,
        sure: false,
    };
}

/// The codewords of one line between the start pattern and the end of the
/// symbol, in reading order. Index 0 is the left row indicator.
#[derive(Debug, Clone)]
pub(crate) struct RowRead {
    pub cw: [Cw; MAX_CW],
    pub len: usize,
    /// Row number modulo 3 shared by the codewords.
    pub cluster: u8,
    /// Index of the codeword where the stop pattern began, if it was seen.
    pub stop: Option<usize>,
    /// Samples per module near the end of the walk.
    pub module: f32,
    /// Profile position of the start pattern's outer edge, and of the end of
    /// the last codeword or the stop pattern.
    pub from: f32,
    pub to: f32,
}

impl Default for RowRead {
    fn default() -> Self {
        Self {
            cw: [Cw::ERASED; MAX_CW],
            len: 0,
            cluster: 0,
            stop: None,
            module: 0.0,
            from: 0.0,
            to: 0.0,
        }
    }
}

/// Where the codewords of a line begin.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Entry {
    /// Run index of the first codeword's first bar.
    pub first: usize,
    /// Samples per module.
    pub module: f32,
    /// Profile position of the start pattern's outer edge and of its end.
    pub from: f32,
    pub to: f32,
}

impl Entry {
    /// The codewords that follow a start pattern found in the runs.
    pub(crate) fn after(start: &RawMark) -> Self {
        Self {
            first: start.run + 8,
            module: start.module,
            from: start.from,
            to: start.to,
        }
    }
}

/// Walks the codewords from `start` on. Returns false when the line has no
/// consistent cluster.
pub(crate) fn walk(runs: &Runs, marks: &[RawMark], start: &Entry, out: &mut RowRead) -> bool {
    let n = runs.len();
    let widths = runs.widths();
    let mut slots = [Cands::new(); MAX_CW];
    let mut len = 0;
    let mut stop = None;
    let mut j = start.first;
    let mut module = start.module;
    let mut to = start.to;
    // Codeword boundaries read cleanly: (index, position), the start pattern
    // being index -1.
    let mut anchors = [(0f32, 0f32); MAX_CW + 2];
    anchors[0] = (-1.0, start.from);
    let mut anchored = 1;
    while len < MAX_CW && j < n {
        if marks
            .iter()
            .any(|s| s.kind == MarkKind::Stop && s.run == j && s.to > s.from)
        {
            stop = Some(len);
            to = runs.end((j + 8).min(n - 1));
            anchors[anchored] = (len as f32, runs.start(j));
            anchored += 1;
            break;
        }
        if j + 8 > n {
            break;
        }
        let x0 = runs.start(j);
        let total = runs.end(j + 7) - x0;
        let expected = 17.0 * module;
        if runs.is_bar(j) && (total - expected).abs() <= LENGTH_TOLERANCE * expected {
            let w: [f32; 8] = widths[j..j + 8].try_into().expect("eight runs");
            slots[len] = read_widths(&w);
            anchors[anchored] = (len as f32, x0);
            anchored += 1;
            len += 1;
            module = 0.7 * module + 0.3 * total / 17.0;
            to = runs.end(j + 7);
            j += 8;
            continue;
        }
        // Unreadable: skip one codeword length and take the next bar edge
        // near it.
        len += 1;
        let target = x0 + expected;
        let reach = 0.3 * expected;
        let next = (j + 1..n)
            .take_while(|&k| runs.start(k) <= target + reach)
            .filter(|&k| runs.is_bar(k) && (runs.start(k) - target).abs() <= reach)
            .min_by(|&a, &b| {
                (runs.start(a) - target)
                    .abs()
                    .total_cmp(&(runs.start(b) - target).abs())
            });
        match next {
            Some(k) => {
                to = target;
                j = k;
            }
            None => break,
        }
    }
    regrid(runs, &anchors[..anchored], &mut slots[..len]);
    out.len = len;
    out.stop = stop;
    out.module = module;
    out.from = start.from;
    out.to = to;
    settle(&slots[..len], out)
}

/// Reads the codewords again on the grid of modules that the clean codeword
/// boundaries span: with a constant module width every edge sits at a whole
/// number of modules from its codeword's start, which is more exact than
/// scaling each codeword to its own length when the pixels are coarse.
fn regrid(runs: &Runs, anchors: &[(f32, f32)], slots: &mut [Cands]) {
    if anchors.len() < 4 {
        return;
    }
    let mut pts: Vec<(f32, f32)> = anchors.to_vec();
    let (mut a, mut b): (f32, f32);
    loop {
        let n = pts.len() as f32;
        let (mk, mx) = pts
            .iter()
            .fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
        let (mut num, mut den) = (0.0, 0.0);
        for p in &pts {
            num += (p.0 - mk) * (p.1 - mx);
            den += (p.0 - mk) * (p.0 - mk);
        }
        if den < 1e-6 {
            return;
        }
        b = num / den;
        a = mx - b * mk;
        let (worst, res) = pts
            .iter()
            .enumerate()
            .map(|(i, p)| (i, (p.1 - (a + b * p.0)).abs()))
            .fold((0, 0.0), |w, r| if r.1 > w.1 { r } else { w });
        let module = b / 17.0;
        if res <= 0.2 * module {
            break;
        }
        if pts.len() <= 4 {
            if res > 0.35 * module {
                return;
            }
            break;
        }
        pts.swap_remove(worst);
    }
    let module = b / 17.0;
    if module < 0.8 || !module.is_finite() {
        return;
    }
    let n = runs.len();
    let edge = |i: usize| {
        if i < n {
            runs.start(i)
        } else {
            runs.end(n - 1)
        }
    };
    for (k, slot) in slots.iter_mut().enumerate() {
        let x = a + b * k as f32;
        // First edge not before the codeword's start, less half a module.
        let (mut lo, mut hi) = (0usize, n + 1);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if edge(mid) < x - 0.5 * module {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        let first = lo;
        if first + 8 > n || (edge(first) - x).abs() > 0.5 * module || !runs.is_bar(first) {
            continue;
        }
        if (edge(first + 8) - x - 17.0 * module).abs() > 0.5 * module {
            continue;
        }
        let mut c = [0f32; 9];
        for (j, u) in c.iter_mut().enumerate().take(8).skip(1) {
            *u = (edge(first + j) - x) / module;
        }
        c[8] = 17.0;
        let mut base = [0i32; 9];
        for (r, &u) in base.iter_mut().zip(&c) {
            *r = u.round() as i32;
        }
        consider(slot, &c, &base);
        for j in 1..8 {
            if (c[j] - c[j].floor() - 0.5).abs() < 0.3 {
                let mut r = base;
                r[j] += if r[j] as f32 > c[j] { -1 } else { 1 };
                consider(slot, &c, &r);
            }
        }
    }
}

/// Picks the row's cluster by majority and the candidate of that cluster for
/// every position.
fn settle(slots: &[Cands], out: &mut RowRead) -> bool {
    let mut votes = [0usize; 3];
    for s in slots {
        if let Some(best) = s.as_slice().first() {
            votes[usize::from(best.cluster)] += 1;
        }
    }
    let (cluster, &top) = votes
        .iter()
        .enumerate()
        .max_by_key(|&(_, v)| *v)
        .expect("three clusters");
    let rival = votes
        .iter()
        .enumerate()
        .filter(|&(c, _)| c != cluster)
        .map(|(_, &v)| v)
        .max()
        .unwrap_or(0);
    if top < 2 || top < 2 * rival {
        return false;
    }
    out.cluster = cluster as u8;
    for (cw, s) in out.cw.iter_mut().zip(slots) {
        let mut same = s
            .as_slice()
            .iter()
            .filter(|c| usize::from(c.cluster) == cluster);
        *cw = match same.next() {
            Some(first) => Cw {
                value: first.value,
                alt: same.next().map_or(ERASED, |c| c.value),
                sure: first.cost < 0.35,
            },
            None => Cw::ERASED,
        };
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{EncodeOptions, render};

    /// Element widths of codeword `value` in cluster `k`, bar first.
    fn widths_of(k: usize, value: u16) -> [f32; 8] {
        let bits = spec::pattern(k, value);
        let mut out = [0f32; 8];
        let (mut idx, mut prev, mut run) = (0usize, 1u32, 0f32);
        for i in (0..17).rev() {
            let b = bits >> i & 1;
            if b == prev {
                run += 1.0;
            } else {
                out[idx] = run;
                idx += 1;
                prev = b;
                run = 1.0;
            }
        }
        out[idx] = run;
        out
    }

    #[test]
    fn exact_widths_read_back_every_codeword() {
        for k in 0..3 {
            for value in (0..929).step_by(7) {
                let c = read_widths(&widths_of(k, value));
                let best = c.as_slice()[0];
                assert_eq!((usize::from(best.cluster), best.value), (k, value));
                assert!(best.cost < 1e-6);
            }
        }
    }

    #[test]
    fn blurred_widths_still_round_to_the_codeword() {
        let mut w = widths_of(1, 400);
        // Shift boundaries by up to 0.3 module.
        let shifts = [0.3, -0.25, 0.2, -0.3, 0.25, -0.2, 0.3, -0.3];
        for (x, s) in w.iter_mut().zip(shifts) {
            *x += s * 0.5;
        }
        let best = read_widths(&w).as_slice()[0];
        assert_eq!((best.cluster, best.value), (1, 400));
    }

    #[test]
    fn ambiguous_widths_keep_a_second_candidate() {
        let mut w = widths_of(0, 100);
        // Move the boundary between elements 2 and 3 to the half module.
        w[2] += 0.5;
        w[3] -= 0.5;
        let c = read_widths(&w);
        assert!(c.n >= 2);
        assert!(
            c.as_slice()
                .iter()
                .any(|x| x.value == 100 && x.cluster == 0)
        );
    }

    #[test]
    fn nonsense_widths_give_nothing() {
        assert_eq!(read_widths(&[1.0, 9.0, 1.0, 1.0, 1.0, 1.0, 1.0, 2.0]).n, 0);
        assert_eq!(read_widths(&[0.0; 8]).n, 0);
        assert_eq!(read_widths(&[f32::NAN; 8]).n, 0);
    }

    /// Runs of one row of a rendered symbol: quiet zone, then every module
    /// run at `module` samples.
    fn runs_of_row(row: &[u8], module: f32) -> Runs {
        let mut widths = Vec::new();
        let mut cur = row[0];
        let mut len = 0usize;
        for &b in row {
            if b == cur {
                len += 1;
            } else {
                widths.push(len as f32 * module);
                cur = b;
                len = 1;
            }
        }
        widths.push(len as f32 * module);
        Runs::from_widths(row[0] != 0, &widths, 0.0, 0.0)
    }

    #[test]
    fn walks_a_rendered_row_to_the_stop_pattern() {
        let r = render("PDF417 row walk 0123456789", EncodeOptions::default());
        let cols = r.layout.cols;
        for row in 0..r.layout.rows.min(5) {
            // Rebuild the module row from the codeword tables.
            let mut bits: Vec<u8> = vec![0; 10];
            let mut push = |m: u32, n: usize| {
                for i in (0..n).rev() {
                    bits.push((m >> i & 1) as u8);
                }
            };
            push(0x1_FEA8, 17);
            push(spec::pattern(row % 3, r.left[row]), 17);
            for c in 0..cols {
                push(spec::pattern(row % 3, r.cells[row * cols + c]), 17);
            }
            push(spec::pattern(row % 3, r.right[row]), 17);
            push(0x3_FA29, 18);
            bits.extend([0u8; 10]);
            let runs = runs_of_row(&bits, 3.0);
            let mut marks = Vec::new();
            breader_linear::stacked::find(&runs, &mut marks);
            let start = marks
                .iter()
                .find(|m| m.kind == MarkKind::Start && m.to > m.from)
                .expect("start");
            let mut read = RowRead::default();
            assert!(walk(&runs, &marks, &Entry::after(start), &mut read));
            assert_eq!(read.cluster as usize, row % 3);
            assert_eq!(read.stop, Some(cols + 2), "row {row}");
            assert_eq!(read.cw[0].value, r.left[row]);
            for c in 0..cols {
                assert_eq!(read.cw[1 + c].value, r.cells[row * cols + c]);
            }
            assert_eq!(read.cw[cols + 1].value, r.right[row]);
        }
    }
}
