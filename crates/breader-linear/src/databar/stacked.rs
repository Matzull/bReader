//! Joins the rows of GS1 DataBar Stacked, Stacked Omnidirectional and
//! Expanded Stacked into symbols.
//!
//! The scanline decoders read each row on its own ([`super::omni`] halves,
//! [`super::rows`]); a row carries no checksum and the aggregator groups its
//! agreeing lines like a symbol. This module takes those groups with their
//! geometry and finds the sets that make a symbol:
//!
//! * **Stacked and Stacked Omnidirectional**: the top row (guard, character
//!   1, finder, character 2) and the bottom row (character 4, finder,
//!   character 3) of one 96-module symbol cut at the middle. Both are read
//!   in the same direction, the bottom row lies on the far side of the top
//!   row (to the right of its reading direction), both ends line up one
//!   module apart, and the four characters and both finders must pass the
//!   checksum of the whole symbol.
//! * **Expanded Stacked**: rows of whole blocks (two characters and a
//!   finder), the first one read as the first row of the symbol, all
//!   printed left aligned (even rows of a symbol of an even number of
//!   columns are mirrored); the first character gives the symbol size and
//!   with it the number of rows, the length of the last row, the finder
//!   patterns and the orientation of the first finder of each row, then the
//!   check character must agree.
//!
//! The geometry uses the ends of the rows only, never the direction of the
//! scanlines (a row is read by lines of several plan angles): the left ends
//! of the rows lie on the left edge of the symbol, the right ends of whole
//! rows on its right edge, so the vectors between the ends of two rows agree.
//! Geometry guards the choice of rows, the checksum guards the symbol.

use breader_core::Symbology;

use super::expanded::{self, char_value, message_of, spec_order};
use super::omni;
use super::rows::{self, BLOCK, HALF_BLOCK};

/// Placement of a group of rows in pass-image pixels: the mean ends of its
/// lines in reading order and its module.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Geom {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub module: f32,
}

/// A symbol made of rows.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Assembly {
    /// Indices of the rows (in the slice given to [`assemble`]).
    pub used: Vec<usize>,
    pub symbology: Symbology,
    pub text: Vec<u8>,
    /// Characters of an Expanded Stacked symbol, 0 for the others.
    pub segments: u8,
    pub rows: u8,
}

/// Largest ratio of the modules of two rows.
const MODULE_RATIO: f32 = 1.25;
/// Rows whose ends are closer than this many modules are the same row seen
/// twice.
const MIN_GAP: f32 = 2.5;
/// Farthest a row of Stacked or Stacked Omnidirectional can be (modules).
const MAX_GAP: f32 = 150.0;
/// Farthest from the first row, per row, in Expanded Stacked (modules).
const MAX_ROW_GAP: f32 = 120.0;
/// Largest distance, in modules, of a right end from the line through the
/// first row's right end along the left edge (the ends of the rows differ
/// by up to two modules, the means of the lines by a module or so).
const ALIGN: f32 = 5.0;
/// Largest distance, in modules, of a left end from the line through the
/// others.
const COLLINEAR: f32 = 3.0;
/// Least cosine between the stacking direction and the perpendicular of a
/// row's chord (the chord of the mean ends can be tilted by the height of
/// the row against its length).
const DOWN_COS: f32 = 0.3;
/// Largest cosine between the vector from a row to the next and the row's
/// own direction: a row below another is not far from perpendicular to it.
const SHEAR_COS: f32 = 0.8;
/// Most search steps (row choices) per first row.
const MAX_TRIES: usize = 2048;

type Pt = (f32, f32);

fn sub(a: Pt, b: Pt) -> Pt {
    (a.0 - b.0, a.1 - b.1)
}

fn dot(a: Pt, b: Pt) -> f32 {
    a.0 * b.0 + a.1 * b.1
}

fn cross(a: Pt, b: Pt) -> f32 {
    a.0 * b.1 - a.1 * b.0
}

fn norm(a: Pt) -> f32 {
    a.0.hypot(a.1)
}

