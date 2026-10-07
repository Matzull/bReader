use crate::edges::Edges;

/// Alternating bar and space widths between consecutive edges.
///
/// Positions are always expressed in the coordinates of the profile the runs
/// came from, also after [`Runs::reverse_from`], where they decrease with the
/// run index. Decoders read widths through the accessors and never depend on
/// the direction.
#[derive(Debug, Clone, Default)]
pub struct Runs {
    first_is_bar: bool,
    widths: Vec<f32>,
    edges: Vec<f32>,
    lead: f32,
    trail: f32,
}

impl Runs {
    pub fn from_edges(edges: &Edges, out: &mut Self) {
        out.widths.clear();
        out.edges.clear();
        let items = edges.as_slice();
        let Some(first) = items.first() else {
            out.lead = 0.0;
            out.trail = 0.0;
            out.first_is_bar = false;
            return;
        };
        out.first_is_bar = !first.rising;
        out.lead = first.pos;
        out.trail = (edges.profile_len() as f32 - 1.0 - items[items.len() - 1].pos).max(0.0);
        out.edges.extend(items.iter().map(|e| e.pos));
        out.widths
            .extend(items.windows(2).map(|w| w[1].pos - w[0].pos));
    }

    /// Fills `self` with `src` read from the other end.
    pub fn reverse_from(src: &Self, out: &mut Self) {
        out.widths.clear();
        out.edges.clear();
        out.widths.extend(src.widths.iter().rev());
        out.edges.extend(src.edges.iter().rev());
        out.first_is_bar = !src.widths.is_empty() && src.is_bar(src.len() - 1);
        out.lead = src.trail;
        out.trail = src.lead;
    }

    /// Swaps the roles of bars and spaces: the same edges, read as a symbol
    /// of light bars on a dark background.
    pub fn flip_roles(&mut self) {
        self.first_is_bar = !self.first_is_bar;
    }

    /// Builds runs from explicit widths; used by tests and fuzzing.
    pub fn from_widths(first_is_bar: bool, widths: &[f32], lead: f32, trail: f32) -> Self {
        let mut edges = Vec::with_capacity(widths.len() + 1);
        let mut x = lead;
        edges.push(x);
        for w in widths {
            x += w;
            edges.push(x);
        }
        Self {
            first_is_bar,
            widths: widths.to_vec(),
            edges,
            lead,
            trail,
        }
    }

    pub fn len(&self) -> usize {
        self.widths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.widths.is_empty()
    }

    pub fn widths(&self) -> &[f32] {
        &self.widths
    }

    pub fn is_bar(&self, i: usize) -> bool {
        self.first_is_bar == i.is_multiple_of(2)
    }

    /// Profile position where run `i` begins, in reading order.
    pub fn start(&self, i: usize) -> f32 {
        self.edges[i]
    }

    /// Profile position where run `i` ends, in reading order.
    pub fn end(&self, i: usize) -> f32 {
        self.edges[i + 1]
    }

    /// Width of the light area before run `i`: the previous run when it is
    /// a space, otherwise the distance from the profile start (a lower bound).
    pub fn space_before(&self, i: usize) -> f32 {
        if i == 0 {
            if self.first_is_bar { self.lead } else { 0.0 }
        } else if self.is_bar(i - 1) {
            0.0
        } else {
            self.widths[i - 1]
        }
    }

    /// Width of the light area after run `i`, symmetrical to [`Self::space_before`].
    pub fn space_after(&self, i: usize) -> f32 {
        if i + 1 >= self.len() {
            if self.is_bar(self.len() - 1) {
                self.trail
            } else {
                0.0
            }
        } else if self.is_bar(i + 1) {
            0.0
        } else {
            self.widths[i + 1]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeDetector, EdgeParams};

    fn runs_of(profile: &[u8]) -> Runs {
        let mut edges = Edges::default();
        EdgeDetector::default().detect(profile, &EdgeParams::default(), &mut edges);
        let mut runs = Runs::default();
        Runs::from_edges(&edges, &mut runs);
        runs
    }

    #[test]
    fn widths_and_polarity_from_edges() {
        let mut p = vec![220u8; 60];
        p[10..14].fill(20);
        p[20..30].fill(20);
        let r = runs_of(&p);
        assert_eq!(r.len(), 3);
        assert!(r.is_bar(0) && !r.is_bar(1) && r.is_bar(2));
        let w = r.widths();
        assert!((w[0] - 4.0).abs() < 0.1 && (w[1] - 6.0).abs() < 0.1 && (w[2] - 10.0).abs() < 0.1);
        assert!((r.space_before(0) - 9.5).abs() < 0.1);
        assert!(
            (r.space_after(2) - 29.5).abs() < 0.2,
            "{}",
            r.space_after(2)
        );
    }

    #[test]
    fn profile_starting_in_a_bar_reports_a_space_first() {
        let mut p = vec![220u8; 60];
        p[..10].fill(20);
        p[30..].fill(20);
        let r = runs_of(&p);
        assert_eq!(r.len(), 1);
        assert!(!r.is_bar(0));
        assert_eq!(r.space_before(0), 0.0);
    }

    #[test]
    fn reverse_swaps_ends_and_polarity() {
        let src = Runs::from_widths(true, &[2.0, 3.0, 4.0, 5.0], 7.0, 11.0);
        let mut rev = Runs::default();
        Runs::reverse_from(&src, &mut rev);
        assert_eq!(rev.widths(), &[5.0, 4.0, 3.0, 2.0]);
        assert!(!rev.is_bar(0));
        assert!(rev.is_bar(1));
        assert_eq!(rev.space_before(1), 5.0);
        assert_eq!(rev.space_after(3), 7.0);
        assert!(rev.start(0) > rev.end(0));
        assert_eq!(rev.start(0), src.end(3));
        assert_eq!(rev.end(3), src.start(0));
    }

    #[test]
    fn flipped_roles_swap_bars_and_quiet_zones() {
        let mut r = Runs::from_widths(true, &[2.0, 3.0, 4.0], 7.0, 11.0);
        r.flip_roles();
        assert!(!r.is_bar(0) && r.is_bar(1) && !r.is_bar(2));
        assert_eq!(r.space_before(0), 0.0);
        assert_eq!(r.space_before(1), 2.0);
        assert_eq!(r.space_after(1), 4.0);
        let mut rev = Runs::default();
        Runs::reverse_from(&r, &mut rev);
        assert!(!rev.is_bar(0) && rev.is_bar(1));
        r.flip_roles();
        assert!(r.is_bar(0));
    }

    #[test]
    fn empty_runs_do_not_panic() {
        let r = runs_of(&[128; 100]);
        assert!(r.is_empty());
        let mut rev = Runs::default();
        Runs::reverse_from(&r, &mut rev);
        assert!(rev.is_empty());
    }
}
