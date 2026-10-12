//! Conversion of measured element widths into whole modules.

/// Most elements a DataBar row has (Expanded with 22 characters).
pub(super) const MAX_ELEMENTS: usize = 256;

/// A run of consecutive elements with a known total module count. A segment
/// of one element is scaled by the symbol-wide module instead.
#[derive(Debug, Clone, Copy)]
pub(super) struct Segment {
    pub at: usize,
    pub len: usize,
    pub modules: u8,
}

impl Segment {
    pub const fn new(at: usize, len: usize, modules: u8) -> Self {
        Self { at, len, modules }
    }
}

/// Result of [`quantize`].
#[derive(Debug, Clone, Copy)]
pub(super) struct Quantized {
    /// Largest distance, in modules, between a scaled element and the whole
    /// number chosen for it.
    pub deviation: f32,
}

/// Rounds the elements of every segment to whole modules whose sum is the
/// segment's module count, after removing the print gain (bars wider than
/// spaces) estimated from the rounding residuals. `bar0` tells whether
/// element 0 is a bar. `None` when a segment cannot be scaled or an element
/// lies further than `tol` modules from a whole number.
pub(super) fn quantize(
    widths: &[f32],
    bar0: bool,
    segments: &[Segment],
    module: f32,
    tol: f32,
    out: &mut [u8],
) -> Option<Quantized> {
    let n = widths.len();
    if n > MAX_ELEMENTS || out.len() != n || !(module.is_finite() && module > 0.0) {
        return None;
    }
    let mut adjusted = [0f32; MAX_ELEMENTS];
    adjusted[..n].copy_from_slice(widths);
    for pass in 0..2 {
        let mut bar = (0.0f32, 0u32);
        let mut space = (0.0f32, 0u32);
        for seg in segments {
            let scale = scale_of(&adjusted, seg, module)?;
            for (i, &a) in adjusted.iter().enumerate().skip(seg.at).take(seg.len) {
                let e = a * scale;
                let residual = e - e.round().max(1.0);
                let slot = if bar0 == (i % 2 == 0) {
                    &mut bar
                } else {
                    &mut space
                };
                slot.0 += residual;
                slot.1 += 1;
            }
        }
        if pass == 1 {
            break;
        }
        if bar.1 == 0 || space.1 == 0 {
            return None;
        }
        let mean = |s: (f32, u32)| s.0 / s.1 as f32;
        let gain = ((mean(bar) - mean(space)) / 2.0 * module).clamp(-0.3 * module, 0.3 * module);
        for (i, a) in adjusted[..n].iter_mut().enumerate() {
            *a = widths[i] + if bar0 == (i % 2 == 0) { -gain } else { gain };
        }
    }
    let mut deviation = 0.0f32;
    for seg in segments {
        let scale = scale_of(&adjusted, seg, module)?;
        let mut e = [0f32; MAX_ELEMENTS];
        for (k, i) in (seg.at..seg.at + seg.len).enumerate() {
            e[k] = adjusted[i] * scale;
        }
        let d = apportion(
            &e[..seg.len],
            seg.modules,
            &mut out[seg.at..seg.at + seg.len],
        )?;
        deviation = deviation.max(d);
        if d > tol {
            return None;
        }
    }
    Some(Quantized { deviation })
}

fn scale_of(widths: &[f32], seg: &Segment, module: f32) -> Option<f32> {
    if seg.len == 1 {
        return Some(1.0 / module);
    }
    let sum: f32 = widths.get(seg.at..seg.at + seg.len)?.iter().sum();
    (sum.is_finite() && sum > 0.0).then(|| f32::from(seg.modules) / sum)
}

/// Whole numbers of at least 1 close to `e` that add up to `modules`; the
/// largest distance to `e` is returned.
fn apportion(e: &[f32], modules: u8, out: &mut [u8]) -> Option<f32> {
    let target = i32::from(modules);
    let mut used = 0i32;
    for (o, &x) in out.iter_mut().zip(e) {
        let v = (x.floor() as i32).clamp(1, 255);
        *o = v as u8;
        used += v;
    }
    if e.len() > 1 {
        while used < target {
            let i = pick(e, out, |frac, best| frac > best, |_| true)?;
            out[i] += 1;
            used += 1;
        }
        while used > target {
            let i = pick(e, out, |frac, best| frac < best, |n| n > 1)?;
            out[i] -= 1;
            used -= 1;
        }
    } else {
        out[0] = modules;
    }
    Some(
        e.iter()
            .zip(out.iter())
            .map(|(&x, &o)| (x - f32::from(o)).abs())
            .fold(0.0, f32::max),
    )
}

/// Index of the element whose fractional surplus `e - n` is extreme.
fn pick(
    e: &[f32],
    out: &[u8],
    better: impl Fn(f32, f32) -> bool,
    allowed: impl Fn(u8) -> bool,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, (&x, &o)) in e.iter().zip(out).enumerate() {
        if !allowed(o) {
            continue;
        }
        let frac = x - f32::from(o);
        if best.is_none_or(|(_, b)| better(frac, b)) {
            best = Some((i, frac));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_whole_modules_and_keeps_the_sum() {
        let mut out = [0u8; 4];
        let w = [10.2f32, 19.7, 9.8, 40.1];
        let q = quantize(&w, true, &[Segment::new(0, 4, 8)], 10.0, 0.4, &mut out).unwrap();
        assert_eq!(out, [1, 2, 1, 4]);
        assert!(q.deviation < 0.3);
    }

    #[test]
    fn print_gain_is_removed() {
        // Bars 0.4 module too wide, spaces 0.4 too narrow.
        let mut out = [0u8; 8];
        let w = [1.4f32, 0.6, 2.4, 1.6, 1.4, 0.6, 2.4, 0.6];
        let r = quantize(&w, true, &[Segment::new(0, 8, 11)], 1.0, 0.2, &mut out);
        assert!(r.is_some());
        assert_eq!(out, [1, 1, 2, 2, 1, 1, 2, 1]);
    }

    #[test]
    fn rejects_elements_between_two_whole_numbers() {
        let mut out = [0u8; 4];
        let w = [10.0f32, 15.0, 10.0, 15.0];
        assert!(quantize(&w, true, &[Segment::new(0, 4, 5)], 10.0, 0.3, &mut out).is_none());
    }

    #[test]
    fn hostile_input_is_refused() {
        let mut out = [0u8; 2];
        assert!(
            quantize(
                &[f32::NAN, 1.0],
                true,
                &[Segment::new(0, 2, 2)],
                1.0,
                0.4,
                &mut out
            )
            .is_none()
        );
        assert!(
            quantize(
                &[0.0, 0.0],
                true,
                &[Segment::new(0, 2, 2)],
                1.0,
                0.4,
                &mut out
            )
            .is_none()
        );
        assert!(
            quantize(
                &[1.0, 1.0],
                true,
                &[Segment::new(0, 2, 2)],
                f32::INFINITY,
                0.4,
                &mut out
            )
            .is_none()
        );
    }
}
