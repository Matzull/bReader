//! Alignment pattern positions (ISO/IEC 18004 Annex E), generated from the
//! placement rule: the first center is 6, the last is `dimension - 7`, and
//! the centers in between are evenly spaced backwards from the last one with
//! an even step (the rounded average spacing, raised to the next even number).

const MAX_CENTERS: usize = 7;

const fn centers_for(version: usize) -> ([u8; MAX_CENTERS], usize) {
    let mut out = [0u8; MAX_CENTERS];
    if version < 2 {
        return (out, 0);
    }
    let last = 17 + 4 * version as i32 - 7;
    let n = version as i32 / 7 + 2;
    let den = n - 1;
    let num = last - 6;
    let mut step = (2 * num + den) / (2 * den);
    if step % 2 == 1 {
        step += 1;
    }
    out[0] = 6;
    let mut k = 1;
    while k < n {
        out[k as usize] = (last - (n - 1 - k) * step) as u8;
        k += 1;
    }
    (out, n as usize)
}

const fn build() -> ([[u8; MAX_CENTERS]; 41], [u8; 41]) {
    let mut table = [[0u8; MAX_CENTERS]; 41];
    let mut lens = [0u8; 41];
    let mut v = 2;
    while v <= 40 {
        let (c, n) = centers_for(v);
        table[v] = c;
        lens[v] = n as u8;
        v += 1;
    }
    (table, lens)
}

static TABLE: ([[u8; MAX_CENTERS]; 41], [u8; 41]) = build();

/// Row/column coordinates of alignment pattern centers (empty for version 1).
pub(crate) fn alignment_centers(version: u8) -> &'static [u8] {
    let v = usize::from(version);
    if v > 40 {
        return &[];
    }
    &TABLE.0[v][..usize::from(TABLE.1[v])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_versions() {
        assert!(alignment_centers(1).is_empty());
        assert!(alignment_centers(0).is_empty());
        assert!(alignment_centers(41).is_empty());
        assert_eq!(alignment_centers(2), [6, 18]);
        assert_eq!(alignment_centers(7), [6, 22, 38]);
        assert_eq!(alignment_centers(14), [6, 26, 46, 66]);
        assert_eq!(alignment_centers(32), [6, 34, 60, 86, 112, 138]);
        assert_eq!(alignment_centers(36), [6, 24, 50, 76, 102, 128, 154]);
        assert_eq!(alignment_centers(40), [6, 30, 58, 86, 114, 142, 170]);
    }

    #[test]
    fn structure() {
        for v in 2..=40u8 {
            let c = alignment_centers(v);
            let dim = 17 + 4 * usize::from(v);
            assert_eq!(c.len(), usize::from(v) / 7 + 2, "v{v}");
            assert_eq!(c[0], 6);
            assert_eq!(usize::from(c[c.len() - 1]), dim - 7, "v{v}");
            assert!(c.windows(2).all(|w| w[1] > w[0]), "v{v}");
        }
    }
}
