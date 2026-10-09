//! From row lines to a codeword matrix: row numbers from the indicators, the
//! layout vote, per-cell votes, erasures and the bounded retries around
//! Reed-Solomon.

use crate::decode::{self, DecodedPdf417, ERASED};
use crate::row::{Cw, RowRead};
use crate::spec::{self, IndicatorField, Layout};

/// One row line: its place along the boundary and what was read on it.
#[derive(Debug, Clone)]
pub(crate) struct Obs {
    pub t: f32,
    pub read: RowRead,
}

/// A decoded symbol with the mapping of boundary positions to rows.
#[derive(Debug, Clone)]
pub(crate) struct Solved {
    pub decoded: DecodedPdf417,
    pub compact: bool,
    /// Row number of a line at boundary position `t`: `slope * t + offset`
    /// (the middle of the row); `None` when too few lines had a row number.
    pub fit: Option<(f32, f32)>,
}

/// Candidate layouts tried per symbol.
const MAX_LAYOUTS: usize = 6;
/// Decoder runs with changed cells per layout.
const MAX_RETRIES: usize = 14;
/// Weight of a clean codeword vote, of a doubtful one and of a second
/// candidate.
const W_SURE: u16 = 4;
const W_WEAK: u16 = 3;
const W_ALT: u16 = 1;

fn indicator(cw: &Cw, cluster: u8, left: bool) -> Option<(usize, IndicatorField)> {
    if usize::from(cw.value) >= 900 {
        return None;
    }
    let row = 3 * (usize::from(cw.value) / 30) + usize::from(cluster);
    Some((row, spec::parse_indicator(row, left, cw.value)?))
}

/// Row number and the fields of the indicators of a line.
#[derive(Debug, Default, Clone, Copy)]
struct Indicators {
    row: Option<usize>,
    left: Option<IndicatorField>,
    right: Option<IndicatorField>,
}

fn indicators(read: &RowRead) -> Indicators {
    let l = (read.len >= 1)
        .then(|| indicator(&read.cw[0], read.cluster, true))
        .flatten();
    let r = read
        .stop
        .filter(|&s| s >= 2)
        .and_then(|s| indicator(&read.cw[s - 1], read.cluster, false));
    match (l, r) {
        (Some(a), Some(b)) if a.0 == b.0 => Indicators {
            row: Some(a.0),
            left: Some(a.1),
            right: Some(b.1),
        },
        (Some(_), Some(_)) => Indicators::default(),
        (Some(a), None) => Indicators {
            row: Some(a.0),
            left: Some(a.1),
            right: None,
        },
        (None, Some(b)) => Indicators {
            row: Some(b.0),
            left: None,
            right: Some(b.1),
        },
        (None, None) => Indicators::default(),
    }
}

#[derive(Debug, Default)]
struct Votes {
    rows_div3: [u32; 30],
    level_rows: [u32; 27],
    cols: [u32; 30],
}

impl Votes {
    fn add(&mut self, f: IndicatorField) {
        match f {
            IndicatorField::RowsDiv3(x) => self.rows_div3[usize::from(x)] += 1,
            IndicatorField::LevelRowsMod3 { level, rows_mod3 } => {
                self.level_rows[usize::from(level) * 3 + usize::from(rows_mod3)] += 1;
            }
            IndicatorField::ColsMinus1(x) => self.cols[usize::from(x)] += 1,
        }
    }
}

/// The values of `votes` worth trying, best first: the top one and a close
/// second.
fn leaders(votes: &[u32]) -> Vec<(usize, u32)> {
    let mut v: Vec<(usize, u32)> = votes
        .iter()
        .copied()
        .enumerate()
        .filter(|&(_, n)| n > 0)
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let top = v.first().map_or(0, |x| x.1);
    v.retain(|x| 2 * x.1 >= top);
    v.truncate(2);
    v
}

fn layouts(votes: &Votes) -> Vec<Layout> {
    let (a, b, c) = (
        leaders(&votes.rows_div3),
        leaders(&votes.level_rows),
        leaders(&votes.cols),
    );
    let mut all = Vec::new();
    for &(rd, n1) in &a {
        for &(lr, n2) in &b {
            for &(cm, n3) in &c {
                if let Some(layout) =
                    spec::combine_fields(rd as u8, (lr / 3) as u8, (lr % 3) as u8, cm as u8)
                    && layout.rows * layout.cols <= spec::MAX_CODEWORDS
                {
                    all.push((n1 + n2 + n3, layout));
                }
            }
        }
    }
    all.sort_by(|x, y| y.0.cmp(&x.0));
    all.into_iter().map(|x| x.1).take(MAX_LAYOUTS).collect()
}