fn chord(g: &Geom) -> Pt {
    sub(g.to, g.from)
}

/// Direction to the right of the row's reading direction: where the next
/// row of the symbol lies (image coordinates, y down).
fn down_of(g: &Geom) -> Option<Pt> {
    let c = chord(g);
    let n = norm(c);
    (n.is_finite() && n > 1e-3).then(|| (-c.1 / n, c.0 / n))
}

fn modules_alike(a: &Geom, b: &Geom) -> bool {
    let r = a.module / b.module;
    r.is_finite() && (1.0 / MODULE_RATIO..=MODULE_RATIO).contains(&r)
}

/// Left and right end of a row, `mirrored` when it is read from its right
/// end.
fn ends(g: &Geom, mirrored: bool) -> (Pt, Pt) {
    if mirrored {
        (g.to, g.from)
    } else {
        (g.from, g.to)
    }
}

/// The top row `t` and the bottom row `b` can be the two rows of one
/// Stacked symbol.
fn pair_fits(t: &Geom, b: &Geom) -> bool {
    let Some(down) = down_of(t) else {
        return false;
    };
    if dot(chord(t), chord(b)) <= 0.0 || !modules_alike(t, b) {
        return false;
    }
    let m = (t.module + b.module) / 2.0;
    let (el, er) = (sub(b.from, t.from), sub(b.to, t.to));
    let gap = norm(el);
    // The right ends lie on the line of the left ends' direction: the
    // edges of the symbol are parallel. (The ends of a row are means over
    // lines at several angles, so how far along the edge they are says
    // nothing.)
    gap.is_finite()
        && (MIN_GAP * m..=MAX_GAP * m).contains(&gap)
        && cross(er, el).abs() <= ALIGN * m * gap
        && dot(el, down) >= DOWN_COS * gap
        && dot(er, down) >= DOWN_COS * norm(er)
        && sheared_ok(t, el)
}

/// The vector `e` between two rows is not more than `SHEAR_COS` along the
/// direction of row `g`.
fn sheared_ok(g: &Geom, e: Pt) -> bool {
    let c = chord(g);
    let (n, ne) = (norm(c), norm(e));
    n > 1e-3 && ne > 1e-3 && dot(c, e).abs() <= SHEAR_COS * n * ne
}

/// Stacked rows among `parts` (row text with the placement of its group).
pub(crate) fn assemble(parts: &[(&[u8], Geom)]) -> Vec<Assembly> {
    let mut found = Vec::new();
    for (i, (text, g)) in parts.iter().enumerate() {
        if text.first() == Some(&omni::PART_TOP) {
            for (j, (other, b)) in parts.iter().enumerate() {
                if other.first() != Some(&omni::PART_BOTTOM) || !pair_fits(g, b) {
                    continue;
                }
                if let Some(body) = omni::combine(&text[1..], &other[1..]) {
                    found.push(Assembly {
                        used: vec![i, j],
                        symbology: Symbology::DataBar,
                        text: super::element_string(body),
                        segments: 0,
                        rows: 2,
                    });
                }
            }
        } else if text.first() == Some(&rows::PART_ROW) {
            expanded_from(parts, i, &mut found);
        }
    }
    merge(found)
}

/// Equal symbols that share a row are one symbol.
fn merge(found: Vec<Assembly>) -> Vec<Assembly> {
    let mut out: Vec<Assembly> = Vec::new();
    for a in found {
        match out.iter_mut().find(|o| {
            o.symbology == a.symbology
                && o.text == a.text
                && o.used.iter().any(|u| a.used.contains(u))
        }) {
            Some(o) => {
                for u in a.used {
                    if !o.used.contains(&u) {
                        o.used.push(u);
                    }
                }
            }
            None => out.push(a),
        }
    }
    out
}

