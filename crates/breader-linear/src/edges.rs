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
    /// With `refine`, blend the reference levels with the extremes of a window
    /// scaled by the local run width (see `EdgeDetector::refine_edges`).
    pub window: bool,
    /// Edges are the crossings of the profile with its local mean, found with
    /// a Schmitt trigger and placed by linear interpolation, instead of
    /// gradient extrema (`EdgeDetector::detect_crossing`). Sees runs of one
    /// sample, which a central difference cannot resolve; `smooth`, `refine`
    /// and `window` are ignored.
    pub crossing: bool,
}

impl EdgeParams {
    /// Direct differentiation: keeps bars of about 1.5 px resolved.
    pub const fn fine() -> Self {
        let mut p = Self::new();
        p.refine = true;
        p.window = true;
        p
    }

    /// Smoothed variant for noisy profiles of wider modules.
    pub const fn smoothed() -> Self {
        let mut p = Self::new();
        p.smooth = true;
        p.refine = true;
        p.window = true;
        p
    }

    /// The same variant without edge relocation.
    #[cfg(test)]
    pub const fn raw(mut self) -> Self {
        self.refine = false;
        self
    }

    /// The same variant relocating with the neighboring runs only.
    #[cfg(test)]
    pub const fn local(mut self) -> Self {
        self.window = false;
        self
    }

    /// Level crossings of the local mean: resolves bars and spaces of about one
    /// sample that the gradient variants merge. Tried after them.
    pub const fn crossing() -> Self {
        let mut p = Self::new();
        p.crossing = true;
        p
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
            window: false,
            crossing: false,
        }
    }
}

impl Default for EdgeParams {
    fn default() -> Self {
        Self::new()
    }
}

const BLOCK: usize = 16;
/// Half-width of the window used for the second reference level, in mean run
/// widths around the edge, and its limits in samples.
const WINDOW_RUNS: f32 = 8.0;
const WINDOW_MIN: f32 = 8.0;
const WINDOW_MAX: f32 = 64.0;
/// Edges on each side whose spacing gives the mean run width.
const WINDOW_SPAN: usize = 4;
/// Largest shift of a reference level from its local value, as a fraction of
/// the local level difference.
const WINDOW_SHIFT: f32 = 0.25;
/// Half-width of the search for the half-level crossing of an edge.
const CROSSING_REACH: usize = 3;
/// Samples on each side of a sample in the mean that `detect_crossing`
/// compares it with.
const CROSSING_WINDOW: usize = 6;
/// Distance from the mean at which the crossing detector changes state, as a
/// fraction of the local range.
const CROSSING_HYSTERESIS: f32 = 0.12;
/// Smallest distance kept between neighboring edges after relocation.
const MIN_RUN: f32 = 0.05;
/// Largest distance from an edge at which plateau levels are measured.
const PLATEAU_REACH: f32 = 6.0;
/// Blocks on each side of a block that contribute to its local range.
const RANGE_REACH: usize = 2;
/// A block whose range stays below this fraction of the line's range is
/// treated as flat when estimating noise.
const FLAT_BLOCK: f32 = 0.6;

/// Half-width in samples of the window around edge `k`, scaled by the mean
/// run width of its neighborhood.
fn window_reach(items: &[Edge], k: usize) -> usize {
    let a = k.saturating_sub(WINDOW_SPAN);
    let b = (k + WINDOW_SPAN).min(items.len() - 1);
    let mean = (items[b].pos - items[a].pos) / (b - a) as f32;
    (WINDOW_RUNS * mean).clamp(WINDOW_MIN, WINDOW_MAX) as usize
}

/// Mean of a local and a window level, kept within `cap` of the local one.
fn blend(local: f32, window: f32, cap: f32) -> i32 {
    (0.5 * (local + window)).clamp(local - cap, local + cap) as i32
}

fn low(samples: &[u8]) -> u8 {
    samples.iter().fold(255, |m, &v| m.min(v))
}

