//! Gray profiles along arbitrary lines, turned into runs and pattern marks.

use breader_core::{ImageView, Point};
use breader_linear::stacked::{
    EdgeDetector, EdgeParams, Edges, Oblique, RawMark, Runs, Scanline, find,
};

use crate::geom::{Vec2, at};

/// Shortest profile worth looking at, in samples.
const MIN_SAMPLES: usize = 16;
/// Light samples added where the image border cuts a line: a symbol cropped
/// without quiet zone then reads as if it had one.
const BORDER_PAD: f32 = 24.0;

/// Buffers for sampling one line at a time.
#[derive(Debug, Default)]
pub(crate) struct Scanner {
    detector: EdgeDetector,
    edges: Edges,
    pub runs: Runs,
    profile: Vec<u8>,
    /// Patterns found on the last loaded line.
    pub marks: Vec<RawMark>,
    /// Line parameter of profile sample 0.
    t0: f32,
}

/// Range of `t` for which `from + dir * t` stays inside the image.
fn clip(from: Point, dir: Vec2, t_lo: f32, t_hi: f32, w: usize, h: usize) -> Option<(f32, f32)> {
    let (mut lo, mut hi) = (t_lo, t_hi);
    for (p, d, max) in [
        (from.x, dir.0, w as f32 - 1.0),
        (from.y, dir.1, h as f32 - 1.0),
    ] {
        if d.abs() < 1e-9 {
            if !(0.0..=max).contains(&p) {
                return None;
            }
        } else {
            let (a, b) = ((0.0 - p) / d, (max - p) / d);
            lo = lo.max(a.min(b));
            hi = hi.min(a.max(b));
        }
    }
    (hi - lo >= MIN_SAMPLES as f32).then_some((lo, hi))
}

impl Scanner {
    /// Samples `from + dir * t` for `t` in `t_lo..=t_hi` (clipped to the
    /// image) and builds its runs and marks. With `inverted` the profile is
    /// flipped so that the bars of a light-on-dark symbol are dark.
    /// Returns false when nothing usable is left.
    pub(crate) fn load(
        &mut self,
        img: &ImageView<'_>,
        from: Point,
        dir: Vec2,
        range: (f32, f32),
        inverted: bool,
        params: &EdgeParams,
    ) -> bool {
        self.marks.clear();
        // A line along the pixel grid is read on whole pixels: interpolating
        // between pixels would blur modules that are only a few pixels wide.
        let axis = (dir.0.abs() == 1.0 && dir.1 == 0.0) || (dir.0 == 0.0 && dir.1.abs() == 1.0);
        let snapped = if axis {
            Point::new(from.x.round(), from.y.round())
        } else {
            from
        };
        let shift = (snapped.x - from.x) * dir.0 + (snapped.y - from.y) * dir.1;
        let range = (range.0 - shift, range.1 - shift);
        let from = snapped;
        let Some((lo, hi)) = clip(from, dir, range.0, range.1, img.width(), img.height()) else {
            return false;
        };
        let line = Scanline::new(at(from, dir, lo), at(from, dir, hi));
        let oblique = Oblique::new(&line);
        let pad_lo = (lo - range.0).clamp(0.0, BORDER_PAD).ceil() as usize;
        let pad_hi = (range.1 - hi).clamp(0.0, BORDER_PAD).ceil() as usize;
        self.profile.clear();
        self.profile.resize(pad_lo + oblique.len + pad_hi, 255);
        let body = pad_lo..pad_lo + oblique.len;
        oblique.sample(img, 0, inverted, &mut self.profile[body.clone()]);
        // Pad with the light level of the line, not a fixed white.
        if pad_lo + pad_hi > 0 {
            let light = self.profile[body.clone()]
                .iter()
                .copied()
                .max()
                .unwrap_or(255);
            self.profile[..pad_lo].fill(light);
            let n = self.profile.len();
            self.profile[body.end..n].fill(light);
        }
        self.t0 = lo + shift - pad_lo as f32;
        self.detector.detect(&self.profile, params, &mut self.edges);
        Runs::from_edges(&self.edges, &mut self.runs);
        if self.runs.len() < 8 {
            return false;
        }
        find(&self.runs, &mut self.marks);
        true
    }

    /// Line parameter of profile position `p`.
    pub(crate) fn t_of(&self, p: f32) -> f32 {
        self.t0 + p
    }

    /// Profile position of line parameter `t`.
    pub(crate) fn pos_of(&self, t: f32) -> f32 {
        t - self.t0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use breader_linear::stacked::MarkKind;

    /// A row of PDF417 start pattern at `module` px, from x = 20.
    fn image_with_start(module: usize) -> (Vec<u8>, usize, usize) {
        let (w, h) = (200, 12);
        let widths = [8, 1, 1, 1, 1, 1, 1, 3];
        let mut row = vec![230u8; w];
        let mut x = 20;
        for (i, &e) in widths.iter().enumerate() {
            if i % 2 == 0 {
                row[x..x + e * module].fill(20);
            }
            x += e * module;
        }
        // The first bar of the next codeword closes the last space.
        row[x..x + 2 * module].fill(20);
        let data = (0..h).flat_map(|_| row.clone()).collect();
        (data, w, h)
    }

    #[test]
    fn loads_runs_and_marks_with_positions_in_line_coordinates() {
        let (data, w, h) = image_with_start(2);
        let img = ImageView::from_packed(&data, w, h).unwrap();
        let mut s = Scanner::default();
        let from = Point::new(10.0, 5.0);
        assert!(s.load(
            &img,
            from,
            (1.0, 0.0),
            (-5.0, 150.0),
            false,
            &EdgeParams::fine()
        ));
        let start = s
            .marks
            .iter()
            .find(|m| m.kind == MarkKind::Start)
            .expect("start pattern");
        assert!((start.module - 2.0).abs() < 0.2);
        // The pattern begins at x = 20 - 0.5: 10 px after `from`.
        let x = from.x + s.t_of(start.from);
        assert!((x - 19.5).abs() < 1.0, "{x}");
        assert!((s.pos_of(s.t_of(7.0)) - 7.0).abs() < 1e-6);
    }

    #[test]
    fn lines_outside_the_image_are_refused() {
        let (data, w, h) = image_with_start(2);
        let img = ImageView::from_packed(&data, w, h).unwrap();
        let mut s = Scanner::default();
        let p = Point::new(10.0, 500.0);
        assert!(!s.load(
            &img,
            p,
            (1.0, 0.0),
            (0.0, 100.0),
            false,
            &EdgeParams::fine()
        ));
    }
}