/// Characters and finders of a row's region, as printed.
fn split_region(region: &[u8], chars: &mut Vec<[u8; 8]>, finders: &mut Vec<[u8; 5]>) -> Option<()> {
    let mut at = 0;
    while at < region.len() {
        chars.push(region.get(at..at + 8)?.try_into().ok()?);
        finders.push(region.get(at + 8..at + 13)?.try_into().ok()?);
        at += 13;
        if at < region.len() {
            chars.push(region.get(at..at + 8)?.try_into().ok()?);
            at += 8;
        }
    }
    Some(())
}

/// What the first row of an Expanded Stacked symbol fixes for the others.
struct Layout {
    /// Blocks per row.
    k: usize,
    rows: usize,
    /// Characters of the symbol.
    n: usize,
    /// Blocks of the last row.
    last_blocks: usize,
}

impl Layout {
    /// Elements of the region of row `i` (from 0).
    fn region_len(&self, i: usize) -> usize {
        if i + 1 < self.rows {
            self.k * BLOCK
        } else if self.n % 2 == 1 {
            BLOCK * (self.last_blocks - 1) + HALF_BLOCK
        } else {
            BLOCK * self.last_blocks
        }
    }

    /// Orientation of the first finder of row `i` (the blocks above it) and,
    /// as bit 1, whether its region starts with a bar: the even rows of a
    /// symbol with an odd number of columns are printed in the opposite
    /// colours.
    fn parity(&self, i: usize) -> u8 {
        ((i * self.k) % 2) as u8 | (u8::from(i % 2 == 1 && self.k % 2 == 1) << 1)
    }

    /// Row `i` is printed mirrored: the even rows of a symbol with an even
    /// number of columns, but for the last one when it has an odd number of
    /// blocks.
    fn mirrored(&self, i: usize) -> bool {
        let special = i + 1 == self.rows
            && self.rows.is_multiple_of(2)
            && self.last_blocks != self.k
            && self.last_blocks % 2 == 1;
        self.k.is_multiple_of(2) && i % 2 == 1 && !special
    }
}

/// Expanded Stacked symbols whose first row is `parts[first]`.
fn expanded_from(parts: &[(&[u8], Geom)], first: usize, found: &mut Vec<Assembly>) {
    let (text, g1) = &parts[first];
    // A first row: first finder as printed, whole blocks.
    if text.len() < 2 + BLOCK || text[1] != 0 || (text.len() - 2) % BLOCK != 0 {
        return;
    }
    let k = (text.len() - 2) / BLOCK;
    let Some(c0) = text[2..10].try_into().ok() else {
        return;
    };
    let Some(v0) = char_value(&spec_order(&c0, 0)) else {
        return;
    };
    let n = 4 + usize::from(v0) / 211;
    if n > expanded::MAX_CHARS {
        return;
    }
    let blocks = n.div_ceil(2);
    if blocks <= k {
        return;
    }
    let rows = blocks.div_ceil(k);
    let layout = Layout {
        k,
        rows,
        n,
        last_blocks: blocks - (rows - 1) * k,
    };
    let Some(down) = down_of(g1) else {
        return;
    };
    // Rows below the first, nearest first, by the distance of their left
    // ends from the first row's.
    let m = g1.module;
    let mut below: Vec<(usize, f32)> = parts
        .iter()
        .enumerate()
        .filter(|(i, (t, _))| *i != first && t.first() == Some(&rows::PART_ROW) && t.len() > 2)
        .filter(|(_, (_, g))| modules_alike(g1, g))
        .map(|(i, (_, g))| {
            // Left end of the row as printed, whichever way it is read.
            let mirrored = dot(chord(g), chord(g1)) < 0.0;
            let (l, _) = ends(g, mirrored);
            (i, dot(sub(l, g1.from), down))
        })
        .filter(|&(_, t)| (MIN_GAP * m..=MAX_ROW_GAP * m * rows as f32).contains(&t))
        .collect();
    below.sort_by(|a, b| a.1.total_cmp(&b.1));
    let mut tries = 0usize;
    let mut chosen = vec![first];
    let mut results: Vec<Assembly> = Vec::new();
    search(
        parts,
        &layout,
        &below,
        &mut chosen,
        &mut tries,
        &mut results,
    );
    // A first row that reads two different symbols is not trusted.
    if results.iter().any(|r| r.text != results[0].text) {
        return;
    }
    found.extend(results);
}