/// A cell of the matrix: candidate values with their vote weights.
#[derive(Debug, Clone, Copy, Default)]
struct Cell {
    v: [u16; 4],
    w: [u16; 4],
}

impl Cell {
    fn add(&mut self, value: u16, weight: u16) {
        if value == ERASED {
            return;
        }
        if let Some(i) = (0..4).find(|&i| self.w[i] > 0 && self.v[i] == value) {
            self.w[i] += weight;
            return;
        }
        let weakest = (0..4).min_by_key(|&i| self.w[i]).expect("four slots");
        if self.w[weakest] < weight {
            self.v[weakest] = value;
            self.w[weakest] = weight;
        }
    }

    /// Candidates, best first, with weights.
    fn ranked(&self) -> [(u16, u16); 4] {
        let mut r = [0, 1, 2, 3].map(|i| (self.v[i], self.w[i]));
        r.sort_by(|a, b| b.1.cmp(&a.1));
        r
    }

    /// The winning value, or erased when there is none or two tie.
    fn value(&self) -> u16 {
        let r = self.ranked();
        if r[0].1 == 0 || r[0].1 == r[1].1 {
            ERASED
        } else {
            r[0].0
        }
    }

    /// Weight margin of the winner over the runner-up.
    fn margin(&self) -> u16 {
        let r = self.ranked();
        r[0].1 - r[1].1
    }
}

/// Row numbers of all lines: those the indicators gave, checked against a
/// linear fit over the boundary position, and the rest interpolated.
/// Returns the fit.
fn assign_rows(
    obs: &[Obs],
    ind: &[Indicators],
    rows: usize,
    out: &mut [Option<usize>],
) -> Option<(f32, f32)> {
    for (o, i) in out.iter_mut().zip(ind) {
        *o = i.row.filter(|&r| r < rows);
    }
    let known: Vec<(f32, f32)> = obs
        .iter()
        .zip(out.iter())
        .filter_map(|(o, r)| r.map(|r| (o.t, r as f32)))
        .collect();
    if known.len() < 2 {
        return None;
    }
    let mut slopes = Vec::new();
    for i in 0..known.len() {
        for k in 1..=3 {
            if let Some(b) = known.get(i + k) {
                let dt = b.0 - known[i].0;
                if dt.abs() > 1e-3 && b.1 != known[i].1 {
                    slopes.push((b.1 - known[i].1) / dt);
                }
            }
        }
    }
    if slopes.is_empty() {
        return None;
    }
    slopes.sort_by(f32::total_cmp);
    let slope = slopes[slopes.len() / 2];
    let mut offsets: Vec<f32> = known.iter().map(|&(t, r)| r - slope * t).collect();
    offsets.sort_by(f32::total_cmp);
    let offset = offsets[offsets.len() / 2];
    let fits = |t: f32, r: f32| (r - (slope * t + offset)).abs() <= 1.5;
    let good = known.iter().filter(|&&(t, r)| fits(t, r)).count();
    if 5 * good < 3 * known.len() {
        return None;
    }
    for (o, ob) in out.iter_mut().zip(obs) {
        if let Some(r) = *o
            && !fits(ob.t, r as f32)
        {
            *o = None;
        }
        if o.is_none() {
            let est = slope * ob.t + offset;
            let cluster = usize::from(ob.read.cluster);
            let best = (0..rows)
                .filter(|r| r % 3 == cluster)
                .min_by(|&a, &b| (a as f32 - est).abs().total_cmp(&(b as f32 - est).abs()));
            *o = best.filter(|&r| (r as f32 - est).abs() <= 1.0);
        }
    }
    Some((slope, offset))
}

fn build_cells(obs: &[Obs], rows_of: &[Option<usize>], layout: Layout) -> Vec<Cell> {
    let mut cells = vec![Cell::default(); layout.rows * layout.cols];
    for (o, row) in obs.iter().zip(rows_of) {
        let Some(row) = *row else {
            continue;
        };
        let r = &o.read;
        if r.len < 2 || r.stop.is_some_and(|s| s != layout.cols + 2) {
            continue;
        }
        // A line cut short by the image border still fills its columns.
        for c in 0..layout.cols.min(r.len - 1) {
            let cw = &r.cw[1 + c];
            let cell = &mut cells[row * layout.cols + c];
            cell.add(cw.value, if cw.sure { W_SURE } else { W_WEAK });
            cell.add(cw.alt, W_ALT);
        }
    }
    cells
}