fn high(samples: &[u8]) -> u8 {
    samples.iter().fold(0, |m, &v| m.max(v))
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
        if params.crossing {
            self.detect_crossing(profile, params, out);
            return;
        }
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
                if v.abs() < threshold {
                    continue;
                }
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
            Self::refine_edges(profile, &mut out.items, params.window);
        }
    }

    /// Edges as the crossings of the profile with its mean over
    /// `CROSSING_WINDOW` samples on each side. A Schmitt trigger decides which
    /// crossings count: the state changes only when the profile is at least
    /// `CROSSING_HYSTERESIS` of the local range (and `min_edge`) beyond the
    /// mean on the other side, and the edge is placed at the last crossing
    /// before that, by linear interpolation between the two samples around it.
    /// A bar or space of one sample is an edge pair like any other, where the
    /// central difference of its two neighbours is blind to it. Blocks whose
    /// local range is below `min_range` hold no edge.
    fn detect_crossing(&mut self, profile: &[u8], params: &EdgeParams, out: &mut Edges) {
        let n = profile.len();
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
                params.min_edge.max(CROSSING_HYSTERESIS * range) as i32
            });
        }
        // Profile value minus the middle of the local extremes, times two.
        let level = |i: usize| -> (i64, i64) {
            let lo = i.saturating_sub(CROSSING_WINDOW);
            let hi = (i + CROSSING_WINDOW + 1).min(n);
            let (l, h) = (low(&profile[lo..hi]), high(&profile[lo..hi]));
            (2 * i64::from(profile[i]) - i64::from(l) - i64::from(h), 2)
        };
        let mut state = 0i8;
        let mut last_same = 0usize;
        for i in 0..n {
            let (d, count) = level(i);
            let h = self.threshold[i / BLOCK];
            if h == i32::MAX {
                continue;
            }
            if d != 0 && state != 0 && (d > 0) == (state > 0) {
                last_same = i;
            }
            if d.abs() < i64::from(h) * count {
                continue;
            }
            let sign = if d > 0 { 1i8 } else { -1i8 };
            if state == 0 {
                state = sign;
                last_same = i;
                continue;
            }
            if sign == state {
                continue;
            }
            // `last_same` is the last sample on the old side; the next one is on the new.
            let k = last_same;
            let (d0, c0) = level(k);
            let (d1, c1) = level(k + 1);
            let (a, b) = (d0 as f32 / c0 as f32, d1 as f32 / c1 as f32);
            let t = if a != b {
                (a / (a - b)).clamp(0.0, 1.0)
            } else {
                0.5
            };
            let rising = sign > 0;
            let pos = k as f32 + t;
            if out.items.last().is_none_or(|e| pos > e.pos) {
                out.items.push(Edge {
                    pos,
                    strength: (b - a).abs() / 2.0,
                    rising,
                });
            }
            state = sign;
            last_same = i;
        }
    }

    /// Moves every edge to where the profile crosses the middle between the
    /// extremes on either side. Gradient peaks are pulled around by
    /// neighboring edges when bars and spaces are narrow or unevenly inked;
    /// the half-level crossing is not.
    ///
    /// The reference levels average two estimates. The local one takes the
    /// extremes of the samples up to `PLATEAU_REACH` away, limited by the
    /// neighboring edges, so it follows uneven illumination but a narrow run
    /// only reaches its own peak: blur lifts narrow bars and sinks narrow
    /// spaces, which puts the crossing of an edge between a narrow and a wide
    /// run at a different level than one between two wide runs. The window one
    /// takes the extremes over a window of `WINDOW_RUNS` mean run widths on
    /// each side, which is the contrast the wide runs of the symbol reach and
    /// removes that dependence on the width, but ignores a gradient steeper
    /// than the window and mistakes a glare veil over a bar for blur. The
    /// mean keeps both errors small, and the shift from the local level is
    /// limited to `WINDOW_SHIFT` of the local level difference; with either
    /// alone, or without the limit, groups of images that read before stop
    /// reading.
    fn refine_edges(profile: &[u8], items: &mut [Edge], window: bool) {
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
                (low(&profile[p..=c]), high(&profile[c..=n]))
            } else {
                (high(&profile[p..=c]), low(&profile[c..=n]))
            };
            let (before, after) = if window {
                let reach = window_reach(items, k);
                let (a, b) = (c.saturating_sub(reach), (c + reach).min(last));
                let (win_low, win_high) = (low(&profile[a..=b]), high(&profile[a..=b]));
                let (win_before, win_after) = if e.rising {
                    (win_low, win_high)
                } else {
                    (win_high, win_low)
                };
                let (before, after) = (f32::from(before), f32::from(after));
                let cap = WINDOW_SHIFT * (after - before).abs();
                (
                    blend(before, f32::from(win_before), cap),
                    blend(after, f32::from(win_after), cap),
                )
            } else {
                (i32::from(before), i32::from(after))
            };
            let twice_mid = before + after;
            let mid = twice_mid as f32 / 2.0;
            let lo = c.saturating_sub(CROSSING_REACH);
            let hi = (c + CROSSING_REACH).min(last - 1);
            let mut best: Option<(f32, f32)> = None;
            for (i, w) in profile[lo..=hi + 1].windows(2).enumerate() {
                let (q0, q1) = (i32::from(w[0]), i32::from(w[1]));
                if q0 != q1
                    && (2 * q0 - twice_mid) * (2 * q1 - twice_mid) <= 0
                    && (q1 > q0) == e.rising
                {
                    let (p0, p1) = (f32::from(w[0]), f32::from(w[1]));
                    let x = (lo + i) as f32 + (mid - p0) / (p1 - p0);
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

    /// Edge errors (detected minus true position, px) over random symbols of
    /// runs 2.5 to 6 px wide whose edges blur with a 20-80 % rise of `rise` px.
    /// `shade` multiplies the profile by a function of the relative position.
    fn symbol_errors(
        params: &EdgeParams,
        rise: f32,
        shade: impl Fn(f32) -> f32,
        seed: u64,
    ) -> Vec<f32> {
        // The 20-80 % rise of a Gaussian edge is 1.683 sigma.
        let sigma = rise / 1.683;
        let mut rng = crate::testutil::Rng(seed);
        let mut errors = Vec::new();
        for _ in 0..60 {
            let widths: Vec<f32> = (0..30).map(|_| 2.5 + rng.unit() * 3.5).collect();
            let lead = 20.0 + rng.unit();
            let mut p = render(&widths, lead, 25.0, 225.0, sigma);
            let n = p.len() as f32;
            for (i, v) in p.iter_mut().enumerate() {
                *v = (f32::from(*v) * shade(i as f32 / n))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
            let mut out = Edges::default();
            EdgeDetector::default().detect(&p, params, &mut out);
            if out.len() != widths.len() {
                continue;
            }
            let mut x = lead;
            for (edge, w) in out.as_slice().iter().zip(&widths) {
                x += w;
                errors.push(edge.pos - x);
            }
        }
        errors
    }

    fn mean_abs(errors: &[f32]) -> f32 {
        errors.iter().map(|e| e.abs()).sum::<f32>() / errors.len().max(1) as f32
    }

    #[test]
    fn window_levels_improve_edges_of_heavily_blurred_symbols() {
        for rise in [1.5, 2.0, 2.5, 3.0] {
            let local = symbol_errors(&EdgeParams::fine().local(), rise, |_| 1.0, 7);
            let blended = symbol_errors(&EdgeParams::fine(), rise, |_| 1.0, 7);
            let (ml, mb) = (mean_abs(&local), mean_abs(&blended));
            assert!(local.len() > 300 && blended.len() >= local.len(), "{rise}");
            assert!(mb < ml, "rise {rise}: {mb} vs {ml}");
        }
    }

    #[test]
    fn window_levels_hold_under_an_illumination_gradient() {
        let shade = |t: f32| 0.45 + 0.55 * t;
        for rise in [1.5, 2.5] {
            let local = symbol_errors(&EdgeParams::fine().local(), rise, shade, 11);
            let blended = symbol_errors(&EdgeParams::fine(), rise, shade, 11);
            let (ml, mb) = (mean_abs(&local), mean_abs(&blended));
            assert!(local.len() > 300 && blended.len() >= local.len());
            // Mild blur: no worse than the local levels by more than the
            // limit allows; heavy blur: clearly better.
            let limit = if rise > 2.0 { 0.85 } else { 1.15 };
            assert!(mb < limit * ml, "rise {rise}: {mb} vs {ml}");
        }
    }

    /// Mean signed width error (px) of the thin runs in a symbol that has
    /// one wide bar and one wide space and thin bars (or thin spaces) between
    /// wide spaces (bars) otherwise. Blur attenuates the peak of a thin run,
    /// so with the neighboring runs as only reference its crossings sit at a
    /// level the wide runs never take and the run is measured too wide.
    fn thin_width_error(params: &EdgeParams, thin_bars: bool) -> f32 {
        let (thin, wide) = (3.0, 9.0);
        // Run 0 is a space, odd runs are bars.
        let mut widths = vec![wide, wide, wide];
        for _ in 0..12 {
            widths.extend(if thin_bars {
                [thin, wide]
            } else {
                [wide, thin]
            });
        }
        let first_thin = if thin_bars { 3 } else { 4 };
        let p = render(&widths, 30.0, 25.0, 225.0, 2.0 / 1.683);
        let mut out = Edges::default();
        EdgeDetector::default().detect(&p, params, &mut out);
        // The last run is a space: its end is not an edge.
        assert_eq!(out.len(), widths.len() - 1, "{params:?}");
        let e = out.as_slice();
        let errors: Vec<f32> = (first_thin..widths.len() - 1)
            .step_by(2)
            .map(|k| e[k].pos - e[k - 1].pos - thin)
            .collect();
        errors.iter().sum::<f32>() / errors.len() as f32
    }

    #[test]
    fn window_levels_reduce_the_width_bias_of_thin_runs() {
        for thin_bars in [true, false] {
            let local = thin_width_error(&EdgeParams::fine().local(), thin_bars);
            let blended = thin_width_error(&EdgeParams::fine(), thin_bars);
            assert!(local > 0.0, "{thin_bars}: {local}");
            assert!(
                blended.abs() < 0.9 * local.abs(),
                "thin bars {thin_bars}: {blended} vs {local}"
            );
        }
    }

    fn crossing(profile: &[u8]) -> Edges {
        let mut out = Edges::default();
        EdgeDetector::default().detect(profile, &EdgeParams::crossing(), &mut out);
        out
    }

    /// A bar and a gap of one sample each between wide runs: the case that
    /// motivated the crossing detector (Code 39 at 1.3 px per module).
    fn one_sample_features() -> Vec<u8> {
        let mut p = vec![180u8; 24];
        p.extend([6, 6, 6, 180, 180, 180]);
        p.extend([55, 177, 46]);
        p.extend([180, 180, 6, 6, 6, 180, 180, 180]);
        p.extend(std::iter::repeat_n(180u8, 16));
        p
    }

    #[test]
    fn crossing_sees_one_sample_runs_that_the_gradient_variants_merge() {
        let p = one_sample_features();
        let e = crossing(&p);
        let pos: Vec<f32> = e.as_slice().iter().map(|e| e.pos).collect();
        // wide bar (2 edges), bar of 55 and bar of 46 (4 edges), wide bar (2 edges)
        assert_eq!(pos.len(), 8, "{pos:?}");
        assert!(e.as_slice().windows(2).all(|w| w[0].rising != w[1].rising));
        assert!(!e.as_slice()[0].rising);
        // The gap between the one-sample bars is about one sample wide.
        let gap = pos[4] - pos[3];
        assert!((0.8..=1.6).contains(&gap), "{gap} {pos:?}");
        let mut merged = Edges::default();
        EdgeDetector::default().detect(&p, &EdgeParams::fine(), &mut merged);
        assert!(
            merged.len() < 8,
            "the gradient variant resolves it: {}",
            merged.len()
        );
    }

    #[test]
    fn crossing_positions_follow_blurred_edges() {
        for sigma in [0.0, 0.4, 0.6] {
            for frac in [0.0f32, 0.3, 0.6] {
                let widths = [10.0 + frac, 1.6, 3.4, 1.5, 1.7, 3.3, 1.6, 6.0, 20.0];
                let p = render(&widths, 25.0, 20.0, 220.0, sigma);
                let e = crossing(&p);
                assert_eq!(e.len(), widths.len() - 1, "sigma {sigma} frac {frac}");
                let mut x = 25.0 + widths[0];
                for (k, edge) in e.as_slice().iter().enumerate() {
                    assert!(
                        (edge.pos - x).abs() < 0.5,
                        "sigma {sigma} edge {k}: {} vs {x}",
                        edge.pos
                    );
                    x += widths[k + 1];
                }
            }
        }
    }

    #[test]
    fn crossing_finds_nothing_in_flat_or_noisy_but_low_contrast_profiles() {
        let mut rng = crate::testutil::Rng(3);
        let flat = vec![120u8; 200];
        assert!(crossing(&flat).is_empty());
        // Noise of a few levels around a mean, range below the minimum.
        let noisy: Vec<u8> = (0..200)
            .map(|_| (120.0 + rng.unit() * 14.0) as u8)
            .collect();
        assert!(crossing(&noisy).is_empty());
        // The same noise next to one real bar: the hysteresis keeps the
        // chatter out, only the bar's two edges remain.
        let mut p: Vec<u8> = (0..200).map(|_| (200.0 + rng.unit() * 6.0) as u8).collect();
        p[90..96].fill(20);
        let e = crossing(&p);
        assert_eq!(e.len(), 2, "{:?}", e.as_slice());
    }
}
