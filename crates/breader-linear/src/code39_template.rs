//! Template fallback for thin Code 39 (D-058).
//!
//! At 1.1 to 1.4 samples per module the edges of a scanline are placed with a
//! noise of about a quarter of a module and some elements merge, so the widths
//! the width-based decoder classifies are wrong. This fallback does not look
//! at edges after the start of a symbol: it fits the start character (module,
//! wide-to-narrow ratio, blur, print gain, position) to the grey profile, then
//! reads every character by correlating its window with the ideal profile of
//! each of the 44 characters, blurred by the same amount, and refits the whole
//! symbol to what it read. A character counts only if the best template beats
//! the second best by a margin. The symbol is accepted only under the rules of
//! the width-based thin read: quiet zones of 7 narrow widths on both sides,
//! start and stop, the same options for the check character and Full ASCII,
//! and the aggregator's 4 independent agreeing lines.

use std::sync::OnceLock;

use crate::code39::{BY_MASK, STAR_MASK, THIN_MAX_MODULE, THIN_QUIET, make_hit};
use crate::decoder::{LineHit, LineOptions};
use crate::runs::Runs;

/// Smallest module, in samples, of a candidate.
const MIN_MODULE: f32 = 0.8;
/// Modules of the coarse fit of the start character that get the second stage.
const TOP: usize = 2;
/// Correlation of the start character below which a start is dropped after
/// the coarse stage, and after the second stage.
const COARSE_MIN: f32 = 0.7;
const SECOND_MIN: f32 = 0.6;
/// Blur (sigma) and print gain (in modules) pairs tried for the start character.
const BLURS: [(f32, f32); 5] = [
    (0.4, 0.0),
    (0.5, 0.25),
    (0.5, 0.45),
    (0.8, 0.25),
    (0.8, 0.5),
];
/// Modules tried for the start character.
const MODULES: [f32; 7] = [0.9, 1.05, 1.2, 1.4, 1.65, 1.95, 2.3];
/// A candidate start follows a light area of at least this many samples.
const MIN_QUIET: f32 = 5.0;
/// Runs that must follow a candidate start, all of the width of the elements of a thin symbol.
const FOLLOWING: usize = 12;
/// Longest run, in samples, of a thin symbol (a wide element of 3.6 modules).
const MAX_RUN: f32 = 9.0;
/// Runs up to this width count as narrow elements or gaps.
const MAX_NARROW: f32 = 3.0;
/// Narrow runs among the `FOLLOWING` after a candidate start.
const MIN_NARROW: usize = 6;
/// Samples after a candidate start in which the profile must swing like a barcode.
const SURVEY: usize = 60;
const MIN_CROSSINGS: usize = 22;
/// Lines with fewer runs hold no symbol of 4 characters (a start, 4 characters and a stop have
/// about 50 elements and the edge detectors miss up to a third of the thin ones).
const MIN_LINE_RUNS: usize = 32;
/// Candidate starts without evidence from the width-based decoder fitted per scan, before one is read.
const FIT_BUDGET: usize = 80;
/// Candidate starts fitted per line.
const MAX_FITS: usize = 8;
/// Characters of a symbol, start and stop included.
const MAX_CHARS: usize = 40;
/// Smallest contrast, in grey levels, between bars and spaces.
const MIN_CONTRAST: f32 = 30.0;
/// Correlation of the start character with its template.
const MIN_START: f32 = 0.8;
/// Correlation of the whole symbol with its template.
const MIN_SYMBOL: f32 = 0.55;
/// Correlation of each character with the template it was read as.
const MIN_CHAR: f32 = 0.45;
/// The read with the ratio of the start fit must get to this character for other ratios to be tried.
const SECOND_TRY_DEPTH: usize = 3;
/// A character whose best template correlates less than this ends the read.
const ABORT_CORR: f32 = 0.42;
/// How much the best template of a character must beat the best other one.
const MIN_MARGIN: f32 = 0.02;
/// The quiet zones must stay at most this far from the light level, as a
/// fraction of the bar contrast.
const QUIET_LEVEL: f32 = 0.45;
/// The centre of a gap between characters stays at most this far from the light level, as a
/// fraction of the bar contrast.
const GAP_LEVEL: f32 = 0.4;
/// Pitch of the characters, in modules, of a symbol with a ratio of 2.5.
const NOMINAL_PITCH: f32 = 14.5;
/// Shifts tried for each character, in modules: first the coarse ones for
/// every template, then the fine ones for the templates that did well.
const COARSE_SHIFTS: [f32; 4] = [-0.6, -0.2, 0.2, 0.6];
const FINE_SHIFTS: [f32; 3] = [-0.4, 0.0, 0.4];
/// Templates within this much of the best coarse correlation get the fine shifts.
const REFINE: f32 = 0.2;
/// Reach of the blur in sigmas.
const REACH: f32 = 3.5;
const LUT_STEPS: f32 = 32.0;
const LUT_LEN: usize = (2.0 * REACH * LUT_STEPS) as usize + 1;

/// The 44 characters as (character, mask of wide elements).
pub(crate) const TEMPLATES: [(u8, u16); 44] = {
    let mut t = [(0u8, 0u16); 44];
    let mut k = 0;
    let mut mask = 0;
    while mask < 512 {
        if BY_MASK[mask] != 0 {
            t[k] = (BY_MASK[mask], mask as u16);
            k += 1;
        }
        mask += 1;
    }
    t
};

/// Where a symbol may start: the position of its first bar in the profile (the
/// mirrored copy when `mirror` is set) and the width of that bar.
#[derive(Debug, Clone, Copy)]
struct Start {
    at: f32,
    /// The width-based decoder read a start character here.
    noted: bool,
    /// Width of the first bar.
    width: f32,
    mirror: bool,
}

