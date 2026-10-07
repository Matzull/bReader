/// Dense bit matrix, one bit per pixel, `true` = dark.
/// Rows are padded to whole `u64` words so row operations stay word-aligned.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BitMatrix {
    width: usize,
    height: usize,
    words_per_row: usize,
    bits: Vec<u64>,
}

impl BitMatrix {
    pub fn new(width: usize, height: usize) -> Self {
        let mut m = Self::default();
        m.resize(width, height);
        m
    }

    /// Resizes keeping the allocation when possible and clears all bits.
    pub fn resize(&mut self, width: usize, height: usize) {
        self.width = width;
        self.height = height;
        self.words_per_row = width.div_ceil(64);
        self.bits.clear();
        self.bits.resize(self.words_per_row * height, 0);
    }

    pub const fn width(&self) -> usize {
        self.width
    }

    pub const fn height(&self) -> usize {
        self.height
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> bool {
        debug_assert!(x < self.width && y < self.height);
        let w = self.bits[y * self.words_per_row + x / 64];
        (w >> (x % 64)) & 1 != 0
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, value: bool) {
        debug_assert!(x < self.width && y < self.height);
        let w = &mut self.bits[y * self.words_per_row + x / 64];
        let mask = 1u64 << (x % 64);
        if value {
            *w |= mask;
        } else {
            *w &= !mask;
        }
    }

    pub fn row_words(&self, y: usize) -> &[u64] {
        let start = y * self.words_per_row;
        &self.bits[start..start + self.words_per_row]
    }

    pub fn row_words_mut(&mut self, y: usize) -> &mut [u64] {
        let start = y * self.words_per_row;
        &mut self.bits[start..start + self.words_per_row]
    }

    /// All words, row after row, with the number of words per row.
    pub fn words_mut(&mut self) -> (&mut [u64], usize) {
        (&mut self.bits, self.words_per_row)
    }

    /// Flips every pixel; padding bits beyond `width` stay zero.
    pub fn invert(&mut self) {
        let tail = self.width % 64;
        let tail_mask = if tail == 0 {
            u64::MAX
        } else {
            (1u64 << tail) - 1
        };
        for row in self.bits.chunks_exact_mut(self.words_per_row.max(1)) {
            for w in row.iter_mut() {
                *w = !*w;
            }
            if let Some(last) = row.last_mut() {
                *last &= tail_mask;
            }
        }
    }

    pub fn copy_from(&mut self, other: &Self) {
        self.width = other.width;
        self.height = other.height;
        self.words_per_row = other.words_per_row;
        self.bits.clear();
        self.bits.extend_from_slice(&other.bits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_invert() {
        let mut m = BitMatrix::new(70, 2);
        m.set(69, 1, true);
        m.set(0, 0, true);
        assert!(m.get(69, 1));
        assert!(!m.get(68, 1));
        m.invert();
        assert!(!m.get(69, 1));
        assert!(m.get(68, 1));
        assert_eq!(m.row_words(0)[1] >> 6, 0);
    }
}
