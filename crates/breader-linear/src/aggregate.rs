use breader_core::{EanAddonPolicy, LinearMeta, Point, Quad, Symbol, SymbolMeta, Symbology};

use std::cell::RefCell;

use crate::databar::stacked::{self, Assembly, Geom};

/// A hit placed in pass-image coordinates.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PlacedHit {
    pub symbology: Symbology,
    pub text: Vec<u8>,
    pub addon: Option<Vec<u8>>,
    /// Segment from the start to the end of the symbol along the scanline.
    pub from: Point,
    pub to: Point,
    /// Index of the scanline that produced the hit.
    pub line: u32,
    /// Distance in pixels between the scanline and its neighbours in the
    /// plan, or between the refinement lines for refined hits.
    pub step: f32,
    /// Estimated module width in pixels of the scanned image.
    pub module: f32,
    pub quality: f32,
    pub aim: [u8; 3],
    pub checksum_verified: bool,
    /// See `LineHit::short_quiet`.
    pub short_quiet: bool,
    /// Light bars on a dark background. Hits of different polarity never
    /// read the same symbol.
    pub inverted: bool,
    pub start_stop: Option<(u8, u8)>,
    pub segments: u8,
    /// See `LineHit::thin`.
    pub thin: bool,
    /// See `LineHit::part`.
    pub part: bool,
}

impl PlacedHit {
    fn length(&self) -> f32 {
        self.from.distance(self.to)
    }

    fn center(&self) -> Point {
        mid(self.from, self.to)
    }

    fn same_symbol(&self, other: &Self) -> bool {
        self.symbology == other.symbology
            && self.inverted == other.inverted
            && self.text == other.text
            && self.start_stop == other.start_stop
            && self.segments == other.segments
            && self.part == other.part
    }

    /// True when `other` can be another view of the same physical symbol:
    /// the same text, and either a neighbouring parallel line or a segment
    /// centered on almost the same spot.
    fn adjacent(&self, other: &Self) -> bool {
        if !self.same_symbol(other) {
            return false;
        }
        let len = self.length().max(other.length());
        let (a, b) = (self.center(), other.center());
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        if dx.hypot(dy) < CLOSE * len {
            return true;
        }
        let (ux, uy) = self.unit();
        let (vx, vy) = other.unit();
        if (ux * vx + uy * vy).abs() < PARALLEL_COS {
            return false;
        }
        let along = (dx * ux + dy * uy).abs();
        let across = (dx * uy - dy * ux).abs();
        along < ALONG * len && across <= LINE_GAP * self.step.max(other.step)
    }

    fn unit(&self) -> (f32, f32) {
        let len = self.length().max(1e-6);
        (
            (self.to.x - self.from.x) / len,
            (self.to.y - self.from.y) / len,
        )
    }
}

fn mid(a: Point, b: Point) -> Point {
    Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
}

/// Centers closer than this fraction of the segment length belong to the
/// same symbol whatever the angle.
const CLOSE: f32 = 0.2;
/// Cosine of the largest angle between lines that can be neighbours.
const PARALLEL_COS: f32 = 0.966;
/// Largest offset of the centers along the lines, as a fraction of the length.
const ALONG: f32 = 0.25;
/// Largest offset across the lines, in scanline steps.
const LINE_GAP: f32 = 2.0;
/// Segments closer than this many pixels count as touching.
const TOUCH: f32 = 1.0;
/// ITF has no check digit in general and any even digit string is a valid
/// payload, so it always needs this many agreeing lines.
const ITF_MIN_AGREEMENT: u16 = 3;
/// Two agreeing lines count as independent when they are this many times the
/// smaller of their scanline steps apart (2 px to 6 px for retry lines).
const INDEPENDENT_STEPS: f32 = 1.9;
/// Largest deviation of a line's module estimate from the group median for
/// the line to count towards the geometric guard.
/// Lines a group read with the relaxed Code 39 rules needs beyond the plain
/// requirement: 4 with the default `min_line_agreement` of 2.
const THIN_EXTRA_LINES: u16 = 2;
const MODULE_AGREEMENT: f32 = 0.15;
/// Upper bound on hits kept per pass, a guard against pathological input.
const MAX_HITS: usize = 4096;