/// Buffers of the fallback, kept between lines.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    rev: Vec<u8>,
    edges: Vec<f32>,
    /// Candidate starts of the line, in the order they were noted.
    starts: Vec<Start>,
    /// Fits of candidates without evidence from the width-based decoder that
    /// this scan may still make; unlimited once a symbol was read.
    budget: usize,
}

impl Scratch {
    /// Starts a scan: the budget of fits is full again.
    pub(crate) fn new_scan(&mut self) {
        self.budget = FIT_BUDGET;
    }

    /// Forgets the candidates of the previous line.
    pub(crate) fn clear(&mut self) {
        self.starts.clear();
    }

    /// Adds the runs that follow a light area of at least `MIN_QUIET` samples and
    /// begin a stretch of `FOLLOWING` runs of the widths of a thin symbol's
    /// elements (see `stretch`), read forward or, with `mirror`, against the
    /// profile of `len` samples.
    pub(crate) fn note_stretches(&mut self, runs: &Runs, mirror: bool, len: usize) {
        let widths = runs.widths();
        let n = widths.len();
        if n < MIN_LINE_RUNS {
            return;
        }
        let mut j = 0;
        while j + FOLLOWING <= n && self.starts.len() < MAX_FITS {
            let before = if j == 0 { runs.lead() } else { widths[j - 1] };
            if before >= MIN_QUIET && stretch(&widths[j..j + FOLLOWING]) {
                let at = if mirror {
                    (len - 1) as f32 - runs.start(j)
                } else {
                    runs.start(j)
                };
                if !self
                    .starts
                    .iter()
                    .any(|s| s.mirror == mirror && (s.at - at).abs() < 3.0)
                {
                    self.starts.push(Start {
                        at,
                        noted: false,
                        width: widths[j],
                        mirror,
                    });
                }
            }
            j += 1;
        }
    }

    /// Adds the thin Code 39 starts the width-based decoder noted in `runs`,
    /// read forward or, with `mirror`, against the profile of `len` samples.
    pub(crate) fn note(&mut self, runs: &Runs, mirror: bool, len: usize) {
        for &j in runs.thin_starts().iter() {
            let at = if mirror {
                (len - 1) as f32 - runs.start(j)
            } else {
                runs.start(j)
            };
            if !self
                .starts
                .iter()
                .any(|s| s.mirror == mirror && (s.at - at).abs() < 3.0)
                && self.starts.len() < MAX_FITS
            {
                self.starts.push(Start {
                    at,
                    noted: true,
                    width: runs.widths()[j],
                    mirror,
                });
            }
        }
    }
}

/// Cumulative normal distribution on a table.
fn phi(z: f32) -> f32 {
    static TABLE: OnceLock<[f32; LUT_LEN]> = OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0.0; LUT_LEN];
        for (i, v) in t.iter_mut().enumerate() {
            let z = f64::from(i as u32) / f64::from(LUT_STEPS) - f64::from(REACH);
            let x = z / std::f64::consts::SQRT_2;
            let k = 1.0 / (1.0 + 0.327_591_1 * x.abs());
            let poly = k
                * (0.254_829_592
                    + k * (-0.284_496_736
                        + k * (1.421_413_741 + k * (-1.453_152_027 + k * 1.061_405_429))));
            let erf = (1.0 - poly * (-x * x).exp()).copysign(x);
            *v = (0.5 * (1.0 + erf)) as f32;
        }
        t
    });
    let i = ((z + REACH) * LUT_STEPS + 0.5) as usize;
    table[i.min(LUT_LEN - 1)]
}

/// Position, module, ratio of wide to narrow, blur (sigma in samples, pixel
/// integration included) and print gain (samples taken from each bar and
/// given to the neighbouring spaces) of a symbol.
#[derive(Debug, Clone, Copy)]
struct Fit {
    s: f32,
    m: f32,
    r: f32,
    sigma: f32,
    bias: f32,
}

impl Fit {
    fn valid(&self) -> bool {
        self.m >= MIN_MODULE
            && self.m <= THIN_MAX_MODULE * 1.3
            && (1.8..=3.4).contains(&self.r)
            && (0.3..=1.4).contains(&self.sigma)
            && self.bias >= -0.3
            && self.bias <= 0.6 * self.m
    }

    /// Distance between the starts of two characters, in samples.
    fn pitch(&self) -> f32 {
        (7.0 + 3.0 * self.r) * self.m
    }
}

/// The profile seen as bars of the given polarity.
struct Obs<'a> {
    p: &'a [u8],
    /// -1 when bars are dark, 1 when they are light.
    sign: f32,
}

/// Result of a correlation: the coefficient, and the line `p = level + step * t`
/// the profile follows on the template `t`.
#[derive(Debug, Clone, Copy)]
struct Corr {
    ncc: f32,
    level: f32,
    step: f32,
}

/// Bar edges of the character with wide elements `mask` whose first bar starts
/// at `s`; returns the position where its last bar ends.
fn char_edges(mask: u16, s: f32, fit: &Fit, out: &mut [f32]) -> f32 {
    let mut pos = s;
    for e in 0..9 {
        let w = if mask >> e & 1 == 1 { fit.r } else { 1.0 } * fit.m;
        if e % 2 == 0 {
            out[e] = pos + fit.bias / 2.0;
            out[e + 1] = pos + w - fit.bias / 2.0;
        }
        pos += w;
    }
    pos
}