fn search(
    parts: &[(&[u8], Geom)],
    layout: &Layout,
    below: &[(usize, f32)],
    chosen: &mut Vec<usize>,
    tries: &mut usize,
    results: &mut Vec<Assembly>,
) {
    let depth = chosen.len();
    *tries += 1;
    if depth == layout.rows {
        if rows_aligned(parts, layout, chosen)
            && let Some(a) = join(parts, chosen, layout)
        {
            results.push(a);
        }
        return;
    }
    let g1 = &parts[chosen[0]].1;
    let prev = chosen
        .last()
        .and_then(|p| below.iter().find(|(i, _)| i == p));
    let prev_t = prev.map_or(0.0, |&(_, t)| t);
    for &(i, t) in below {
        if *tries >= MAX_TRIES {
            return;
        }
        let (text, g) = &parts[i];
        if chosen.contains(&i)
            || t < prev_t + MIN_GAP * g.module
            || text.len() != 2 + layout.region_len(depth)
            || text[1] != layout.parity(depth)
            // Read in the direction its printing calls for.
            || (dot(chord(g), chord(g1)) < 0.0) != layout.mirrored(depth)
        {
            continue;
        }
        chosen.push(i);
        search(parts, layout, below, chosen, tries, results);
        chosen.pop();
    }
}

/// Left ends on one line, right ends of the whole rows on a parallel one,
/// the rows in order along it, one below the other.
fn rows_aligned(parts: &[(&[u8], Geom)], layout: &Layout, chosen: &[usize]) -> bool {
    let g1 = &parts[chosen[0]].1;
    let (l1, r1) = ends(g1, false);
    let Some(down) = down_of(g1) else {
        return false;
    };
    let ends_of = |depth: usize, i: usize| ends(&parts[i].1, layout.mirrored(depth));
    // Direction of the left edge: from the first row to the farthest.
    let Some(&last) = chosen.last() else {
        return false;
    };
    let far = sub(ends_of(chosen.len() - 1, last).0, l1);
    let len = norm(far);
    if !(len.is_finite() && len > 0.0) {
        return false;
    }
    let edge = (far.0 / len, far.1 / len);
    let mut prev = 0.0f32;
    for (depth, &i) in chosen.iter().enumerate().skip(1) {
        let g = &parts[i].1;
        let m = (g1.module + g.module) / 2.0;
        let (l, r) = ends_of(depth, i);
        let el = sub(l, l1);
        let along = dot(el, edge);
        if cross(el, edge).abs() > COLLINEAR * m
            || along <= prev
            || dot(el, down) < DOWN_COS * norm(el)
            || !sheared_ok(g1, el)
        {
            return false;
        }
        prev = along;
        let last_row = depth + 1 == layout.rows;
        if last_row {
            // The last row is not longer than the others.
            if norm(sub(r, l)) > norm(sub(r1, l1)) + 6.0 * m + 0.5 * norm(el) {
                return false;
            }
        } else if cross(sub(r, r1), edge).abs() > ALIGN * m {
            return false;
        }
    }
    true
}

fn join(parts: &[(&[u8], Geom)], chosen: &[usize], layout: &Layout) -> Option<Assembly> {
    let n = layout.n;
    let mut chars = Vec::with_capacity(n);
    let mut finders = Vec::with_capacity(n.div_ceil(2));
    for &i in chosen {
        split_region(&parts[i].0[2..], &mut chars, &mut finders)?;
    }
    let text = message_of(&chars, &finders)?;
    Some(Assembly {
        used: chosen.to_vec(),
        symbology: Symbology::DataBarExpanded,
        text,
        segments: u8::try_from(n).ok()?,
        rows: u8::try_from(layout.rows).ok()?,
    })
}
