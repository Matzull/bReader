/// A luminance transition along a profile.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Edge {
    /// Sub-pixel position in profile coordinates (sample `i` is at `i`).
    pub pos: f32,
    /// Magnitude of the smoothed central difference.
    pub strength: f32,
    /// Dark to light when true; light to dark (start of a bar) otherwise.
    pub rising: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Edges {
    items: Vec<Edge>,
    profile_len: usize,
}

impl Edges {
    pub fn as_slice(&self) -> &[Edge] {
        &self.items
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn profile_len(&self) -> usize {
        self.profile_len
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EdgeParams {
    /// Smallest accepted gradient, in gray levels across two pixels.
    pub min_edge: f32,
    /// Smallest accepted gradient as a fraction of the local range.
    pub relative: f32,
    /// Local ranges below this are treated as flat.
    pub min_range: f32,
    /// Gradient threshold in multiples of the gradient noise estimate.
    pub noise_factor: f32,
    /// Largest threshold as a fraction of the local range; keeps heavy
    /// noise estimates from hiding genuine edges.
    pub max_relative: f32,
    /// Smooth with [1 2 1] before differentiating. Suppresses noise but
    /// merges the gradients of very narrow bars.
    pub smooth: bool,
    /// Relocate each edge to the half-level crossing between the extremes of
    /// its neighboring runs.
    pub refine: bool,
}

impl EdgeParams {
    /// Direct differentiation: keeps bars of about 1.5 px resolved.
    pub const fn fine() -> Self {
        let mut p = Self::new();
        p.refine = true;
        p
    }

    /// Smoothed variant for noisy profiles of wider modules.
    pub const fn smoothed() -> Self {
        let mut p = Self::new();
        p.smooth = true;
        p.refine = true;
        p
    }

    /// The same variant without edge relocation.
    #[cfg(test)]
    pub const fn raw(mut self) -> Self {
        self.refine = false;
        self
    }

    const fn new() -> Self {
        Self {
            min_edge: 6.0,
            relative: 0.12,
            min_range: 28.0,
            noise_factor: 3.5,
            max_relative: 0.3,
            smooth: false,
            refine: false,
        }
    }
}

impl Default for EdgeParams {
    fn default() -> Self {
        Self::new()
    }
}

const BLOCK: usize = 16;
/// Half-width of the search for the half-level crossing of an edge.
const CROSSING_REACH: usize = 3;
/// Smallest distance kept between neighboring edges after relocation.
const MIN_RUN: f32 = 0.05;
/// Largest distance from an edge at which plateau levels are measured.
const PLATEAU_REACH: f32 = 6.0;
/// Blocks on each side of a block that contribute to its local range.
const RANGE_REACH: usize = 2;
/// A block whose range stays below this fraction of the line's range is
/// treated as flat when estimating noise.
const FLAT_BLOCK: f32 = 0.6;

fn extreme(samples: &[u8], max: bool) -> f32 {
    let it = samples.iter().copied();
    f32::from(if max { it.max() } else { it.min() }.unwrap_or(0))
}

/// Reusable buffers for edge detection.
#[derive(Debug, Clone, Default)]
pub struct EdgeDetector {
    diff: Vec<i32>,
    block_min: Vec<u8>,
    block_max: Vec<u8>,
    block_noise: Vec<u8>,
    threshold: Vec<i32>,
}

impl EdgeDetector {
    /// Finds alternating edges in `profile` (dark = low).
    ///
    /// Edges are local extrema of the central difference whose magnitude
    /// exceeds `min_edge`, `relative` times the range of the profile in the
    /// surrounding blocks (which keeps the detector independent of uneven
    /// illumination) and a multiple of the estimated noise. Optionally the
    /// profile is first smoothed with [1 2 1] and edges are relocated to the
    /// half-level crossing. Consecutive same-sign extrema collapse into the
    /// strongest.
    pub fn detect(&mut self, profile: &[u8], params: &EdgeParams, out: &mut Edges) {
        out.items.clear();
        out.profile_len = profile.len();
        let n = profile.len();
        if n < 8 {
            return;
        }
        let noise = self.block_statistics(profile);
        let noise_d = noise * if params.smooth { 0.79 } else { 1.41 };
        self.thresholds(params, noise_d);
        if self.threshold.iter().all(|&t| t == i32::MAX) {
            return;
        }
        if self.diff.len() < n {
            self.diff.resize(n, 0);
        }

        for b in 0..self.threshold.len() {
            let threshold = self.threshold[b];
            if threshold == i32::MAX {
                continue;
            }
            let lo = (b * BLOCK).max(1);
            let hi = ((b + 1) * BLOCK).min(n - 1);
            self.differentiate(profile, lo - 1, (hi + 1).min(n - 1), params.smooth);
            let d = &self.diff;
            for i in lo..hi {
                let v = d[i];
                let sign = v.signum();
                let (a, b, c) = (sign * d[i - 1], sign * v, sign * d[i + 1]);
                if b < threshold || b < a || b <= c {
                    continue;
                }
                let denom = (a - 2 * b + c) as f32;
                let offset = if denom < 0.0 {
                    (0.5 * (a - c) as f32 / denom).clamp(-0.5, 0.5)
                } else {
                    0.0
                };
                let edge = Edge {
                    pos: i as f32 + offset,
                    strength: b as f32 / 4.0,
                    rising: sign > 0,
                };
                match out.items.last_mut() {
                    Some(last) if last.rising == edge.rising => {
                        if edge.strength > last.strength {
                            *last = edge;
                        }
                    }
                    _ => out.items.push(edge),
                }
            }
        }
        if params.refine {
            Self::refine_edges(profile, &mut out.items);
        }
    }

    /// Moves every edge to where the profile crosses the middle between the
    /// extremes on either side. Gradient peaks are pulled around by
    /// neighboring edges when bars and spaces are narrow or unevenly inked;
    /// the half-level crossing is not. The levels come from the samples up to
    /// `PLATEAU_REACH` away, limited by the neighboring edges, so narrow runs
    /// use their own peak as reference.
    fn refine_edges(profile: &[u8], items: &mut [Edge]) {
        let last = profile.len() - 1;
        for k in 1..items.len().saturating_sub(1) {
            let e = items[k];
            let prev = items[k - 1].pos.max(e.pos - PLATEAU_REACH);
            let next = items[k + 1].pos.min(e.pos + PLATEAU_REACH);
            let (p, c, n) = (
                prev.ceil() as usize,
                e.pos.round() as usize,
                next.floor() as usize,
            );
            if p >= c || c >= n || n > last {
                continue;
            }
            let (before, after) = if e.rising {
                (
                    extreme(&profile[p..=c], false),
                    extreme(&profile[c..=n], true),
                )
            } else {
                (
                    extreme(&profile[p..=c], true),
                    extreme(&profile[c..=n], false),
                )
            };
            let mid = (before + after) / 2.0;
            let lo = c.saturating_sub(CROSSING_REACH);
            let hi = (c + CROSSING_REACH).min(last - 1);
            let mut best: Option<(f32, f32)> = None;
            for i in lo..=hi {
                let (p0, p1) = (f32::from(profile[i]), f32::from(profile[i + 1]));
                if p0 != p1 && (p0 - mid) * (p1 - mid) <= 0.0 && (p1 > p0) == e.rising {
                    let x = i as f32 + (mid - p0) / (p1 - p0);
                    let d = (x - e.pos).abs();
                    if best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, x));
                    }
                }
            }
            if let Some((_, x)) = best {
                let (lo, hi) = (items[k - 1].pos + MIN_RUN, items[k + 1].pos - MIN_RUN);
                if lo <= hi {
                    items[k].pos = x.clamp(lo, hi);
                }
            }
        }
    }