/// EAN-13 and UPC-A share one layout, so a probe of one is met by the other.
fn same_family(a: Symbology, b: Symbology) -> bool {
    a == b
        || matches!(
            (a, b),
            (Symbology::Ean13, Symbology::UpcA) | (Symbology::UpcA, Symbology::Ean13)
        )
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Settings {
    pub min_agreement: u16,
    /// Lines with an index below this belong to the sparse scan plan; higher
    /// ones were added around a lone hit. Lines next to each other see almost
    /// the same pixels, so a group backed by a single plan line needs one
    /// more agreeing line than `min_agreement`.
    pub plan_lines: u32,
    /// Bit `n` set: an ITF symbol of `n` digits needs one more agreeing line.
    pub itf_strict: u128,
    pub addon: EanAddonPolicy,
    /// Added to the across extent of the quad on both sides, in pass pixels.
    pub across_pad: f32,
    /// Pass-image to original-frame scale.
    pub scale: (f32, f32),
}

#[derive(Debug, Clone)]
struct Group {
    members: Vec<usize>,
}

#[derive(Debug, Clone)]
struct Candidate {
    members: Vec<usize>,
    lines: u16,
    addon: Option<Vec<u8>>,
    frame: Frame,
    /// A stacked GS1 DataBar symbol joined from the rows of its members.
    stacked: Option<Assembly>,
}

/// Oriented bounding rectangle of a group's segments.
#[derive(Debug, Clone, Copy)]
struct Frame {
    origin: Point,
    u: (f32, f32),
    along: (f32, f32),
    across: (f32, f32),
}

impl Frame {
    fn project(&self, p: Point) -> (f32, f32) {
        let (dx, dy) = (p.x - self.origin.x, p.y - self.origin.y);
        (
            dx * self.u.0 + dy * self.u.1,
            -dx * self.u.1 + dy * self.u.0,
        )
    }

    fn contains(&self, p: Point) -> bool {
        let (s, t) = self.project(p);
        s >= self.along.0 && s <= self.along.1 && t >= self.across.0 && t <= self.across.1
    }

    fn corner(&self, s: f32, t: f32) -> Point {
        Point::new(
            self.origin.x + s * self.u.0 - t * self.u.1,
            self.origin.y + s * self.u.1 + t * self.u.0,
        )
    }
}

/// Working vectors of the agreement rules, kept across calls (and scans) so
/// that evaluating a group does not allocate.
#[derive(Debug, Clone, Default)]
struct Scratch {
    /// Line indices seen by `distinct_lines`.
    lines: Vec<u32>,
    /// Members that are not thin hits, in a group that mixes both.
    plain: Vec<usize>,
    /// Module widths and (offset, step) pairs of `independent_lines`.
    modules: Vec<f32>,
    offsets: Vec<(f32, f32)>,
    /// Chain of adjacent hits of `seed_met`.
    members: Vec<usize>,
    pending: Vec<usize>,
    seen: Vec<bool>,
}

impl PlacedHit {
    /// A copy whose text comes from the spare buffers.
    pub fn clone_pooled(&self) -> Self {
        let mut text = crate::pool::take(self.text.len());
        text.extend_from_slice(&self.text);
        Self {
            text,
            ..self.clone_without_text()
        }
    }

    fn clone_without_text(&self) -> Self {
        Self {
            symbology: self.symbology,
            text: Vec::new(),
            addon: self.addon.clone(),
            from: self.from,
            to: self.to,
            line: self.line,
            step: self.step,
            module: self.module,
            quality: self.quality,
            aim: self.aim,
            checksum_verified: self.checksum_verified,
            short_quiet: self.short_quiet,
            inverted: self.inverted,
            start_stop: self.start_stop,
            segments: self.segments,
            thin: self.thin,
            part: self.part,
        }
    }

    /// Hands the text buffer back to the spare buffers.
    pub fn recycle(self) {
        crate::pool::give(self.text);
    }
}

/// Collects the hits of one pass and turns agreeing ones into symbols.
#[derive(Debug, Clone, Default)]
pub(crate) struct Aggregator {
    hits: Vec<PlacedHit>,
    groups: Vec<Group>,
    /// Member vectors of dropped groups, reused by the next grouping.
    spare_members: Vec<Vec<usize>>,
    scratch: RefCell<Scratch>,
}

impl Aggregator {
    /// Forgets the hits of the previous pass; keeps the allocations.
    pub fn reset(&mut self) {
        for hit in self.hits.drain(..) {
            hit.recycle();
        }
        self.clear_groups();
    }

    fn clear_groups(&mut self) {
        for g in self.groups.drain(..) {
            self.spare_members.push(g.members);
        }
    }

    pub fn push(&mut self, hit: PlacedHit) {
        if self.hits.len() < MAX_HITS {
            self.hits.push(hit);
        }
    }

    /// Puts the hits from `mid` on among those from `from` to `mid`, both in
    /// line order, by line and after the hits of the same line, as if each
    /// had been pushed when its line was scanned. Allocates nothing.
    pub fn merge_lines(&mut self, from: usize, mid: usize) {
        let mid = mid.min(self.hits.len());
        for i in mid..self.hits.len() {
            let line = self.hits[i].line;
            let at = from + self.hits[from..i].partition_point(|h| h.line <= line);
            self.hits[at..=i].rotate_right(1);
        }
    }

    /// Removes and returns the hits collected so far.
    #[cfg(feature = "rayon")]
    pub fn take_hits(&mut self) -> Vec<PlacedHit> {
        std::mem::take(&mut self.hits)
    }

    pub fn len(&self) -> usize {
        self.hits.len()
    }

    /// Hits added since the aggregator held `since` hits that read the same
    /// symbol as `seed`.
    pub fn count_matching(&self, since: usize, seed: &PlacedHit) -> usize {
        self.hits[since.min(self.hits.len())..]
            .iter()
            .filter(|h| h.same_symbol(seed))
            .count()
    }

    /// True when the hits that read the same physical symbol as `seed` (the
    /// chain of adjacent hits from it) already meet the agreement `finish`
    /// will require. Refinement lines beyond this point only widen the quad.
    pub fn seed_met(&self, seed: &PlacedHit, settings: &Settings) -> bool {
        let mut s = self.scratch.take();
        let (mut members, mut pending, mut seen) = (
            std::mem::take(&mut s.members),
            std::mem::take(&mut s.pending),
            std::mem::take(&mut s.seen),
        );
        members.clear();
        pending.clear();
        pending.extend((0..self.hits.len()).filter(|&i| self.hits[i] == *seed));
        let met = if pending.is_empty() {
            false
        } else {
            seen.clear();
            seen.resize(self.hits.len(), false);
            for &i in &pending {
                seen[i] = true;
            }
            while let Some(i) = pending.pop() {
                members.push(i);
                for (j, done) in seen.iter_mut().enumerate() {
                    if !*done && self.hits[i].adjacent(&self.hits[j]) {
                        *done = true;
                        pending.push(j);
                    }
                }
            }
            let (lines, need) = self.agreement(&members, settings, &mut s);
            lines >= need
        };
        (s.members, s.pending, s.seen) = (members, pending, seen);
        self.scratch.replace(s);
        met
    }

    /// True when a symbol of `symbology` that starts within `radius` of `at`
    /// already has the agreement `finish` will require.
    pub fn symbol_met_near(
        &self,
        symbology: Symbology,
        at: Point,
        radius: f32,
        settings: &Settings,
    ) -> bool {
        self.hits
            .iter()
            .filter(|h| same_family(h.symbology, symbology) && h.from.distance(at) < radius)
            .any(|h| self.seed_met(h, settings))
    }

    /// One representative hit of every group that lacks the agreement
    /// required by `settings`.
    pub fn seeds(&mut self, settings: &Settings, out: &mut Vec<PlacedHit>) {
        for seed in out.drain(..) {
            seed.recycle();
        }
        self.group();
        let mut s = self.scratch.take();
        for g in &self.groups {
            let (lines, need) = self.agreement(&g.members, settings, &mut s);
            if lines < need {
                out.push(self.hits[g.members[0]].clone_pooled());
            }
        }
        self.scratch.replace(s);
    }

    /// Agreeing lines of a group and the lines it needs. Thin hits count only
    /// in a group made of nothing else: next to ordinary hits they are
    /// ignored, so they can neither help nor hinder.
    fn agreement(&self, members: &[usize], settings: &Settings, s: &mut Scratch) -> (u16, u16) {
        if members.iter().any(|&m| self.hits[m].thin) && members.iter().any(|&m| !self.hits[m].thin)
        {
            let mut plain = std::mem::take(&mut s.plain);
            plain.clear();
            plain.extend(members.iter().copied().filter(|&m| !self.hits[m].thin));
            let result = (
                distinct_lines(&self.hits, &plain, &mut s.lines, |_| true),
                self.required(&plain, settings, s),
            );
            s.plain = plain;
            return result;
        }
        (
            distinct_lines(&self.hits, members, &mut s.lines, |_| true),
            self.required(members, settings, s),
        )
    }

    fn required(&self, members: &[usize], settings: &Settings, s: &mut Scratch) -> u16 {
        let mut min = settings.min_agreement.max(1);
        match self.hits[members[0]].symbology {
            Symbology::Itf => {
                min = min.max(ITF_MIN_AGREEMENT);
                let digits = self.hits[members[0]].text.len();
                if settings.itf_strict >> digits.min(127) & 1 == 1 {
                    min += 1;
                }
            }
            // No check character either: one more line than the setting asks.
            Symbology::Codabar => min += 1,
            _ => {}
        }
        let thin = members.iter().any(|&m| self.hits[m].thin);
        if thin {
            min += THIN_EXTRA_LINES;
        }
        // Code 39 and ITF have no check character, so retry lines around one
        // plan line are not enough. Instead of one more line, `min` lines that
        // are apart from each other and agree on the module width are.
        let guarded = matches!(
            self.hits[members[0]].symbology,
            Symbology::Code39 | Symbology::Code93 | Symbology::Itf
        ) && self.independent_lines(members, min, s);
        if thin && !guarded {
            return u16::MAX;
        }
        // Lines of the sparse plan, as opposed to the ones added around a hit.
        let sparse = |h: &PlacedHit| h.line < settings.plan_lines;
        let mut need = if min >= 2
            && !guarded
            && distinct_lines(&self.hits, members, &mut s.lines, sparse) < 2
        {
            min + 1
        } else {
            min
        };
        // A short quiet zone is only trusted with a check character or with
        // one more agreeing line.
        if members
            .iter()
            .any(|&m| self.hits[m].short_quiet && !self.hits[m].checksum_verified)
        {
            need += 1;
        }
        need
    }

    /// True when at least `need` lines of the group are at least
    /// `INDEPENDENT_STEPS` of their steps apart across the symbol and their
    /// module estimates are within `MODULE_AGREEMENT` of the group median.
    fn independent_lines(&self, members: &[usize], need: u16, s: &mut Scratch) -> bool {
        let modules = &mut s.modules;
        modules.clear();
        modules.extend(members.iter().map(|&m| self.hits[m].module));
        modules.sort_by(f32::total_cmp);
        let median = modules[modules.len() / 2];
        if median <= 0.0 {
            return false;
        }
        let (ux, uy) = self.hits[members[0]].unit();
        let offsets = &mut s.offsets;
        offsets.clear();
        offsets.extend(
            members
                .iter()
                .map(|&m| &self.hits[m])
                .filter(|h| (h.module / median - 1.0).abs() <= MODULE_AGREEMENT)
                .map(|h| {
                    let c = h.center();
                    (c.x * -uy + c.y * ux, h.step)
                }),
        );
        offsets.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut count = 0u16;
        let mut last: Option<(f32, f32)> = None;
        for &(off, step) in offsets.iter() {
            if last.is_none_or(|(o, s)| off - o >= INDEPENDENT_STEPS * step.min(s)) {
                count += 1;
                last = Some((off, step));
            }
        }
        count >= need
    }

    /// Groups hits by symbol identity and proximity, applies the agreement
    /// and conflict rules and appends the surviving symbols to `out`.
    pub fn finish(&mut self, settings: &Settings, out: &mut Vec<Symbol>) {
        self.group();
        let min = settings.min_agreement.max(1);
        let mut candidates: Vec<Candidate> = Vec::new();
        // Rows of stacked DataBar symbols: not symbols until joined.
        let mut rows: Vec<Candidate> = Vec::new();
        let mut s = self.scratch.take();
        for g in &self.groups {
            let (lines, need) = self.agreement(&g.members, settings, &mut s);
            if lines < need {
                continue;
            }
            let addon = self.choose_addon(&g.members, min, &mut s.lines);
            if settings.addon == EanAddonPolicy::Required && addon.is_none() {
                continue;
            }
            let candidate = Candidate {
                members: g.members.clone(),
                lines,
                addon,
                frame: self.frame(&g.members, settings.across_pad),
                stacked: None,
            };
            if self.hits[g.members[0]].part {
                rows.push(candidate);
            } else {
                candidates.push(candidate);
            }
        }
        self.scratch.replace(s);
        if !rows.is_empty() {
            self.join_rows(&rows, settings, &mut candidates);
        }

        for (i, c) in candidates.iter().enumerate() {
            let survives = candidates.iter().enumerate().all(|(j, o)| {
                i == j
                    || self.same_read(c, o)
                    || !self.overlap(c, o)
                    || u32::from(c.lines) >= 2 * u32::from(o.lines)
            });
            if survives {
                out.push(self.symbol(c, settings));
            }
        }
    }

    /// True when both candidates read the same symbol.
    fn same_read(&self, a: &Candidate, b: &Candidate) -> bool {
        match (&a.stacked, &b.stacked) {
            (None, None) => self.hits[a.members[0]].same_symbol(&self.hits[b.members[0]]),
            (Some(x), Some(y)) => x.symbology == y.symbology && x.text == y.text,
            _ => false,
        }
    }

    /// Joins the groups of rows into stacked symbols and appends them to
    /// `candidates`.
    fn join_rows(&self, rows: &[Candidate], settings: &Settings, candidates: &mut Vec<Candidate>) {
        let geoms: Vec<Geom> = rows.iter().map(|r| self.geom(&r.members)).collect();
        let parts: Vec<(&[u8], Geom)> = rows
            .iter()
            .zip(&geoms)
            .map(|(r, &g)| (self.hits[r.members[0]].text.as_slice(), g))
            .collect();
        for a in stacked::assemble(&parts) {
            // Rows seen on at least `need` lines each; the symbol has as
            // many agreeing lines as its weakest row.
            let used: Vec<&Candidate> = a.used.iter().map(|&i| &rows[i]).collect();
            let members: Vec<usize> = used.iter().flat_map(|c| c.members.clone()).collect();
            let lines = used.iter().map(|c| c.lines).min().unwrap_or(0);
            candidates.push(Candidate {
                frame: self.frame(&members, settings.across_pad),
                members,
                lines,
                addon: None,
                stacked: Some(a),
            });
        }
    }

    /// Mean ends of the lines of a group of rows, in reading order.
    fn geom(&self, members: &[usize]) -> Geom {
        let k = members.len().max(1) as f32;
        let (mut f, mut t, mut m) = ((0.0f32, 0.0f32), (0.0f32, 0.0f32), 0.0f32);
        for &i in members {
            let h = &self.hits[i];
            f = (f.0 + h.from.x, f.1 + h.from.y);
            t = (t.0 + h.to.x, t.1 + h.to.y);
            m += h.module;
        }
        Geom {
            from: (f.0 / k, f.1 / k),
            to: (t.0 / k, t.1 / k),
            module: m / k,
        }
    }

    fn group(&mut self) {
        self.clear_groups();
        for i in 0..self.hits.len() {
            let mut joined: Option<usize> = None;
            let mut g = 0;
            while g < self.groups.len() {
                let linked = self.groups[g].members.iter().any(|&m| {
                    let (a, b) = (&self.hits[i], &self.hits[m]);
                    a.adjacent(b)
                });
                if !linked {
                    g += 1;
                    continue;
                }
                match joined {
                    None => {
                        self.groups[g].members.push(i);
                        joined = Some(g);
                        g += 1;
                    }
                    Some(first) => {
                        let mut moved = self.groups.remove(g).members;
                        self.groups[first].members.extend_from_slice(&moved);
                        moved.clear();
                        self.spare_members.push(moved);
                    }
                }
            }
            if joined.is_none() {
                let mut members = self.spare_members.pop().unwrap_or_default();
                members.clear();
                members.push(i);
                self.groups.push(Group { members });
            }
        }
    }

    /// The supplement read by at least `min` distinct lines, if exactly one.
    fn choose_addon(&self, members: &[usize], min: u16, seen: &mut Vec<u32>) -> Option<Vec<u8>> {
        let mut found: Vec<&Vec<u8>> = Vec::new();
        for &m in members {
            if let Some(a) = &self.hits[m].addon
                && !found.contains(&a)
            {
                found.push(a);
            }
        }
        let mut agreed = found.into_iter().filter(|a| {
            let same = |h: &PlacedHit| h.addon.as_ref() == Some(*a);
            distinct_lines(&self.hits, members, seen, same) >= min
        });
        let first = agreed.next()?;
        agreed.next().is_none().then(|| first.clone())
    }

    fn frame(&self, members: &[usize], pad: f32) -> Frame {
        let (mut ux, mut uy) = (0.0f32, 0.0f32);
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        let (rx, ry) = self.hits[members[0]].unit();
        for &m in members {
            let h = &self.hits[m];
            let len = h.length().max(1e-6);
            // Rows of a stacked symbol may be read in opposite directions.
            let sign = if (h.to.x - h.from.x) * rx + (h.to.y - h.from.y) * ry < 0.0 {
                -1.0
            } else {
                1.0
            };
            ux += sign * (h.to.x - h.from.x) / len;
            uy += sign * (h.to.y - h.from.y) / len;
            let c = h.center();
            cx += c.x;
            cy += c.y;
        }
        let norm = ux.hypot(uy);
        let u = if norm > 1e-3 {
            (ux / norm, uy / norm)
        } else {
            (1.0, 0.0)
        };
        let k = members.len().max(1) as f32;
        let mut frame = Frame {
            origin: Point::new(cx / k, cy / k),
            u,
            along: (0.0, 0.0),
            across: (0.0, 0.0),
        };
        for &m in members {
            for p in [self.hits[m].from, self.hits[m].to] {
                let (s, t) = frame.project(p);
                frame.along = (frame.along.0.min(s), frame.along.1.max(s));
                frame.across = (frame.across.0.min(t), frame.across.1.max(t));
            }
        }
        frame.across = (frame.across.0 - pad, frame.across.1 + pad);
        frame
    }

    fn overlap(&self, a: &Candidate, b: &Candidate) -> bool {
        for &i in &a.members {
            for &j in &b.members {
                let (x, y) = (&self.hits[i], &self.hits[j]);
                if segment_distance(x.from, x.to, y.from, y.to) <= TOUCH {
                    return true;
                }
            }
        }
        let points = |c: &Candidate| {
            c.members
                .iter()
                .flat_map(|&m| {
                    let h = &self.hits[m];
                    [h.from, h.to, h.center()]
                })
                .collect::<Vec<_>>()
        };
        points(b).into_iter().any(|p| a.frame.contains(p))
            || points(a).into_iter().any(|p| b.frame.contains(p))
    }

    fn symbol(&self, c: &Candidate, settings: &Settings) -> Symbol {
        let first = &self.hits[c.members[0]];
        let (sx, sy) = settings.scale;
        let map = |p: Point| Point::new((p.x + 0.5) * sx - 0.5, (p.y + 0.5) * sy - 0.5);
        let f = &c.frame;
        let quad = Quad::new([
            map(f.corner(f.along.0, f.across.0)),
            map(f.corner(f.along.1, f.across.0)),
            map(f.corner(f.along.1, f.across.1)),
            map(f.corner(f.along.0, f.across.1)),
        ]);
        let quality =
            c.members.iter().map(|&m| self.hits[m].quality).sum::<f32>() / c.members.len() as f32;
        let aim_id = if c.addon.is_some() {
            *b"]E3"
        } else {
            first.aim
        };
        let (symbology, data) = match &c.stacked {
            Some(a) => (a.symbology, a.text.clone()),
            None => (first.symbology, first.text.clone()),
        };
        let mut meta = LinearMeta::new(c.lines);
        meta.addon = c
            .addon
            .as_ref()
            .and_then(|a| String::from_utf8(a.clone()).ok());
        meta.checksum_verified = first.checksum_verified || c.stacked.is_some();
        meta.codabar_start_stop = first.start_stop;
        meta.databar_segments = match &c.stacked {
            Some(a) => (a.segments > 0).then_some(a.segments),
            None => (first.segments > 0).then_some(first.segments),
        };
        meta.databar_rows = c.stacked.as_ref().map(|a| a.rows);
        let mut symbol = Symbol::new(symbology, data, quad, aim_id, SymbolMeta::Linear(meta));
        symbol.quality = quality.clamp(0.0, 1.0);
        symbol
    }
}

/// Distinct scanlines of the members that satisfy `keep`; `seen` is scratch.
fn distinct_lines(
    hits: &[PlacedHit],
    members: &[usize],
    seen: &mut Vec<u32>,
    keep: impl Fn(&PlacedHit) -> bool,
) -> u16 {
    seen.clear();
    for &m in members {
        if !keep(&hits[m]) {
            continue;
        }
        let line = hits[m].line;
        if !seen.contains(&line) {
            seen.push(line);
        }
    }
    seen.len().min(usize::from(u16::MAX)) as u16
}

fn cross(a: Point, b: Point, c: Point) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

fn point_segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

fn segment_distance(a: Point, b: Point, c: Point, d: Point) -> f32 {
    let (d1, d2) = (cross(a, b, c), cross(a, b, d));
    let (d3, d4) = (cross(c, d, a), cross(c, d, b));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return 0.0;
    }
    [
        point_segment_distance(a, c, d),
        point_segment_distance(b, c, d),
        point_segment_distance(c, a, b),
        point_segment_distance(d, a, b),
    ]
    .into_iter()
    .fold(f32::MAX, f32::min)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pooled_clone_equals_clone_and_recycling_reuses_the_buffer() {
        let mut h = hit(Symbology::Ean13, "5901234123457", 10.0, 0.0, 100.0, 3);
        h.addon = Some(b"12".to_vec());
        h.part = true;
        let c = h.clone_pooled();
        assert_eq!(c, h);
        let ptr = c.text.as_ptr();
        c.recycle();
        assert_eq!(h.clone_pooled().text.as_ptr(), ptr);
    }

    fn hit(sym: Symbology, text: &str, y: f32, x0: f32, x1: f32, line: u32) -> PlacedHit {
        PlacedHit {
            symbology: sym,
            text: text.as_bytes().to_vec(),
            addon: None,
            from: Point::new(x0, y),
            to: Point::new(x1, y),
            line,
            step: 6.0,
            module: 3.0,
            quality: 0.5,
            aim: if sym == Symbology::Ean8 {
                *b"]E4"
            } else {
                *b"]E0"
            },
            checksum_verified: true,
            short_quiet: false,
            inverted: false,
            start_stop: None,
            segments: 0,
            thin: false,
            part: false,
        }
    }

    #[test]
    fn late_hits_are_merged_in_line_order() {
        let mut agg = Aggregator::default();
        let first = hit(Symbology::Code39, "EARLIER", 0.0, 0.0, 10.0, 0);
        agg.push(first);
        for line in [1, 3, 3, 6] {
            agg.push(hit(Symbology::Code39, "W", line as f32, 0.0, 10.0, line));
        }
        for line in [2, 4, 7] {
            agg.push(hit(Symbology::Code39, "T", line as f32, 0.0, 10.0, line));
        }
        agg.merge_lines(1, 5);
        let order: Vec<(u32, &[u8])> = agg.hits.iter().map(|h| (h.line, &h.text[..])).collect();
        assert_eq!(
            order,
            [
                (0, &b"EARLIER"[..]),
                (1, b"W"),
                (2, b"T"),
                (3, b"W"),
                (3, b"W"),
                (4, b"T"),
                (6, b"W"),
                (7, b"T"),
            ]
        );
    }

    fn settings(min: u16) -> Settings {
        Settings {
            min_agreement: min,
            plan_lines: u32::MAX,
            itf_strict: 0,
            addon: EanAddonPolicy::Optional,
            across_pad: 2.0,
            scale: (1.0, 1.0),
        }
    }

    fn run(hits: Vec<PlacedHit>, s: &Settings) -> Vec<Symbol> {
        let mut agg = Aggregator::default();
        for h in hits {
            agg.push(h);
        }
        let mut out = Vec::new();
        agg.finish(s, &mut out);
        out
    }

    #[test]
    fn single_line_is_not_enough_and_two_lines_are() {
        let one = vec![hit(Symbology::Ean13, "5901234123457", 10.0, 0.0, 100.0, 0)];
        assert!(run(one.clone(), &settings(2)).is_empty());
        assert_eq!(run(one.clone(), &settings(1)).len(), 1);
        let mut two = one;
        two.push(hit(Symbology::Ean13, "5901234123457", 20.0, 2.0, 101.0, 1));
        let out = run(two, &settings(2));
        assert_eq!(out.len(), 1);
        let SymbolMeta::Linear(meta) = &out[0].meta else {
            panic!("linear meta expected");
        };
        assert_eq!(meta.lines_agreeing, 2);
        assert!(meta.checksum_verified);
        assert_eq!(out[0].data, b"5901234123457");
        assert_eq!(&out[0].aim_id, b"]E0");
        assert!(out[0].quad.contains(Point::new(50.0, 15.0)));
    }

    #[test]
    fn a_single_sparse_line_needs_one_extra_confirmation() {
        let sparse = |plan_lines| Settings {
            plan_lines,
            ..settings(2)
        };
        let t = "5901234123457";
        let hit = |y: f32, line| hit(Symbology::Ean13, t, y, 0.0, 100.0, line);
        // Line 0 is the only plan line; 5 and 6 were added around it.
        let two = vec![hit(10.0, 0), hit(12.0, 5)];
        assert!(run(two.clone(), &sparse(1)).is_empty());
        let mut three = two.clone();
        three.push(hit(14.0, 6));
        assert_eq!(run(three, &sparse(1)).len(), 1);
        // Two plan lines are independent enough.
        assert_eq!(run(two, &sparse(10)).len(), 1);
        let mut agg = Aggregator::default();
        agg.push(hit(10.0, 0));
        agg.push(hit(12.0, 5));
        let mut seeds = Vec::new();
        agg.seeds(&sparse(1), &mut seeds);
        assert_eq!(seeds.len(), 1, "still short of the required lines");
        agg.seeds(&sparse(10), &mut seeds);
        assert!(seeds.is_empty());
    }

    #[test]
    fn code39_and_itf_retry_lines_need_to_be_apart_and_agree_on_the_module() {
        let sparse = |plan_lines, min| Settings {
            plan_lines,
            ..settings(min)
        };
        let c = |y: f32, line: u32, module: f32| {
            let mut h = hit(Symbology::Code39, "1234567", y, 0.0, 100.0, line);
            h.checksum_verified = false;
            h.module = module;
            h
        };
        // Line 0 is the only plan line; step is 6, so 12 apart is two steps.
        let apart = vec![c(10.0, 0, 3.0), c(22.0, 5, 3.0)];
        assert_eq!(run(apart.clone(), &sparse(1, 2)).len(), 1);
        let touching = vec![c(10.0, 0, 3.0), c(16.0, 5, 3.0)];
        assert!(run(touching.clone(), &sparse(1, 2)).is_empty());
        let mut three = touching;
        three.push(c(22.0, 6, 3.0));
        assert_eq!(run(three, &sparse(1, 2)).len(), 1);
        let off_module = vec![c(10.0, 0, 3.0), c(22.0, 5, 3.6)];
        assert!(run(off_module, &sparse(1, 2)).is_empty());
        let near_module = vec![c(10.0, 0, 3.0), c(22.0, 5, 3.3)];
        assert_eq!(run(near_module, &sparse(1, 2)).len(), 1);
        // A check character keeps the old rule: other symbologies are unchanged.
        let ean = |y: f32, line| hit(Symbology::Ean13, "5901234123457", y, 0.0, 100.0, line);
        assert!(run(vec![ean(10.0, 0), ean(22.0, 5)], &sparse(1, 2)).is_empty());
        // Duplicate hits on one line are one line.
        let mut same = apart;
        same.push(c(10.0, 0, 3.0));
        same.remove(1);
        assert!(run(same, &sparse(1, 2)).is_empty());
    }

    #[test]
    fn code93_follows_the_agreement_rules_of_code_39() {
        let sparse = |plan_lines, min| Settings {
            plan_lines,
            ..settings(min)
        };
        let c = |y: f32, line: u32, module: f32| {
            let mut h = hit(Symbology::Code93, "CODE 93", y, 0.0, 100.0, line);
            h.module = module;
            h.checksum_verified = true;
            h
        };
        // `min_line_agreement` lines, no more, when they are independent plan lines.
        assert!(run(vec![c(10.0, 0, 3.0)], &settings(2)).is_empty());
        assert_eq!(
            run(vec![c(10.0, 0, 3.0), c(22.0, 1, 3.0)], &settings(2)).len(),
            1
        );
        // Retry lines next to one plan line: apart and on one module, or one more line.
        let apart = vec![c(10.0, 0, 3.0), c(22.0, 5, 3.0)];
        assert_eq!(run(apart, &sparse(1, 2)).len(), 1);
        let touching = vec![c(10.0, 0, 3.0), c(16.0, 5, 3.0)];
        assert!(run(touching.clone(), &sparse(1, 2)).is_empty());
        let mut three = touching;
        three.push(c(22.0, 6, 3.0));
        assert_eq!(run(three, &sparse(1, 2)).len(), 1);
        assert!(run(vec![c(10.0, 0, 3.0), c(22.0, 5, 3.6)], &sparse(1, 2)).is_empty());
        let out = run(vec![c(10.0, 0, 3.0), c(22.0, 1, 3.0)], &settings(2));
        let SymbolMeta::Linear(meta) = &out[0].meta else {
            panic!("linear meta");
        };
        assert!(meta.checksum_verified && meta.codabar_start_stop.is_none());
    }

    #[test]
    fn itf_added_lengths_need_one_more_line_than_itf14() {
        let strict = Settings {
            itf_strict: (1 << 12) | (1 << 8),
            ..settings(2)
        };
        let i = |text: &str, line: u32| {
            let mut h = hit(
                Symbology::Itf,
                text,
                10.0 + 8.0 * line as f32,
                0.0,
                100.0,
                line,
            );
            h.checksum_verified = false;
            h
        };
        let lines = |text: &str, n: u32| (0..n).map(|l| i(text, l)).collect::<Vec<_>>();
        assert_eq!(run(lines("12345678901231", 3), &strict).len(), 1);
        assert!(run(lines("123456789012", 3), &strict).is_empty());
        assert_eq!(run(lines("123456789012", 4), &strict).len(), 1);
        assert_eq!(
            run(lines("1234567890", 3), &strict).len(),
            1,
            "not in the mask"
        );
        assert_eq!(run(lines("123456789012", 3), &settings(2)).len(), 1);
        assert!(run(lines("12345678", 3), &strict).is_empty());
        assert_eq!(run(lines("12345678", 4), &strict).len(), 1);
    }

    #[test]
    fn thin_code39_needs_four_independent_lines() {
        let s = Settings {
            plan_lines: 1,
            ..settings(2)
        };
        let t = |y: f32, line: u32, module: f32| {
            let mut h = hit(Symbology::Code39, "1234567", y, 0.0, 100.0, line);
            h.checksum_verified = false;
            h.thin = true;
            h.module = module;
            h
        };
        let two = vec![t(10.0, 0, 1.5), t(22.0, 5, 1.5)];
        assert!(run(two.clone(), &s).is_empty());
        let mut three = two.clone();
        three.push(t(34.0, 6, 1.5));
        assert!(run(three.clone(), &s).is_empty());
        // Lines 12 apart chain into one group (the gap limit is two steps).
        three.push(t(46.0, 7, 1.5));
        assert_eq!(run(three.clone(), &s).len(), 1);
        // Three lines next to each other are one view of the symbol.
        let near = vec![
            t(10.0, 0, 1.5),
            t(13.0, 5, 1.5),
            t(16.0, 6, 1.5),
            t(19.0, 7, 1.5),
        ];
        assert!(run(near, &s).is_empty());
        // The module estimates must agree.
        three[3].module = 2.0;
        assert!(run(three, &s).is_empty());
        // Plain Code 39 hits on the same lines need only two.
        let mut plain = two;
        for h in &mut plain {
            h.thin = false;
        }
        assert_eq!(run(plain, &s).len(), 1);
    }

    #[test]
    fn hits_of_different_polarity_never_agree() {
        let t = "5901234123457";
        let mut flipped = hit(Symbology::Ean13, t, 20.0, 0.0, 100.0, 1);
        flipped.inverted = true;
        let normal = hit(Symbology::Ean13, t, 10.0, 0.0, 100.0, 0);
        assert!(run(vec![normal.clone(), flipped.clone()], &settings(2)).is_empty());
        // A third dark line makes two agreeing ones; the light line adds nothing.
        let mut both = vec![normal.clone(), flipped.clone()];
        both.push(hit(Symbology::Ean13, t, 14.0, 0.0, 100.0, 2));
        let out = run(both, &settings(2));
        assert_eq!(out.len(), 1);
        let SymbolMeta::Linear(meta) = &out[0].meta else {
            panic!("linear meta expected");
        };
        assert_eq!(meta.lines_agreeing, 2);
        // Equal support on both polarities over the same spot: a conflict.
        let mut tie = vec![normal.clone()];
        flipped.from.y = 12.0;
        flipped.to.y = 12.0;
        tie.push(flipped);
        tie.push(hit(Symbology::Ean13, t, 14.0, 0.0, 100.0, 2));
        let mut light = hit(Symbology::Ean13, t, 13.0, 0.0, 100.0, 3);
        light.inverted = true;
        tie.push(light);
        assert!(run(tie, &settings(2)).is_empty());
    }

    #[test]
    fn a_short_quiet_zone_needs_one_more_line_unless_checked() {
        let t = "CODE39";
        let lines = |n: u32, checked: bool| {
            (0..n)
                .map(|i| {
                    let mut h = hit(Symbology::Code39, t, 10.0 + 3.0 * i as f32, 0.0, 100.0, i);
                    h.short_quiet = true;
                    h.checksum_verified = checked;
                    h
                })
                .collect::<Vec<_>>()
        };
        assert!(run(lines(2, false), &settings(2)).is_empty());
        assert_eq!(run(lines(3, false), &settings(2)).len(), 1);
        assert_eq!(run(lines(2, true), &settings(2)).len(), 1);
    }

    #[test]
    fn itf_needs_three_lines_whatever_the_setting() {
        let t = "12345678901231";
        let lines = |n: u32| {
            (0..n)
                .map(|i| hit(Symbology::Itf, t, 10.0 + 3.0 * i as f32, 0.0, 100.0, i))
                .collect::<Vec<_>>()
        };
        assert!(run(lines(2), &settings(1)).is_empty());
        assert_eq!(run(lines(3), &settings(1)).len(), 1);
        assert_eq!(run(lines(3), &settings(2)).len(), 1);
        assert!(run(lines(3), &settings(4)).is_empty());
    }

    #[test]
    fn codabar_needs_one_line_more_than_the_setting() {
        let lines = |n: u32, stops: (u8, u8)| {
            (0..n)
                .map(|i| {
                    let mut h = hit(
                        Symbology::Codabar,
                        "1234",
                        10.0 + 3.0 * i as f32,
                        0.0,
                        100.0,
                        i,
                    );
                    h.start_stop = Some(stops);
                    h
                })
                .collect::<Vec<_>>()
        };
        assert!(run(lines(1, (b'A', b'B')), &settings(1)).is_empty());
        assert_eq!(run(lines(2, (b'A', b'B')), &settings(1)).len(), 1);
        assert!(run(lines(2, (b'A', b'B')), &settings(2)).is_empty());
        let out = run(lines(3, (b'A', b'B')), &settings(2));
        assert_eq!(out.len(), 1);
        let SymbolMeta::Linear(meta) = &out[0].meta else {
            panic!("linear meta");
        };
        assert_eq!(meta.codabar_start_stop, Some((b'A', b'B')));
        assert!(run(lines(3, (b'A', b'B')), &settings(3)).is_empty());
        // The same payload with other guards is another symbol.
        let mut mixed = lines(2, (b'A', b'B'));
        mixed.extend(lines(2, (b'C', b'D')).into_iter().map(|mut h| {
            h.line += 2;
            h
        }));
        assert!(run(mixed, &settings(2)).is_empty());
    }

    #[test]
    fn a_partial_read_inside_a_bigger_symbol_is_never_emitted() {
        let full = "12345678901231";
        let partial = "345678901231";
        let build = |n_full: u32, n_part: u32, sym: Symbology, a: &str, b: &str| {
            let mut v = Vec::new();
            for i in 0..n_full {
                v.push(hit(sym, a, 10.0 + 3.0 * i as f32, 0.0, 100.0, i));
            }
            for i in 0..n_part {
                v.push(hit(sym, b, 10.0 + 3.0 * i as f32, 15.0, 90.0, 50 + i));
            }
            v
        };
        let itf = |f, p| build(f, p, Symbology::Itf, full, partial);
        // Equal support: both dropped. Clear majority: only the full one.
        assert!(run(itf(4, 4), &settings(2)).is_empty());
        assert!(run(itf(6, 4), &settings(2)).is_empty());
        let out = run(itf(8, 3), &settings(2));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, full.as_bytes());
        // Fewer lines than needed: the partial read is simply not reported.
        let out = run(itf(4, 2), &settings(2));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, full.as_bytes());

        let c39 = |f, p| build(f, p, Symbology::Code39, "ABC123XYZ", "123X");
        assert!(run(c39(3, 3), &settings(2)).is_empty());
        let out = run(c39(6, 2), &settings(2));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, b"ABC123XYZ");
    }

    #[test]
    fn same_line_twice_counts_once() {
        let h = hit(Symbology::Ean13, "5901234123457", 10.0, 0.0, 100.0, 3);
        assert!(run(vec![h.clone(), h], &settings(2)).is_empty());
    }

    #[test]
    fn distant_identical_hits_do_not_agree() {
        let hits = vec![
            hit(Symbology::Ean8, "73513537", 10.0, 0.0, 100.0, 0),
            hit(Symbology::Ean8, "73513537", 10.0, 900.0, 1000.0, 1),
        ];
        assert!(run(hits, &settings(2)).is_empty());
    }

    #[test]
    fn two_separate_symbols_with_the_same_text_are_reported_separately() {
        let t = "73513537";
        let hits = vec![
            hit(Symbology::Ean8, t, 10.0, 0.0, 100.0, 0),
            hit(Symbology::Ean8, t, 20.0, 0.0, 100.0, 1),
            hit(Symbology::Ean8, t, 10.0, 900.0, 1000.0, 0),
            hit(Symbology::Ean8, t, 20.0, 900.0, 1000.0, 1),
        ];
        assert_eq!(run(hits, &settings(2)).len(), 2);
    }

    #[test]
    fn identical_neighbours_stay_separate_symbols() {
        let t = "73513537";
        // Side by side, closer than one symbol length.
        let mut hits = Vec::new();
        for (i, x) in [0.0, 150.0].into_iter().enumerate() {
            for k in 0..3 {
                let line = (i * 3 + k) as u32;
                hits.push(hit(
                    Symbology::Ean8,
                    t,
                    10.0 + 6.0 * k as f32,
                    x,
                    x + 100.0,
                    line,
                ));
            }
        }
        assert_eq!(run(hits, &settings(2)).len(), 2);
        // Stacked, with a gap of several scanline steps.
        let mut hits = Vec::new();
        for (i, y) in [10.0, 90.0].into_iter().enumerate() {
            for k in 0..3 {
                let line = (i * 3 + k) as u32;
                hits.push(hit(
                    Symbology::Ean8,
                    t,
                    y + 6.0 * k as f32,
                    0.0,
                    100.0,
                    line,
                ));
            }
        }
        assert_eq!(run(hits, &settings(2)).len(), 2);
        // Rotated copies around a common center, as in a pinwheel.
        let mut hits = Vec::new();
        for i in 0..6u32 {
            let a = (i as f32 * 30.0).to_radians();
            let (s, c) = a.sin_cos();
            for k in 0..2u32 {
                let mut h = hit(Symbology::Ean8, t, 0.0, 0.0, 0.0, i * 2 + k);
                let (cx, cy) = (150.0 * c, 150.0 * s);
                let o = 6.0 * k as f32;
                h.from = Point::new(cx - 50.0 * c - o * s, cy - 50.0 * s + o * c);
                h.to = Point::new(cx + 50.0 * c - o * s, cy + 50.0 * s + o * c);
                hits.push(h);
            }
        }
        assert_eq!(run(hits, &settings(2)).len(), 6);
    }

    #[test]
    fn one_tall_symbol_is_one_group_however_many_lines_read_it() {
        let t = "73513537";
        // 40 lines six pixels apart, far taller than the symbol is long.
        let hits: Vec<_> = (0..40)
            .map(|k| hit(Symbology::Ean8, t, 10.0 + 6.0 * k as f32, 0.0, 100.0, k))
            .collect();
        let out = run(hits, &settings(2));
        assert_eq!(out.len(), 1);
        let SymbolMeta::Linear(meta) = &out[0].meta else {
            panic!("linear meta expected");
        };
        assert_eq!(meta.lines_agreeing, 40);
        // The same with a diagonal symbol and reversed reading directions.
        let hits: Vec<_> = (0..20u32)
            .map(|k| {
                let mut h = hit(Symbology::Ean8, t, 0.0, 0.0, 0.0, k);
                let o = 6.0 * k as f32;
                let (s, c) = 30.0f32.to_radians().sin_cos();
                h.from = Point::new(100.0 * c - o * s, 100.0 * s + o * c);
                h.to = Point::new(-o * s, o * c);
                if k % 2 == 1 {
                    std::mem::swap(&mut h.from, &mut h.to);
                }
                h
            })
            .collect();
        assert_eq!(run(hits, &settings(2)).len(), 1);
    }

    #[test]
    fn conflicting_texts_drop_the_weaker_only_with_a_two_to_one_majority() {
        let a = "5901234123457";
        let b = "4006381333931";
        let mk = |na: u32, nb: u32| {
            let mut v = Vec::new();
            for i in 0..na {
                v.push(hit(Symbology::Ean13, a, 10.0 + i as f32, 0.0, 100.0, i));
            }
            for i in 0..nb {
                v.push(hit(
                    Symbology::Ean13,
                    b,
                    10.0 + i as f32,
                    0.0,
                    100.0,
                    100 + i,
                ));
            }
            v
        };
        let out = run(mk(4, 2), &settings(2));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].data, a.as_bytes());
        assert!(run(mk(3, 2), &settings(2)).is_empty());
        assert!(run(mk(2, 2), &settings(2)).is_empty());
    }

    #[test]
    fn non_overlapping_different_symbols_coexist() {
        let hits = vec![
            hit(Symbology::Ean13, "5901234123457", 10.0, 0.0, 100.0, 0),
            hit(Symbology::Ean13, "5901234123457", 14.0, 0.0, 100.0, 1),
            hit(Symbology::Ean8, "73513537", 200.0, 0.0, 70.0, 2),
            hit(Symbology::Ean8, "73513537", 204.0, 0.0, 70.0, 3),
        ];
        assert_eq!(run(hits, &settings(2)).len(), 2);
    }

    #[test]
    fn contained_false_symbol_is_dropped_by_the_majority() {
        let mut hits = Vec::new();
        for i in 0..4 {
            hits.push(hit(
                Symbology::Ean13,
                "5901234123457",
                10.0 + 4.0 * i as f32,
                0.0,
                100.0,
                i,
            ));
        }
        for i in 0..2 {
            hits.push(hit(
                Symbology::UpcE,
                "15901236",
                10.0 + 4.0 * i as f32,
                0.0,
                45.0,
                10 + i,
            ));
        }
        let out = run(hits, &settings(2));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].symbology, Symbology::Ean13);
    }

    #[test]
    fn addon_needs_agreement_and_uniqueness() {
        let mk = |addons: &[Option<&str>]| {
            addons
                .iter()
                .enumerate()
                .map(|(i, a)| {
                    let mut h = hit(
                        Symbology::Ean13,
                        "5901234123457",
                        10.0 + i as f32 * 3.0,
                        0.0,
                        100.0,
                        i as u32,
                    );
                    h.addon = a.map(|s| s.as_bytes().to_vec());
                    h
                })
                .collect::<Vec<_>>()
        };
        let sym = |hits, s: &Settings| run(hits, s);
        let out = sym(mk(&[Some("12"), Some("12"), None]), &settings(2));
        let SymbolMeta::Linear(meta) = &out[0].meta else {
            panic!()
        };
        assert_eq!(meta.addon.as_deref(), Some("12"));
        assert_eq!(&out[0].aim_id, b"]E3");

        let out = sym(mk(&[Some("12"), None, None]), &settings(2));
        let SymbolMeta::Linear(meta) = &out[0].meta else {
            panic!()
        };
        assert_eq!(meta.addon, None);
        assert_eq!(&out[0].aim_id, b"]E0");

        let out = sym(
            mk(&[Some("12"), Some("12"), Some("34"), Some("34")]),
            &settings(2),
        );
        let SymbolMeta::Linear(meta) = &out[0].meta else {
            panic!()
        };
        assert_eq!(meta.addon, None);

        let required = Settings {
            addon: EanAddonPolicy::Required,
            ..settings(2)
        };
        assert!(sym(mk(&[Some("12"), None, None]), &required).is_empty());
        assert_eq!(sym(mk(&[Some("12"), Some("12")]), &required).len(), 1);
    }

    #[test]
    fn quad_is_mapped_to_the_original_frame_with_orientation() {
        let mut a = hit(Symbology::Ean8, "73513537", 10.0, 0.0, 100.0, 0);
        let mut b = hit(Symbology::Ean8, "73513537", 20.0, 0.0, 100.0, 1);
        a.quality = 1.0;
        b.quality = 0.0;
        let s = Settings {
            scale: (2.0, 2.0),
            ..settings(2)
        };
        let out = run(vec![a, b], &s);
        let q = out[0].quad;
        assert!((q.corners[0].x - 0.5).abs() < 1e-3);
        assert!((q.corners[1].x - 200.5).abs() < 1e-3);
        assert!(q.corners[0].y < q.corners[3].y);
        assert!(out[0].orientation_deg.abs() < 1e-3);
        assert!((out[0].quality - 0.5).abs() < 1e-3);
    }

    #[test]
    fn reversed_segments_give_a_half_turn_orientation() {
        let a = hit(Symbology::Ean8, "73513537", 10.0, 100.0, 0.0, 0);
        let b = hit(Symbology::Ean8, "73513537", 20.0, 100.0, 0.0, 1);
        let out = run(vec![a, b], &settings(2));
        assert!((out[0].orientation_deg - 180.0).abs() < 1e-3);
    }

    #[test]
    fn segment_distance_cases() {
        let p = Point::new;
        assert_eq!(
            segment_distance(p(0.0, 0.0), p(10.0, 10.0), p(0.0, 10.0), p(10.0, 0.0)),
            0.0
        );
        assert!(
            (segment_distance(p(0.0, 0.0), p(10.0, 0.0), p(0.0, 5.0), p(10.0, 5.0)) - 5.0).abs()
                < 1e-5
        );
        assert_eq!(
            segment_distance(p(0.0, 0.0), p(10.0, 0.0), p(5.0, 0.0), p(20.0, 0.0)),
            0.0
        );
    }
}
