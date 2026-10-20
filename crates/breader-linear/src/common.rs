//! Helpers shared by the width-based decoders (Code 128, Code 39, Code 93, ITF).

use crate::runs::Runs;

/// Elements of a character split into wide and narrow ones.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Split {
    /// Bit `i` set: element `i` is wide.
    pub wide_mask: u16,
    pub narrow: f32,
    pub wide: f32,
    /// Smallest wide element over the largest narrow one; the closer to 1,
    /// the more ambiguous the classification.
    pub separation: f32,
}

/// Classifies the `wide` largest of `w` as wide. `None` when a width is not
/// a positive finite number.
pub(crate) fn split_wide<const N: usize>(w: &[f32; N], wide: usize) -> Option<Split> {
    if wide == 0 || wide >= N || !w.iter().all(|x| x.is_finite() && *x > 0.0) {
        return None;
    }
    let mut order = [0u8; N];
    for (i, o) in order.iter_mut().enumerate() {
        *o = i as u8;
    }
    order.sort_unstable_by(|&a, &b| w[usize::from(b)].total_cmp(&w[usize::from(a)]));
    let (wides, narrows) = order.split_at(wide);
    let mask = wides.iter().fold(0u16, |m, &i| m | 1 << i);
    let mean = |ids: &[u8]| ids.iter().map(|&i| w[usize::from(i)]).sum::<f32>() / ids.len() as f32;
    let min_wide = w[usize::from(wides[wide - 1])];
    let max_narrow = w[usize::from(narrows[0])];
    Some(Split {
        wide_mask: mask,
        narrow: mean(narrows),
        wide: mean(wides),
        separation: min_wide / max_narrow,
    })
}

/// Widths of runs `at..at + N` with the print-gain `bias` removed: bars
/// shrink and spaces grow by it. `None` when the runs do not exist or a
/// width is not positive and finite afterwards.
pub(crate) fn adjusted_widths<const N: usize>(
    runs: &Runs,
    at: usize,
    bias: f32,
) -> Option<[f32; N]> {
    let src = runs.widths().get(at..at.checked_add(N)?)?;
    let mut out = [0.0; N];
    for (k, (o, &w)) in out.iter_mut().zip(src).enumerate() {
        *o = if runs.is_bar(at + k) {
            w - bias
        } else {
            w + bias
        };
        if !(o.is_finite() && *o > 0.0) {
            return None;
        }
    }
    Some(out)
}

/// Width of one run with the bias removed, `None` when it is not usable.
pub(crate) fn adjusted_width(runs: &Runs, i: usize, bias: f32) -> Option<f32> {
    adjusted_widths::<1>(runs, i, bias).map(|w| w[0])
}

/// Quiet zone before run `i` (a bar), bias removed. Light area at the start
/// of a profile counts as measured: it is a lower bound.
pub(crate) fn quiet_before(runs: &Runs, i: usize, bias: f32) -> f32 {
    let q = runs.space_before(i);
    if q > 0.0 { q + bias } else { 0.0 }
}

/// [`quiet_before`] for symbologies whose quiet zone may end at the profile
/// border: a margin up to the border is taken as open, since the border, not
/// the content, limits it. Only for symbologies that cannot read as a
/// shorter valid symbol when the border cuts the symbol.
pub(crate) fn quiet_before_open(runs: &Runs, i: usize, bias: f32) -> f32 {
    if runs.open_before(i) {
        f32::INFINITY
    } else {
        quiet_before(runs, i, bias)
    }
}

/// [`quiet_after`] with the border rule of [`quiet_before_open`].
pub(crate) fn quiet_after_open(runs: &Runs, i: usize, bias: f32) -> f32 {
    if runs.open_after(i) {
        f32::INFINITY
    } else {
        quiet_after(runs, i, bias)
    }
}

/// Quiet zone after run `i` (a bar), bias removed.
pub(crate) fn quiet_after(runs: &Runs, i: usize, bias: f32) -> f32 {
    let q = runs.space_after(i);
    if q > 0.0 { q + bias } else { 0.0 }
}

/// Full ASCII of Code 39 and Code 93: the character that `lead` (`$`, `%`,
/// `/` or `+`, the shift character) and the capital `letter` after it stand
/// for. `None` for a pair the specifications do not assign, including the
/// aliases of DEL (`%X` to `%Z`) and the two pairs `/M` and `/N`, whose
/// characters have their own code.
pub(crate) fn full_ascii_pair(lead: u8, letter: u8) -> Option<u8> {
    if !letter.is_ascii_uppercase() {
        return None;
    }
    let k = letter - b'A';
    match lead {
        b'$' => Some(k + 1),
        b'+' => Some(b'a' + k),
        b'/' => match letter {
            b'A'..=b'L' | b'O' => Some(33 + k),
            b'Z' => Some(b':'),
            _ => None,
        },
        b'%' => match letter {
            b'A'..=b'E' => Some(27 + k),
            b'F'..=b'J' => Some(59 + (k - 5)),
            b'K'..=b'O' => Some(91 + (k - 10)),
            b'P'..=b'S' => Some(123 + (k - 15)),
            b'T' => Some(127),
            b'U' => Some(0),
            b'V' => Some(b'@'),
            b'W' => Some(b'`'),
            _ => None,
        },
        _ => None,
    }
}

/// Smallest module width, in profile samples, the decoders consider.
pub(crate) const MIN_MODULE: f32 = 0.75;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_by_size_and_reports_separation() {
        let s = split_wide(&[1.0, 2.5, 1.1, 2.4, 0.9], 2).unwrap();
        assert_eq!(s.wide_mask, 0b01010);
        assert!((s.narrow - 1.0).abs() < 0.01 && (s.wide - 2.45).abs() < 0.01);
        assert!((s.separation - 2.4 / 1.1).abs() < 1e-4);
        let flat = split_wide(&[1.0; 5], 2).unwrap();
        assert!((flat.separation - 1.0).abs() < 1e-6);
    }

    #[test]
    fn hostile_widths_are_rejected() {
        assert!(split_wide(&[1.0, f32::NAN, 1.0], 1).is_none());
        assert!(split_wide(&[1.0, 0.0, 1.0], 1).is_none());
        assert!(split_wide(&[1.0, f32::INFINITY, 1.0], 1).is_none());
        assert!(split_wide(&[1.0, 2.0, 1.0], 3).is_none());
        let runs = Runs::from_widths(true, &[1.0, 2.0, 3.0], 5.0, 5.0);
        assert!(adjusted_widths::<4>(&runs, 0, 0.0).is_none());
        assert!(adjusted_widths::<2>(&runs, usize::MAX, 0.0).is_none());
        assert!(adjusted_widths::<3>(&runs, 0, 1.5).is_none());
        assert_eq!(adjusted_widths::<2>(&runs, 1, 0.5), Some([2.5, 2.5]));
    }
}