    /// Fills the per-block min and max and returns the noise standard
    /// deviation in gray levels, estimated from blocks that contain no major
    /// edge. The estimate is 0 when no such block exists, as in a profile
    /// that is dense with narrow bars from end to end.
    fn block_statistics(&mut self, p: &[u8]) -> f32 {
        self.block_min.clear();
        self.block_max.clear();
        let (mut lo, mut hi) = (255u8, 0u8);
        for chunk in p.chunks(BLOCK) {
            let (l, h) = chunk
                .iter()
                .fold((255u8, 0u8), |(l, h), &v| (l.min(v), h.max(v)));
            self.block_min.push(l);
            self.block_max.push(h);
            lo = lo.min(l);
            hi = hi.max(h);
        }
        let flat_limit = FLAT_BLOCK * f32::from(hi - lo);
        self.block_noise.clear();
        for (b, chunk) in p.chunks_exact(BLOCK).enumerate() {
            if f32::from(self.block_max[b] - self.block_min[b]) > flat_limit {
                continue;
            }
            let sum: u32 = chunk
                .windows(2)
                .map(|w| u32::from(w[0].abs_diff(w[1])))
                .sum();
            self.block_noise.push((sum / (BLOCK as u32 - 1)) as u8);
        }
        if self.block_noise.is_empty() {
            return 0.0;
        }
        self.block_noise.sort_unstable();
        // Mean absolute difference of Gaussian noise is 1.128 sigma.
        f32::from(self.block_noise[self.block_noise.len() / 2]) / 1.128
    }