fn try_decode(grid: &[u16], layout: Layout) -> Option<DecodedPdf417> {
    decode::decode_codewords(grid, layout).ok()
}

fn decode_cells(cells: &[Cell], layout: Layout) -> Option<DecodedPdf417> {
    let grid: Vec<u16> = cells.iter().map(Cell::value).collect();
    if let Some(d) = try_decode(&grid, layout) {
        return Some(d);
    }
    let ec = spec::ec_codewords(layout.ec_level);
    let budget = ec - decode::reserve(ec).min(ec);
    // Cells that have a rival, weakest first.
    let mut doubtful: Vec<usize> = (0..cells.len())
        .filter(|&i| cells[i].ranked()[1].1 > 0 && cells[i].value() != ERASED)
        .collect();
    doubtful.sort_by_key(|&i| (cells[i].margin(), cells[i].ranked()[0].1));
    let mut tries = 0;
    // Erase the doubtful cells, a growing number of them.
    let erased = grid.iter().filter(|&&v| v == ERASED).count();
    let room = budget.saturating_sub(erased);
    let mut k = 1;
    while k <= room.min(doubtful.len()) && tries < MAX_RETRIES / 2 {
        let mut g = grid.clone();
        for &i in &doubtful[..k] {
            g[i] = ERASED;
        }
        tries += 1;
        if let Some(d) = try_decode(&g, layout) {
            return Some(d);
        }
        k = if k < 8 { k + 1 } else { k * 2 };
    }
    // Put the runner-up into the weakest cells, a growing number of them.
    let mut k = 1;
    while k <= doubtful.len() && tries < MAX_RETRIES {
        let mut g = grid.clone();
        for &i in &doubtful[..k] {
            g[i] = cells[i].ranked()[1].0;
        }
        tries += 1;
        if let Some(d) = try_decode(&g, layout) {
            return Some(d);
        }
        k = if k < 4 { k + 1 } else { k * 2 };
    }
    None
}

/// Solves the symbol from its lines, or `None`.
pub(crate) fn solve(obs: &[Obs]) -> Option<Solved> {
    let ind: Vec<Indicators> = obs.iter().map(|o| indicators(&o.read)).collect();
    let mut votes = Votes::default();
    for (o, i) in obs.iter().zip(&ind) {
        for f in [i.left, i.right].into_iter().flatten() {
            votes.add(f);
        }
        if let Some(s) = o.read.stop.filter(|s| (3..=32).contains(s)) {
            votes.cols[s - 3] += 1;
        }
    }
    let mut rows_of = vec![None; obs.len()];
    for layout in layouts(&votes) {
        let fit = assign_rows(obs, &ind, layout.rows, &mut rows_of);
        let cells = build_cells(obs, &rows_of, layout);
        let Some(decoded) = decode_cells(&cells, layout) else {
            continue;
        };
        let compact = is_compact(obs, &rows_of, layout);
        return Some(Solved {
            decoded,
            compact,
            fit,
        });
    }
    None
}