/// Calls `f(x, t)` for the pixels `lo..=hi` with the ideal profile `t`, 1 in a
/// bar and 0 in a space, of the bars between the sorted `edges` (start, end,
/// start, ...) blurred by `sigma`, pixel `x` being at coordinate `x`.
fn for_each_template(
    edges: &[f32],
    sigma: f32,
    lo: usize,
    hi: usize,
    mut f: impl FnMut(usize, f32),
) {
    let reach = REACH * sigma;
    let inv = 1.0 / sigma;
    let mut first = edges.partition_point(|&e| e < lo as f32 - reach);
    let mut last = first;
    for x in lo..=hi {
        let xf = x as f32;
        while first < edges.len() && edges[first] < xf - reach {
            first += 1;
        }
        last = last.max(first);
        while last < edges.len() && edges[last] <= xf + reach {
            last += 1;
        }
        // The bars that ended long before are fully in the sum: an odd number
        // of edges behind leaves a bar open.
        let mut t = (first & 1) as f32;
        for (k, &e) in edges[first..last].iter().enumerate() {
            let v = phi((xf - e) * inv);
            if (first + k) & 1 == 0 {
                t += v;
            } else {
                t -= v;
            }
        }
        f(x, t);
    }
}

/// Correlation of the profile over the pixels `lo..=hi` with the blurred bars
/// between `edges` (see [`for_each_template`]). `None` when the window leaves
/// the profile.
fn correlate(obs: &Obs<'_>, edges: &[f32], sigma: f32, lo: f32, hi: f32) -> Option<Corr> {
    if !(lo >= 0.0 && hi >= lo && hi + 1.0 < obs.p.len() as f32) {
        return None;
    }
    let (lo, hi) = (lo.floor() as usize, hi.ceil() as usize);
    let (mut sp, mut st, mut spp, mut stt, mut spt) = (0.0f32, 0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for_each_template(edges, sigma, lo, hi, |x, t| {
        let p = f32::from(obs.p[x]);
        sp += p;
        st += t;
        spp += p * p;
        stt += t * t;
        spt += p * t;
    });
    let n = (hi - lo + 1) as f32;
    let cov = spt - sp * st / n;
    let vp = spp - sp * sp / n;
    let vt = stt - st * st / n;
    if vp < 1.0 || vt < 1e-3 {
        return None;
    }
    let step = cov / vt;
    Some(Corr {
        ncc: obs.sign * cov / (vp * vt).sqrt(),
        level: (sp - step * st) / n,
        step,
    })
}

fn start_corr(obs: &Obs<'_>, fit: &Fit) -> Option<Corr> {
    let mut edges = [0.0; 10];
    let end = char_edges(STAR_MASK, fit.s, fit, &mut edges);
    correlate(
        obs,
        &edges,
        fit.sigma,
        fit.s - 3.0 * fit.m,
        end + 0.8 * fit.m,
    )
}

fn symbol_corr(obs: &Obs<'_>, masks: &[u16], fit: &Fit, edges: &mut Vec<f32>) -> Option<Corr> {
    edges.clear();
    let mut pos = fit.s;
    let mut one = [0.0; 10];
    for &mask in masks {
        let end = char_edges(mask, pos, fit, &mut one);
        edges.extend_from_slice(&one);
        pos = end + fit.m;
    }
    let end = pos - fit.m;
    correlate(
        obs,
        edges,
        fit.sigma,
        fit.s - 4.0 * fit.m,
        end + 4.0 * fit.m,
    )
}

/// Coordinate descent on the five parameters of a fit; `score` is `None` for
/// fits that leave the profile.
fn descend(mut fit: Fit, mut score: impl FnMut(&Fit) -> Option<f32>) -> Option<(Fit, f32)> {
    let mut best = score(&fit)?;
    let mut scale = 1.0;
    for _ in 0..3 {
        for param in 0..5 {
            for dir in [-1.0f32, 1.0] {
                let mut f = fit;
                let step = scale
                    * match param {
                        0 => 0.3,
                        1 => 0.06 * fit.m,
                        2 => 0.15,
                        3 => 0.12,
                        _ => 0.15,
                    };
                *match param {
                    0 => &mut f.s,
                    1 => &mut f.m,
                    2 => &mut f.r,
                    3 => &mut f.sigma,
                    _ => &mut f.bias,
                } += dir * step;
                if !f.valid() {
                    continue;
                }
                if let Some(v) = score(&f)
                    && v > best
                {
                    best = v;
                    fit = f;
                }
            }
        }
        scale *= 0.5;
    }
    Some((fit, best))
}

/// The module that gives a character the length that module `m` gives it
/// with the ratio `r0`, for the ratio `r`: 6 narrow and 3 wide elements.
fn same_length(m: f32, r0: f32, r: f32) -> f32 {
    m * (6.0 + 3.0 * r0) / (6.0 + 3.0 * r)
}

/// Fits the start character near position `s0`: a coarse search over position
/// and module with a middling ratio, blur and print gain, then the ratio, the
/// blur and the gain around the best modules, then a descent.
fn fit_start(obs: &Obs<'_>, s0: f32) -> Option<(Fit, f32)> {
    let mut coarse = [(f32::NEG_INFINITY, 0.0f32); MODULES.len()];
    for ds in [-0.9, -0.6, -0.3, 0.0, 0.3, 0.6, 0.9] {
        for (slot, &m) in coarse.iter_mut().zip(&MODULES) {
            let fit = Fit {
                s: s0 + ds,
                m,
                r: 2.5,
                sigma: 0.5,
                bias: 0.35 * m,
            };
            if let Some(c) = start_corr(obs, &fit)
                && c.ncc > slot.0
            {
                *slot = (c.ncc, fit.s);
            }
        }
    }
    let mut order = [0, 1, 2, 3, 4, 5, 6];
    order.sort_unstable_by(|&a, &b| coarse[b].0.total_cmp(&coarse[a].0));
    if coarse[order[0]].0 < COARSE_MIN {
        return None;
    }
    let mut best: Option<(Fit, f32)> = None;
    for &k in &order[..TOP] {
        let (_, s) = coarse[k];
        for ds in [-0.3, 0.0, 0.3] {
            for r in [2.0, 2.5, 3.0] {
                // The start character is about as long with every ratio.
                let m = MODULES[k];
                for (sigma, gain) in BLURS {
                    let fit = Fit {
                        s: s + ds,
                        m,
                        r,
                        sigma,
                        bias: gain * m,
                    };
                    if let Some(c) = start_corr(obs, &fit)
                        && best.is_none_or(|(_, b)| c.ncc > b)
                    {
                        best = Some((fit, c.ncc));
                    }
                }
            }
        }
    }
    let (fit, score) = best?;
    if score < SECOND_MIN {
        return None;
    }
    descend(fit, |f| start_corr(obs, f).map(|c| c.ncc))
}

/// The characters of a symbol as masks, with how well each was told apart.
struct Chars {
    masks: [u16; MAX_CHARS],
    len: usize,
    min_corr: f32,
    min_margin: f32,
    /// Where the last bar ends.
    end: f32,
}

/// Reads characters one after the other from the start at `fit.s` until a
/// star: each is the best of the 44 templates over a few shifts, the next
/// begins one pitch further on. `depth` is raised to the index of the last
/// character looked at.
fn read_chars(obs: &Obs<'_>, fit: &Fit, depth: &mut usize) -> Option<Chars> {
    let mut chars = Chars {
        masks: [0; MAX_CHARS],
        len: 0,
        min_corr: 1.0,
        min_margin: 1.0,
        end: 0.0,
    };
    let pitch = fit.pitch();
    let mut cur = fit.s;
    let mut edges = [0.0; 10];
    for k in 0..MAX_CHARS {
        let mut best = [(-1.0f32, 0.0f32); TEMPLATES.len()];
        let mut score = |t: usize, d: f32, best: &mut [(f32, f32); TEMPLATES.len()]| {
            let s = cur + d * fit.m;
            let end = char_edges(TEMPLATES[t].1, s, fit, &mut edges);
            if let Some(c) = correlate(obs, &edges, fit.sigma, s - 0.8 * fit.m, end + 0.8 * fit.m)
                && c.ncc > best[t].0
            {
                best[t] = (c.ncc, d);
            }
        };
        if k == 0 {
            for t in 0..TEMPLATES.len() {
                score(t, 0.0, &mut best);
            }
        } else {
            // Every second shift for all templates, the others for those
            // within `REFINE` of the best: a shift off by 0.2 modules costs
            // less than that.
            for t in 0..TEMPLATES.len() {
                for d in COARSE_SHIFTS {
                    score(t, d, &mut best);
                }
            }
            let top = best.iter().map(|b| b.0).fold(-1.0f32, f32::max);
            for t in 0..TEMPLATES.len() {
                if best[t].0 >= top - REFINE {
                    for d in FINE_SHIFTS {
                        score(t, d, &mut best);
                    }
                }
            }
        }
        let top = (0..best.len()).max_by(|&a, &b| best[a].0.total_cmp(&best[b].0))?;
        let second = (0..best.len())
            .filter(|&t| t != top)
            .map(|t| best[t].0)
            .fold(-1.0f32, f32::max);
        let mask = TEMPLATES[top].1;
        *depth = (*depth).max(k);
        chars.min_corr = chars.min_corr.min(best[top].0);
        chars.min_margin = chars.min_margin.min(best[top].0 - second);
        if k == 0 && mask != STAR_MASK {
            return None;
        }
        if k >= 1 && best[top].0 < ABORT_CORR {
            return None;
        }
        let s = cur + best[top].1 * fit.m;
        chars.masks[chars.len] = mask;
        chars.len += 1;
        if mask == STAR_MASK && k > 0 {
            chars.end = char_edges(mask, s, fit, &mut edges);
            return Some(chars);
        }
        cur = s + pitch;
    }
    None
}

/// A symbol read with the templates, positions in the coordinates of the profile.
struct Found {
    chars: Chars,
    fit: Fit,
    corr: Corr,
}

/// Reads the symbol whose start bar begins near `s0`. The start character says
/// little about the ratio of wide to narrow, so the symbol is read with the
/// ratio of the start fit and with the two ratios around it; the best fit of
/// the three wins.
fn read_symbol(obs: &Obs<'_>, s0: f32, edges: &mut Vec<f32>) -> Option<Found> {
    let (fit, score) = fit_start(obs, s0)?;
    if score < MIN_START {
        return None;
    }
    let mut best: Option<Found> = None;
    for (k, (r, keep)) in [(fit.r, true), (3.0, true), (2.0, false), (3.0, false)]
        .into_iter()
        .enumerate()
    {
        if k > 0 && r == fit.r {
            continue;
        }
        let m = if keep {
            fit.m
        } else {
            same_length(fit.m, fit.r, r)
        };
        let mut depth = 0;
        let found = settle(obs, Fit { r, m, ..fit }, edges, &mut depth);
        if let Some(f) = found
            && best.as_ref().is_none_or(|b| f.corr.ncc > b.corr.ncc)
        {
            best = Some(f);
        }
        // A start that fails within its first characters with the ratio
        // of its fit is not a symbol of another ratio.
        if k == 0 && best.is_none() && depth < SECOND_TRY_DEPTH {
            break;
        }
    }
    best
}

/// Reads the characters with `fit`, refits the symbol to what was read and
/// reads again, until two reads agree.
fn settle(obs: &Obs<'_>, mut fit: Fit, edges: &mut Vec<f32>, depth: &mut usize) -> Option<Found> {
    let mut chars = read_chars(obs, &fit, depth)?;
    for _ in 0..3 {
        if chars.len < 3 {
            return None;
        }
        let masks = chars.masks;
        let n = chars.len;
        let (refit, _) = descend(fit, |f| {
            symbol_corr(obs, &masks[..n], f, edges).map(|c| c.ncc)
        })?;
        fit = refit;
        let again = read_chars(obs, &fit, depth)?;
        let same = again.masks[..again.len] == masks[..n];
        chars = again;
        if same {
            let corr = symbol_corr(obs, &chars.masks[..chars.len], &fit, edges)?;
            return Some(Found { chars, fit, corr });
        }
    }
    None
}

/// Whether the gap between every two characters is light where it is centred, as
/// far from the light level, in units of the bar contrast, as `GAP_LEVEL` allows.
fn gaps_light(obs: &Obs<'_>, f: &Found) -> bool {
    let mut pos = f.fit.s;
    let mut one = [0.0; 10];
    for &mask in &f.chars.masks[..f.chars.len - 1] {
        let end = char_edges(mask, pos, &f.fit, &mut one);
        let centre = end + 0.5 * f.fit.m;
        let lo = (centre - 0.5 * f.fit.m).ceil().max(0.0) as usize;
        let hi = ((centre + 0.5 * f.fit.m).floor() as usize).min(obs.p.len() - 1);
        let lightest = (lo..=hi.max(lo))
            .filter_map(|x| obs.p.get(x))
            .map(|&v| (f32::from(v) - f.corr.level) / f.corr.step)
            .fold(f32::INFINITY, f32::min);
        if lightest > GAP_LEVEL {
            return false;
        }
        pos = end + f.fit.m;
    }
    true
}

/// Whether the light area of `lo..hi` stays near the light level of `corr`.
fn quiet(obs: &Obs<'_>, corr: &Corr, lo: f32, hi: f32) -> bool {
    if lo < 0.0 || hi + 1.0 >= obs.p.len() as f32 || corr.step.abs() < MIN_CONTRAST {
        return false;
    }
    (lo.ceil() as usize..=hi.floor() as usize)
        .all(|x| (f32::from(obs.p[x]) - corr.level) / corr.step <= QUIET_LEVEL)
}

/// Whether `widths` are those of a run of narrow elements and gaps with a few
/// wide elements: none is longer than the widest element of a thin symbol and
/// most are narrow.
fn stretch(widths: &[f32]) -> bool {
    let mut narrow = 0;
    for &w in widths {
        if w > MAX_RUN {
            return false;
        }
        narrow += usize::from(w <= MAX_NARROW);
    }
    narrow >= MIN_NARROW
}

/// Reads the symbol that starts at `at`, if the profile allows it, and pushes the hit.
/// Returns whether one was found.
fn read_start(
    profile: &[u8],
    start: Start,
    opts: &LineOptions,
    edges: &mut Vec<f32>,
    out: &mut Vec<LineHit>,
) -> bool {
    let Start {
        at, width, mirror, ..
    } = start;
    let len = profile.len();
    if at < 5.0 || at + 4.0 >= len as f32 {
        return false;
    }
    let Some((sign, quiet_level, bar_level)) = polarity(profile, at, width) else {
        return false;
    };
    // The polarities the pass asked for, as for the width-based decoders.
    if !(if sign > 0.0 { opts.invert } else { opts.normal }) {
        return false;
    }
    if !swings(profile, at, quiet_level, bar_level) {
        return false;
    }
    let obs = Obs { p: profile, sign };
    let Some(f) = read_symbol(&obs, at, edges) else {
        return false;
    };
    let Some(hit) = accept(&obs, &f, opts, mirror, len) else {
        return false;
    };
    out.push(hit);
    true
}

/// -1 for dark bars on a light margin, 1 for light bars on a dark one, `None`
/// when the margin and the run do not differ clearly; with the two levels.
fn polarity(p: &[u8], at: f32, width: f32) -> Option<(f32, f32, f32)> {
    let bar = f32::from(p[((at + width / 2.0).round() as usize).min(p.len() - 1)]);
    let quiet = (2..=4)
        .map(|d| f32::from(p[(at - d as f32).round() as usize]))
        .sum::<f32>()
        / 3.0;
    let diff = quiet - bar;
    if diff > MIN_CONTRAST {
        Some((-1.0, quiet, bar))
    } else if diff < -MIN_CONTRAST {
        Some((1.0, quiet, bar))
    } else {
        None
    }
}

/// Whether the profile after a start at `at` swings like a thin barcode: over
/// the next `SURVEY` samples it crosses the level halfway between the margin and
/// the first bar, with hysteresis, at least `MIN_CROSSINGS` times.
fn swings(p: &[u8], at: f32, quiet_level: f32, bar_level: f32) -> bool {
    let lo = at.round() as usize;
    let Some(window) = p.get(lo..lo + SURVEY) else {
        return false;
    };
    let mid = (quiet_level + bar_level) / 2.0;
    let band = 0.2 * (quiet_level - bar_level).abs();
    let mut high = quiet_level > bar_level;
    let mut crossings = 0;
    for &v in window {
        let v = f32::from(v);
        if high && v < mid - band || !high && v > mid + band {
            high = !high;
            crossings += 1;
        }
    }
    crossings >= MIN_CROSSINGS
}

/// Applies the acceptance rules to a symbol read with the templates.
fn accept(
    obs: &Obs<'_>,
    f: &Found,
    opts: &LineOptions,
    mirror: bool,
    len: usize,
) -> Option<LineHit> {
    let chars = &f.chars;
    if f.corr.ncc < MIN_SYMBOL
        || chars.min_corr < MIN_CHAR
        || chars.min_margin < MIN_MARGIN
        || f.fit.m > THIN_MAX_MODULE
        || !gaps_light(obs, f)
    {
        return None;
    }
    let reach = THIN_QUIET * f.fit.m;
    if !quiet(obs, &f.corr, f.fit.s - reach, f.fit.s - 0.4 * f.fit.m)
        || !quiet(obs, &f.corr, chars.end + 0.4 * f.fit.m, chars.end + reach)
    {
        return None;
    }
    let mut data = [0u8; MAX_CHARS];
    for (d, &mask) in data.iter_mut().zip(&chars.masks[1..chars.len - 1]) {
        *d = BY_MASK[usize::from(mask) & 0x1ff];
    }
    let span = if mirror {
        let flip = (len - 1) as f32;
        (flip - f.fit.s, flip - chars.end)
    } else {
        (f.fit.s, chars.end)
    };
    let mut hit = make_hit(
        &data[..chars.len - 2],
        opts,
        span,
        // The narrow width trades off with the ratio and the print gain in the
        // fit; the pitch of the characters does not, and the aggregator
        // compares the modules of the lines.
        f.fit.pitch() / NOMINAL_PITCH,
        f.corr.ncc.clamp(0.0, 1.0),
        false,
        true,
    )?;
    hit.inverted = obs.sign > 0.0;
    Some(hit)
}

/// Thin Code 39 read by templates at the starts the width-based decoder noted
/// on this line (see [`Scratch::note`]).
pub(crate) fn read_line(
    profile: &[u8],
    opts: &LineOptions,
    scratch: &mut Scratch,
    out: &mut Vec<LineHit>,
) {
    if profile.len() < 64 || scratch.starts.is_empty() {
        return;
    }
    let mut mirrored = false;
    for k in 0..scratch.starts.len() {
        let start = scratch.starts[k];
        let mirror = start.mirror;
        if !start.noted {
            if scratch.budget == 0 {
                continue;
            }
            scratch.budget -= 1;
        }
        if mirror && !mirrored {
            scratch.rev.clear();
            scratch.rev.extend(profile.iter().rev());
            mirrored = true;
        }
        let p: &[u8] = if mirror { &scratch.rev } else { profile };
        if read_start(p, start, opts, &mut scratch.edges, out) {
            scratch.budget = usize::MAX;
            // The same symbol read from its other end, or one more candidate
            // inside it, adds nothing to the line.
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code39::Code39Decoder;
    use crate::decoder::LinearDecoder;
    use crate::edges::{EdgeDetector, EdgeParams, Edges};
    use crate::testutil::*;

    /// A line as `scan_profile` handles it: the width-based decoder on both
    /// directions and polarities, then the template fallback where it read
    /// nothing. Returns the width-based hits and the template hits.
    fn read(profile: &[u8], opts: &LineOptions) -> (Vec<LineHit>, Vec<LineHit>) {
        let mut edges = Edges::default();
        EdgeDetector::default().detect(profile, &EdgeParams::fine(), &mut edges);
        let mut runs = Runs::default();
        Runs::from_edges(&edges, &mut runs);
        let mut rev = Runs::default();
        Runs::reverse_from(&runs, &mut rev);
        let mut plain = Vec::new();
        for flipped in [false, true] {
            if flipped {
                runs.flip_roles();
                rev.flip_roles();
            }
            Code39Decoder.decode(&runs, opts, &mut plain);
            Code39Decoder.decode(&rev, opts, &mut plain);
        }
        plain.retain(|h| !h.text.is_empty());
        let mut scratch = Scratch::default();
        scratch.new_scan();
        scratch.clear();
        scratch.note(&runs, false, profile.len());
        scratch.note(&rev, true, profile.len());
        scratch.note_stretches(&runs, false, profile.len());
        scratch.note_stretches(&rev, true, profile.len());
        let mut out = Vec::new();
        read_line(profile, opts, &mut scratch, &mut out);
        (plain, out)
    }

    const TEXT: &[u8] = b"9033296";

    /// 1.35 samples per module, bars 0.6 sample thinner than spaces: the
    /// element widths of a line scatter by a quarter of a module and the
    /// width-based decoder reads none of the phases below.
    fn thin(offset_step: usize, text: &str) -> Vec<u8> {
        let r = Render {
            offset: offset_step as f32 * 0.13,
            gain: -0.3,
            sigma: 0.45,
            ..Render::px(1.35)
        };
        let mut p = r.profile(&code39_modules(text, 2, 1));
        add_noise(&mut p, 4.0, &mut Rng(offset_step as u64 + 1));
        p
    }

    #[test]
    fn reads_thin_bars_that_the_widths_miss() {
        let opts = line_options(|_| {});
        let mut reads = 0;
        for k in 0..6 {
            let p = thin(k, "9033296");
            let (plain, hits) = read(&p, &opts);
            assert!(plain.is_empty(), "{k}: {plain:?}");
            for h in &hits {
                assert_eq!(h.text, TEXT, "{k}");
                assert!(h.thin && !h.inverted && !h.short_quiet && !h.checksum_verified);
                assert_eq!(&h.aim, b"]A0");
                // The symbol starts and ends where it was drawn (12 modules of margin; 9
                // characters of 12 modules and 8 gaps; the thin bars start 0.3 samples late
                // and end 0.3 early).
                let shift = k as f32 * 0.13;
                assert!(
                    (h.start - (16.2 + shift + 0.3)).abs() < 1.0,
                    "{k} {}",
                    h.start
                );
                let end = 16.2 + shift + 116.0 * 1.35 - 0.3;
                assert!((h.end - end).abs() < 2.0, "{k} {}", h.end);
                // The module is the pitch of the characters over that of a ratio of 2.5.
                assert!(
                    (h.module - 13.0 * 1.35 / 14.5).abs() < 0.2,
                    "{k} {}",
                    h.module
                );
            }
            reads += usize::from(!hits.is_empty());
        }
        assert!(reads >= 5, "{reads}");
    }

    #[test]
    fn reads_both_directions_and_polarities() {
        let opts = line_options(|_| {});
        let p = thin(2, "9033296");
        let mut rev = p.clone();
        rev.reverse();
        let light: Vec<u8> = p.iter().map(|&v| 255 - v).collect();
        for (name, line) in [("reversed", &rev), ("light on dark", &light)] {
            let (plain, hits) = read(line, &opts);
            assert!(plain.is_empty(), "{name}");
            assert_eq!(hits.len(), 1, "{name}");
            assert_eq!(hits[0].text, TEXT, "{name}");
            assert_eq!(hits[0].inverted, name == "light on dark", "{name}");
        }
        // A polarity the pass does not ask for is not read.
        let mut dark_only = line_options(|_| {});
        (dark_only.normal, dark_only.invert) = (true, false);
        assert!(read(&light, &dark_only).1.is_empty());
        assert_eq!(read(&p, &dark_only).1.len(), 1);
        let mut light_only = dark_only;
        (light_only.normal, light_only.invert) = (false, true);
        assert!(read(&p, &light_only).1.is_empty());
        assert_eq!(read(&light, &light_only).1.len(), 1);
    }

    #[test]
    fn mirrored_hits_are_in_the_coordinates_of_the_profile() {
        let opts = line_options(|_| {});
        let mut rev = thin(2, "9033296");
        let (_, forward) = read(&rev, &opts);
        rev.reverse();
        let (_, back) = read(&rev, &opts);
        let n = rev.len() as f32 - 1.0;
        assert_eq!((forward.len(), back.len()), (1, 1));
        assert!((forward[0].start - (n - back[0].start)).abs() < 2.0);
        assert!((forward[0].end - (n - back[0].end)).abs() < 2.0);
    }

    #[test]
    fn quiet_zones_of_seven_modules_are_needed_on_both_sides() {
        let opts = line_options(|_| {});
        let modules = code39_modules("9033296", 2, 1);
        let at = |left: f32, right: f32| {
            let r = Render {
                gain: -0.3,
                sigma: 0.45,
                quiet_left: left,
                quiet_right: right,
                ..Render::px(1.35)
            };
            let mut p = r.profile(&modules);
            add_noise(&mut p, 4.0, &mut Rng(3));
            read(&p, &opts).1.len()
        };
        assert_eq!(at(12.0, 12.0), 1);
        assert_eq!(at(8.0, 8.0), 1);
        for (left, right) in [(5.0, 12.0), (12.0, 5.0), (3.0, 3.0), (0.0, 12.0)] {
            assert_eq!(at(left, right), 0, "{left} {right}");
        }
    }

    #[test]
    fn options_of_the_width_decoder_apply() {
        let sum: usize = "9033296"
            .bytes()
            .map(|c| {
                b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%"
                    .iter()
                    .position(|&x| x == c)
                    .unwrap()
            })
            .sum();
        let check = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%"[sum % 43] as char;
        let need = line_options(|o| o.code39.require_checksum = true);
        let ok = read(&thin(2, &format!("9033296{check}")), &need).1;
        assert_eq!((ok.len(), &ok[0].text[..]), (1, TEXT));
        assert!(ok[0].checksum_verified);
        assert_eq!(&ok[0].aim, b"]A3");
        // The same symbol without its check character is no symbol under that option, a short one is no symbol.
        assert!(read(&thin(2, "9033296"), &need).1.is_empty());
        let long = line_options(|o| o.code39.min_length = 8);
        assert!(read(&thin(2, "9033296"), &long).1.is_empty());
    }

    #[test]
    fn a_symbol_that_breaks_never_reads_as_itself_and_rarely_at_all() {
        // Six modules inverted: an inter-character gap or a character is destroyed. At 1.35
        // samples per module a narrow bar that moves by a module is a change of a few percent in
        // the correlation of a character, so where the inverted block leaves the nearest valid
        // character a template can still be read as that (D-058, risks); a destroyed gap is
        // not accepted, and the width-based decoder of the reader tests above reads none.
        let opts = line_options(|_| {});
        let modules = code39_modules("9033296", 2, 1);
        let mut rng = Rng(9);
        let mut reads = 0;
        for at in [25usize, 45, 65, 85, 105] {
            let mut broken = modules.clone();
            for m in &mut broken[at..at + 7] {
                *m = !*m;
            }
            let r = Render {
                gain: -0.3,
                sigma: 0.45,
                ..Render::px(1.35)
            };
            let mut p = r.profile(&broken);
            add_noise(&mut p, 4.0, &mut rng);
            for h in read(&p, &opts).1 {
                assert_ne!(h.text, TEXT, "{at}");
                reads += 1;
            }
        }
        assert!(reads <= 2, "{reads}");
    }

    #[test]
    fn a_destroyed_gap_is_not_accepted() {
        let opts = line_options(|_| {});
        let modules = code39_modules("9033296", 2, 1);
        // Gaps are the module after each of the nine characters of 12 modules.
        for gap in 0..8 {
            let mut broken = modules.clone();
            broken[gap * 13 + 12] = true;
            let r = Render {
                gain: -0.3,
                sigma: 0.45,
                ..Render::px(1.35)
            };
            let mut p = r.profile(&broken);
            add_noise(&mut p, 4.0, &mut Rng(gap as u64 + 40));
            assert!(read(&p, &opts).1.is_empty(), "{gap}");
        }
    }

    #[test]
    fn symbols_beyond_the_resolution_are_never_misread() {
        // 1 to 1.2 samples per module and a heavy blur: no clear winner among the templates of
        // some characters, and when a symbol is read at all its text is the one drawn.
        let opts = line_options(|_| {});
        let mut rng = Rng(21);
        let mut reads = 0;
        for i in 0..120 {
            let text = rng.digits(7);
            let r = Render {
                offset: rng.unit(),
                gain: -0.4 * rng.unit(),
                sigma: 0.6 + 0.5 * rng.unit(),
                ..Render::px(1.0 + 0.25 * rng.unit())
            };
            let mut p = r.profile(&code39_modules(&text, 2, 1));
            add_noise(&mut p, 6.0, &mut rng);
            for h in read(&p, &opts).1 {
                assert_eq!(h.text, text.as_bytes(), "{i}");
                reads += 1;
            }
        }
        assert!(reads <= 60, "{reads}");
    }

    #[test]
    fn other_profiles_leave_nothing() {
        let opts = line_options(|_| {});
        let mut rng = Rng(77);
        let wide = Render::px(1.4);
        let others = [
            wide.profile(&ean13_modules("4006381333931")),
            wide.profile(&itf_modules("0123456789", 2)),
            wide.profile(&code128_modules(&[104, 33, 34, 35, 36, 37, 38])),
            wide.profile(&runs_to_modules(&[1; 160])),
            wide.profile(&code39_modules("ABC123", 3, 1)),
        ];
        for (i, p) in others.iter().enumerate() {
            let hits = read(p, &opts).1;
            assert!(hits.iter().all(|h| h.text == b"ABC123"), "{i}: {hits:?}");
        }
        for _ in 0..200 {
            let widths: Vec<usize> = (0..120).map(|_| 1 + rng.below(3) as usize).collect();
            let mut p = Render::px(1.3).profile(&runs_to_modules(&widths));
            add_noise(&mut p, 5.0, &mut rng);
            assert!(read(&p, &opts).1.is_empty(), "{widths:?}");
        }
    }

    #[test]
    fn a_cut_symbol_is_not_read_shorter() {
        let opts = line_options(|_| {});
        let p = thin(2, "9033296ABC");
        let (_, whole) = read(&p, &opts);
        assert!(whole.iter().all(|h| h.text == b"9033296ABC"));
        for cut in [60usize, 90, 120, 150, 180, 200] {
            let hits = read(&p[..p.len() - cut], &opts).1;
            assert!(hits.is_empty(), "{cut}: {hits:?}");
            let hits = read(&p[cut..], &opts).1;
            assert!(hits.is_empty(), "{cut}: {hits:?}");
        }
    }

    #[test]
    fn the_response_of_the_blur_table_is_a_distribution() {
        assert!(phi(-REACH) < 0.001 && phi(REACH) > 0.999);
        assert!((phi(0.0) - 0.5).abs() < 0.01);
        let mut last = 0.0;
        for i in -35..=35 {
            let v = phi(i as f32 / 10.0);
            assert!(v >= last);
            last = v;
        }
    }

    #[test]
    fn candidate_starts_need_a_quiet_zone_and_narrow_runs() {
        let mut scratch = Scratch::default();
        // Thirty-six runs of 1.5 samples after 8 samples of margin.
        let mut widths = vec![8.0];
        widths.extend([1.5; 40]);
        let runs = Runs::from_widths(false, &widths, 0.0, 10.0);
        scratch.note_stretches(&runs, false, 200);
        assert_eq!(scratch.starts.len(), 1);
        assert!((scratch.starts[0].at - 8.0).abs() < 1e-3);
        // Wide runs, or a short line, give none.
        scratch.clear();
        let mut widths = vec![8.0];
        widths.extend([12.0; 40]);
        scratch.note_stretches(&Runs::from_widths(false, &widths, 0.0, 10.0), false, 600);
        scratch.note_stretches(&Runs::from_widths(false, &[8.0; 20], 0.0, 10.0), false, 600);
        assert!(scratch.starts.is_empty());
        // The mirrored copy counts from the other end.
        let mut widths = vec![8.0];
        widths.extend([1.5; 40]);
        scratch.note_stretches(&Runs::from_widths(false, &widths, 0.0, 10.0), true, 200);
        assert!((scratch.starts[0].at - 191.0).abs() < 1e-3);
    }

    #[test]
    fn candidates_without_evidence_are_limited_per_scan_until_a_symbol_is_read() {
        let opts = line_options(|_| {});
        let p = thin(2, "9033296");
        let candidate = Start {
            at: 16.5,
            noted: false,
            width: 1.0,
            mirror: false,
        };
        let mut scratch = Scratch::default();
        scratch.new_scan();
        scratch.starts.push(candidate);
        let mut out = Vec::new();
        scratch.budget = 0;
        read_line(&p, &opts, &mut scratch, &mut out);
        assert!(out.is_empty(), "no budget, no evidence");
        scratch.starts[0].noted = true;
        read_line(&p, &opts, &mut scratch, &mut out);
        assert_eq!(
            out.len(),
            1,
            "a start the width decoder noted is always tried"
        );
        assert_eq!(scratch.budget, usize::MAX, "a symbol read lifts the limit");
        scratch.new_scan();
        assert_eq!(scratch.budget, FIT_BUDGET);
    }
}
