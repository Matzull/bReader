use breader_core::{
    Content, EanAddonPolicy, LinearMeta, Point, Quad, Symbol, SymbolMeta, Symbology,
};

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
/// Upper bound on hits kept per pass, a guard against pathological input.
const MAX_HITS: usize = 4096;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Settings {
    pub min_agreement: u16,
    /// Lines with an index below this belong to the sparse scan plan; higher
    /// ones were added around a lone hit. Lines next to each other see almost
    /// the same pixels, so a group backed by a single plan line needs one
    /// more agreeing line than `min_agreement`.
    pub plan_lines: u32,
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

/// Collects the hits of one pass and turns agreeing ones into symbols.
#[derive(Debug, Clone, Default)]
pub(crate) struct Aggregator {
    hits: Vec<PlacedHit>,
    groups: Vec<Group>,
}

impl Aggregator {
    /// Forgets the hits of the previous pass; keeps the allocations.
    pub fn reset(&mut self) {
        self.hits.clear();
        self.groups.clear();
    }

    pub fn push(&mut self, hit: PlacedHit) {
        if self.hits.len() < MAX_HITS {
            self.hits.push(hit);
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

    /// One representative hit of every group that lacks the agreement
    /// required by `settings`.
    pub fn seeds(&mut self, settings: &Settings, out: &mut Vec<PlacedHit>) {
        out.clear();
        self.group();
        for g in &self.groups {
            let lines = distinct_lines(&self.hits, &g.members);
            if lines < self.required(&g.members, settings) {
                out.push(self.hits[g.members[0]].clone());
            }
        }
    }

    fn required(&self, members: &[usize], settings: &Settings) -> u16 {
        let mut min = settings.min_agreement.max(1);
        match self.hits[members[0]].symbology {
            Symbology::Itf => min = min.max(ITF_MIN_AGREEMENT),
            // No check character either: one more line than the setting asks.
            Symbology::Codabar => min += 1,
            _ => {}
        }
        let sparse: Vec<usize> = members
            .iter()
            .copied()
            .filter(|&m| self.hits[m].line < settings.plan_lines)
            .collect();
        let mut need = if min >= 2 && distinct_lines(&self.hits, &sparse) < 2 {
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

    /// Groups hits by symbol identity and proximity, applies the agreement
    /// and conflict rules and appends the surviving symbols to `out`.
    pub fn finish(&mut self, settings: &Settings, out: &mut Vec<Symbol>) {
        self.group();
        let min = settings.min_agreement.max(1);
        let mut candidates: Vec<Candidate> = Vec::new();
        for g in &self.groups {
            let lines = distinct_lines(&self.hits, &g.members);
            if lines < self.required(&g.members, settings) {
                continue;
            }
            let addon = self.choose_addon(&g.members, min);
            if settings.addon == EanAddonPolicy::Required && addon.is_none() {
                continue;
            }
            candidates.push(Candidate {
                members: g.members.clone(),
                lines,
                addon,
                frame: self.frame(&g.members, settings.across_pad),
            });
        }

        for (i, c) in candidates.iter().enumerate() {
            let survives = candidates.iter().enumerate().all(|(j, o)| {
                i == j
                    || self.hits[c.members[0]].same_symbol(&self.hits[o.members[0]])
                    || !self.overlap(c, o)
                    || u32::from(c.lines) >= 2 * u32::from(o.lines)
            });
            if survives {
                out.push(self.symbol(c, settings));
            }
        }
    }

    fn group(&mut self) {
        self.groups.clear();
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
                        let moved = self.groups.remove(g).members;
                        self.groups[first].members.extend(moved);
                    }
                }
            }
            if joined.is_none() {
                self.groups.push(Group { members: vec![i] });
            }
        }
    }

    /// The supplement read by at least `min` distinct lines, if exactly one.
    fn choose_addon(&self, members: &[usize], min: u16) -> Option<Vec<u8>> {
        let mut found: Vec<&Vec<u8>> = Vec::new();
        for &m in members {
            if let Some(a) = &self.hits[m].addon
                && !found.contains(&a)
            {
                found.push(a);
            }
        }
        let mut agreed = found.into_iter().filter(|a| {
            let lines: Vec<usize> = members
                .iter()
                .copied()
                .filter(|&m| self.hits[m].addon.as_ref() == Some(*a))
                .collect();
            distinct_lines(&self.hits, &lines) >= min
        });
        let first = agreed.next()?;
        agreed.next().is_none().then(|| first.clone())
    }

    fn frame(&self, members: &[usize], pad: f32) -> Frame {
        let (mut ux, mut uy) = (0.0f32, 0.0f32);
        let (mut cx, mut cy) = (0.0f32, 0.0f32);
        for &m in members {
            let h = &self.hits[m];
            let len = h.length().max(1e-6);
            ux += (h.to.x - h.from.x) / len;
            uy += (h.to.y - h.from.y) / len;
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
        Symbol {
            symbology: first.symbology,
            data: first.text.clone(),
            text: None,
            segments: Vec::new(),
            content: Content::Unparsed,
            orientation_deg: quad.orientation_deg(),
            quad,
            aim_id,
            meta: SymbolMeta::Linear(LinearMeta {
                addon: c
                    .addon
                    .as_ref()
                    .and_then(|a| String::from_utf8(a.clone()).ok()),
                checksum_verified: first.checksum_verified,
                lines_agreeing: c.lines,
                codabar_start_stop: first.start_stop,
                databar_segments: (first.segments > 0).then_some(first.segments),
            }),
            quality: quality.clamp(0.0, 1.0),
        }
    }
}

fn distinct_lines(hits: &[PlacedHit], members: &[usize]) -> u16 {
    let mut seen: Vec<u32> = Vec::with_capacity(members.len());
    for &m in members {
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
        }
    }

    fn settings(min: u16) -> Settings {
        Settings {
            min_agreement: min,
            plan_lines: u32::MAX,
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