/// Full symbols have a right indicator after the data columns that matches
/// the layout; compact ones do not.
fn is_compact(obs: &[Obs], rows_of: &[Option<usize>], layout: Layout) -> bool {
    let (mut with, mut matching) = (0usize, 0usize);
    for (o, row) in obs.iter().zip(rows_of) {
        let Some(row) = *row else {
            continue;
        };
        if o.read.len > layout.cols + 1 {
            with += 1;
            if o.read.cw[layout.cols + 1].value == layout.right_indicator(row) {
                matching += 1;
            }
        }
    }
    3 * matching < with.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{EncodeOptions, Rendered, Rng, render};

    /// Row lines for `lines_per_row` positions of every row of a rendered
    /// symbol.
    fn lines(r: &Rendered, lines_per_row: usize) -> Vec<Obs> {
        let cols = r.layout.cols;
        let mut obs = Vec::new();
        for row in 0..r.layout.rows {
            for k in 0..lines_per_row {
                let mut read = RowRead::default();
                let sure = |v: u16| Cw {
                    value: v,
                    alt: ERASED,
                    sure: true,
                };
                read.cw[0] = sure(r.left[row]);
                for c in 0..cols {
                    read.cw[1 + c] = sure(r.cells[row * cols + c]);
                }
                if r.compact {
                    read.len = cols + 1;
                } else {
                    read.cw[cols + 1] = sure(r.right[row]);
                    read.len = cols + 2;
                    read.stop = Some(cols + 2);
                }
                read.cluster = (row % 3) as u8;
                obs.push(Obs {
                    t: (row * lines_per_row + k) as f32 * 1.5,
                    read,
                });
            }
        }
        obs
    }

    fn text(len: usize) -> String {
        (0..len)
            .map(|i| (b'A' + (i * 7 % 26) as u8) as char)
            .collect()
    }

    #[test]
    fn solves_a_clean_symbol() {
        let r = render(&text(60), EncodeOptions::default());
        let s = solve(&lines(&r, 2)).expect("solved");
        assert_eq!(s.decoded.data, text(60).as_bytes());
        assert_eq!(s.decoded.layout, r.layout);
        assert!(!s.compact);
        let (slope, _) = s.fit.unwrap();
        assert!((slope - 1.0 / 3.0).abs() < 0.02, "{slope}");
    }

    #[test]
    fn compact_symbols_are_recognised() {
        let opts = EncodeOptions {
            compact: true,
            ..EncodeOptions::default()
        };
        let r = render(&text(50), opts);
        let s = solve(&lines(&r, 2)).expect("solved");
        assert!(s.compact);
        assert_eq!(s.decoded.data, text(50).as_bytes());
    }

    #[test]
    fn lines_without_indicators_are_placed_by_position() {
        let r = render(&text(80), EncodeOptions::default());
        let mut obs = lines(&r, 3);
        let mut rng = Rng(5);
        for o in &mut obs {
            if rng.below(2) == 0 {
                o.read.cw[0].value = ERASED;
                o.read.cw[o.read.stop.unwrap() - 1].value = ERASED;
            }
        }
        let s = solve(&obs).expect("solved");
        assert_eq!(s.decoded.data, text(80).as_bytes());
    }

    #[test]
    fn votes_beat_a_wrong_line_and_missing_cells_become_erasures() {
        let opts = EncodeOptions {
            level: 4,
            ..EncodeOptions::default()
        };
        let r = render(&text(90), opts);
        let mut obs = lines(&r, 3);
        let cols = r.layout.cols;
        // One line in three of some rows reads a wrong value.
        for o in obs.iter_mut().step_by(7) {
            o.read.cw[1].value ^= 5;
        }
        // Some cells are unreadable on every line.
        for row in [1usize, 4] {
            for o in obs.iter_mut() {
                if o.read.cluster == (row % 3) as u8 && o.t as usize / 5 == row {
                    o.read.cw[1 + (row % cols)].value = ERASED;
                }
            }
        }
        let s = solve(&obs).expect("solved");
        assert_eq!(s.decoded.data, text(90).as_bytes());
    }

    #[test]
    fn a_runner_up_value_can_be_swapped_in() {
        let r = render(&text(70), EncodeOptions::default());
        let mut obs = lines(&r, 1);
        let cols = r.layout.cols;
        // Single lines: wrong best value, right alternative.
        for (row, o) in obs.iter_mut().enumerate().take(8) {
            let right = o.read.cw[1 + row % cols].value;
            o.read.cw[1 + row % cols] = Cw {
                value: right ^ 3,
                alt: right,
                sure: false,
            };
        }
        let s = solve(&obs).expect("solved");
        assert_eq!(s.decoded.data, text(70).as_bytes());
    }

    #[test]
    fn garbage_lines_decode_to_nothing() {
        let mut rng = Rng(77);
        for _ in 0..50 {
            let mut obs = Vec::new();
            for j in 0..40 {
                let mut read = RowRead {
                    len: 8,
                    cluster: (j % 3) as u8,
                    ..RowRead::default()
                };
                for cw in read.cw.iter_mut().take(8) {
                    *cw = Cw {
                        value: rng.below(929) as u16,
                        alt: ERASED,
                        sure: true,
                    };
                }
                obs.push(Obs { t: j as f32, read });
            }
            assert!(solve(&obs).is_none());
        }
    }
}