    /// Central difference of the (optionally smoothed) profile for samples
    /// `lo..=hi`, scaled by 4 in both variants so that thresholds do not
    /// depend on `smooth`. Samples closer than the filter reach to the
    /// profile ends stay 0.
    fn differentiate(&mut self, p: &[u8], lo: usize, hi: usize, smooth: bool) {
        let n = p.len();
        let reach = if smooth { 2 } else { 1 };
        for slot in &mut self.diff[lo..=hi] {
            *slot = 0;
        }
        let (a, b) = (lo.max(reach), hi.min(n - 1 - reach));
        if a > b {
            return;
        }
        let out = &mut self.diff[a..=b];
        if smooth {
            for (slot, w) in out.iter_mut().zip(p[a - 2..=b + 2].windows(5)) {
                *slot =
                    i32::from(w[4]) + 2 * i32::from(w[3]) - 2 * i32::from(w[1]) - i32::from(w[0]);
            }
        } else {
            for (slot, w) in out.iter_mut().zip(p[a - 1..=b + 1].windows(3)) {
                *slot = 4 * (i32::from(w[2]) - i32::from(w[0]));
            }
        }
    }

    fn thresholds(&mut self, params: &EdgeParams, noise_d: f32) {
        let blocks = self.block_min.len();
        self.threshold.clear();
        for b in 0..blocks {
            let lo = b.saturating_sub(RANGE_REACH);
            let hi = (b + RANGE_REACH).min(blocks - 1);
            let max = self.block_max[lo..=hi].iter().copied().max().unwrap_or(0);
            let min = self.block_min[lo..=hi].iter().copied().min().unwrap_or(0);
            let range = f32::from(max - min);
            self.threshold.push(if range < params.min_range {
                i32::MAX
            } else {
                let noise_limit = (params.noise_factor * noise_d).min(params.max_relative * range);
                (4.0 * params.min_edge)
                    .max(4.0 * params.relative * range)
                    .max(4.0 * noise_limit) as i32
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::render_intervals;

    /// Renders space/bar widths (px, first is a space) with Gaussian blur.
    fn render(widths: &[f32], lead: f32, dark: f32, light: f32, sigma: f32) -> Vec<u8> {
        let mut x = lead;
        let mut bars = Vec::new();
        for (k, w) in widths.iter().enumerate() {
            if k % 2 == 1 {
                bars.push((x, x + w));
            }
            x += w;
        }
        render_intervals(&bars, (x + lead).ceil() as usize, dark, light, sigma)
    }

    fn detect(profile: &[u8]) -> Edges {
        let mut out = Edges::default();
        EdgeDetector::default().detect(profile, &EdgeParams::default(), &mut out);
        out
    }

    #[test]
    fn sharp_steps_have_exact_subpixel_positions() {
        let mut p = vec![200u8; 40];
        p[10..20].fill(30);
        let e = detect(&p);
        let pos: Vec<f32> = e.as_slice().iter().map(|e| e.pos).collect();
        assert_eq!(e.len(), 2);
        assert!((pos[0] - 9.5).abs() < 0.05, "{pos:?}");
        assert!((pos[1] - 19.5).abs() < 0.05, "{pos:?}");
        assert!(!e.as_slice()[0].rising && e.as_slice()[1].rising);
    }

    #[test]
    fn fractional_edges_are_recovered_after_blur() {
        for sigma in [0.0, 0.8, 1.5] {
            for frac in [0.0f32, 0.25, 0.5, 0.75] {
                let widths = [12.0 + frac, 6.3, 9.0, 4.4, 20.0];
                let p = render(&widths, 25.0, 20.0, 220.0, sigma);
                let e = detect(&p);
                assert_eq!(e.len(), 4, "sigma {sigma} frac {frac}");
                let mut x = 25.0 + widths[0];
                for (k, edge) in e.as_slice().iter().enumerate() {
                    assert!(
                        (edge.pos - x).abs() < 0.35,
                        "sigma {sigma} edge {k}: {} vs {x}",
                        edge.pos
                    );
                    x += widths[k + 1];
                }
            }
        }
    }

    /// Mean absolute edge position error over random narrow runs.
    fn mean_error(params: &EdgeParams, sigma: f32, seed: u64) -> (f32, usize) {
        let mut rng = crate::testutil::Rng(seed);
        let (mut total, mut count) = (0.0, 0);
        for _ in 0..60 {
            let widths: Vec<f32> = (0..24).map(|_| 1.4 + rng.unit() * 2.6).collect();
            let lead = 20.0 + rng.unit();
            let p = render(&widths, lead, 25.0, 225.0, sigma);
            let mut out = Edges::default();
            EdgeDetector::default().detect(&p, params, &mut out);
            if out.len() != widths.len() {
                continue;
            }
            let mut x = lead;
            for (edge, w) in out.as_slice().iter().zip(&widths) {
                x += w;
                total += (edge.pos - x).abs();
                count += 1;
            }
        }
        (total / count.max(1) as f32, count)
    }

    #[test]
    fn half_level_relocation_improves_narrow_run_accuracy() {
        for sigma in [0.6, 0.9] {
            let (raw, n_raw) = mean_error(&EdgeParams::fine().raw(), sigma, 5);
            let (refined, n_ref) = mean_error(&EdgeParams::fine(), sigma, 5);
            assert!(n_raw > 500 && n_ref > 500, "{n_raw} {n_ref}");
            assert!(
                refined < raw,
                "sigma {sigma}: refined {refined} vs raw {raw}"
            );
            assert!(refined < 0.25, "sigma {sigma}: {refined}");
        }
    }

    #[test]
    fn illumination_gradient_does_not_move_or_hide_edges() {
        let widths = [10.0, 5.0, 5.0, 5.0, 10.0, 5.0, 10.0];
        let base = render(&widths, 30.0, 10.0, 230.0, 1.0);
        let n = base.len() as f32;
        let shaded: Vec<u8> = base
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let g = 0.35 + 0.65 * (i as f32 / n);
                (f32::from(v) * g) as u8
            })
            .collect();
        let a = detect(&base);
        let b = detect(&shaded);
        assert_eq!(a.len(), b.len());
        for (x, y) in a.as_slice().iter().zip(b.as_slice()) {
            assert!((x.pos - y.pos).abs() < 0.5, "{} vs {}", x.pos, y.pos);
        }
    }

    #[test]
    fn flat_and_low_contrast_profiles_yield_no_edges() {
        assert!(detect(&[128; 200]).is_empty());
        let mut p = vec![120u8; 100];
        p[40..60].fill(130);
        assert!(detect(&p).is_empty());
        assert!(detect(&[1, 2, 3]).is_empty());
        assert!(detect(&[]).is_empty());
    }

    #[test]
    fn edges_alternate_in_polarity() {
        let mut rng = 0x1234_5678_9abc_def1u64;
        let mut p = Vec::new();
        for _ in 0..2000 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            p.push((rng >> 24) as u8);
        }
        let e = detect(&p);
        assert!(e.as_slice().windows(2).all(|w| w[0].rising != w[1].rising));
        assert!(e.as_slice().windows(2).all(|w| w[0].pos < w[1].pos));
    }

    #[test]
    fn noise_does_not_create_edges_inside_bars() {
        let widths = [10.0, 8.0, 8.0, 8.0, 8.0, 8.0, 10.0];
        let mut p = render(&widths, 20.0, 40.0, 210.0, 0.8);
        let mut rng = 99u64;
        for v in p.iter_mut() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let noise = (rng >> 40) as i32 % 21 - 10;
            *v = (i32::from(*v) + noise).clamp(0, 255) as u8;
        }
        assert_eq!(detect(&p).len(), 6);
    }
}
