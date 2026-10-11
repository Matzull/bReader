//! Rank of an element-width combination, as used by the data characters of
//! all GS1 DataBar variants (ISO/IEC 24724): the widths of `K` elements that
//! sum to `n` modules, each between 1 and `max`, are numbered in
//! lexicographic order. Some character sets exclude the combination without
//! a width-1 element ("no narrow").

/// Ways to write `s` as a sum of `r` integers, each in `1..=max`.
fn compositions(r: u32, s: i32, max: i32) -> i64 {
    if r == 0 {
        return i64::from(s == 0);
    }
    if s < r as i32 || max < 1 {
        return 0;
    }
    let mut total = 0i64;
    let mut sign = 1i64;
    let mut j = 0u32;
    while j <= r {
        let rest = s - j as i32 * max - 1;
        if rest < r as i32 - 1 {
            break;
        }
        total += sign * binomial(rest as u32, r - 1) * binomial(r, j);
        sign = -sign;
        j += 1;
    }
    total
}

fn binomial(n: u32, k: u32) -> i64 {
    if k > n {
        return 0;
    }
    let k = k.min(n - k);
    let mut v = 1i64;
    for i in 0..k {
        v = v * i64::from(n - i) / i64::from(i + 1);
    }
    v
}

/// Valid completions of `r` elements summing to `s`; `need_one` demands a
/// width-1 element among them.
fn completions(r: u32, s: i32, max: i32, need_one: bool) -> i64 {
    let all = compositions(r, s, max);
    if need_one {
        all - compositions(r, s - r as i32, max - 1)
    } else {
        all
    }
}

#[cfg(test)]
/// Number of combinations of `k` widths with sum `n`, widths in `1..=max`.
pub(super) fn count(k: u32, n: i32, max: i32, no_narrow: bool) -> u32 {
    completions(k, n, max, no_narrow).max(0) as u32
}

/// Lexicographic rank of `widths` among the combinations with the same sum,
/// each width in `1..=max`; `None` when `widths` is not such a combination.
pub(super) fn rank<const K: usize>(widths: &[u8; K], max: u8, no_narrow: bool) -> Option<u32> {
    let max = i32::from(max);
    let mut remaining: i32 = widths.iter().map(|&w| i32::from(w)).sum();
    let mut seen_one = false;
    let mut value = 0i64;
    for (pos, &w) in widths.iter().enumerate() {
        let w = i32::from(w);
        if w < 1 || w > max {
            return None;
        }
        if pos + 1 < K {
            let after = (K - pos - 1) as u32;
            for smaller in 1..w {
                let need_one = no_narrow && !seen_one && smaller > 1;
                value += completions(after, remaining - smaller, max, need_one);
            }
        }
        seen_one |= w == 1;
        remaining -= w;
    }
    (!no_narrow || seen_one).then_some(value as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enumerate<const K: usize>(n: i32, max: u8, no_narrow: bool) -> Vec<[u8; K]> {
        fn go<const K: usize>(
            at: usize,
            left: i32,
            max: u8,
            cur: &mut [u8; K],
            out: &mut Vec<[u8; K]>,
        ) {
            if at == K {
                if left == 0 {
                    out.push(*cur);
                }
                return;
            }
            for w in 1..=max {
                if i32::from(w) <= left {
                    cur[at] = w;
                    go(at + 1, left - i32::from(w), max, cur, out);
                }
            }
        }
        let mut out = Vec::new();
        go(0, n, max, &mut [0; K], &mut out);
        if no_narrow {
            out.retain(|c| c.contains(&1));
        }
        out
    }

    fn check<const K: usize>(n: i32, max: u8, no_narrow: bool) {
        let all = enumerate::<K>(n, max, no_narrow);
        assert_eq!(count(K as u32, n, i32::from(max), no_narrow) as usize, all.len());
        for (i, c) in all.iter().enumerate() {
            assert_eq!(rank(c, max, no_narrow), Some(i as u32), "{c:?} {n} {max}");
        }
    }

    #[test]
    fn rank_is_the_lexicographic_index_for_four_elements() {
        for n in 4..=14 {
            for max in 1..=9 {
                for nn in [false, true] {
                    check::<4>(n, max, nn);
                }
            }
        }
    }

    #[test]
    fn rank_is_the_lexicographic_index_for_seven_elements() {
        for (n, max, nn) in [
            (17, 6, false),
            (9, 3, true),
            (13, 5, false),
            (13, 4, true),
            (11, 5, true),
            (19, 8, false),
            (7, 1, false),
            (19, 8, true),
        ] {
            check::<7>(n, max, nn);
        }
    }

    #[test]
    fn rejects_what_is_not_a_member() {
        assert_eq!(rank(&[2, 2, 2, 2], 5, true), None);
        assert_eq!(rank(&[1, 1, 1, 7], 6, false), None);
        assert_eq!(rank(&[0, 4, 4, 1], 6, false), None);
    }
}
