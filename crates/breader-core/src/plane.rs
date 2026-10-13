//! Binary image seen in one polarity: `get` returns true for symbol-dark
//! modules whether the symbol is dark-on-light or light-on-dark.

use crate::BitMatrix;

/// A binary image seen in one polarity.
#[derive(Debug, Clone, Copy)]
pub struct Plane<'a> {
    raw: &'a BitMatrix,
    inverted: bool,
}

impl<'a> Plane<'a> {
    /// Views `raw` as is, or with its polarity flipped when `inverted`.
    pub fn new(raw: &'a BitMatrix, inverted: bool) -> Self {
        Self { raw, inverted }
    }

    /// The underlying matrix.
    pub fn raw(&self) -> &'a BitMatrix {
        self.raw
    }

    /// Whether the polarity is flipped.
    pub fn inverted(&self) -> bool {
        self.inverted
    }

    /// Width in pixels.
    pub fn width(&self) -> usize {
        self.raw.width()
    }

    /// Height in pixels.
    pub fn height(&self) -> usize {
        self.raw.height()
    }

    /// Whether the pixel is symbol-dark in this view.
    #[inline]
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.raw.get(x, y) != self.inverted
    }

    /// Length of the run of pixels whose darkness equals `dark`, walking from
    /// `(x, y)` in steps of `d`, and whether the walk stopped because it left
    /// the image or reached `limit` pixels (otherwise a pixel of the other
    /// color ended it). A start outside the image gives `(0, true)`.
    pub fn run_len(&self, x: i32, y: i32, d: (i32, i32), dark: bool, limit: u32) -> (u32, bool) {
        let (w, h) = (self.width() as i64, self.height() as i64);
        // Steps that stay inside the image along one axis.
        let room = |p: i32, dp: i32, size: i64| -> i64 {
            let p = i64::from(p);
            if p < 0 || p >= size {
                0
            } else if dp > 0 {
                size - p
            } else if dp < 0 {
                p + 1
            } else {
                i64::MAX
            }
        };
        let max = room(x, d.0, w).min(room(y, d.1, h)).min(i64::from(limit)) as u32;
        let (mut cx, mut cy) = (x, y);
        for n in 0..max {
            if self.get(cx as usize, cy as usize) != dark {
                return (n, false);
            }
            cx += d.0;
            cy += d.1;
        }
        (max, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The step-by-step walk `run_len` replaces.
    fn walk(
        m: Plane<'_>,
        mut x: i32,
        mut y: i32,
        d: (i32, i32),
        dark: bool,
        limit: u32,
    ) -> (u32, bool) {
        let mut n = 0;
        loop {
            let inside = x >= 0 && y >= 0 && (x as usize) < m.width() && (y as usize) < m.height();
            if !inside || n >= limit {
                return (n, true);
            }
            if m.get(x as usize, y as usize) != dark {
                return (n, false);
            }
            n += 1;
            x += d.0;
            y += d.1;
        }
    }

    #[test]
    fn run_len_matches_the_step_by_step_walk() {
        let (w, h) = (37usize, 29usize);
        let mut state = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for density in [2u64, 5, 40] {
            let mut bits = BitMatrix::new(w, h);
            for y in 0..h {
                for x in 0..w {
                    bits.set(x, y, next() % density == 0);
                }
            }
            for inverted in [false, true] {
                let plane = Plane::new(&bits, inverted);
                for _ in 0..3000 {
                    let (x, y) = ((next() % 49) as i32 - 6, (next() % 41) as i32 - 6);
                    let d = [
                        (1, 0),
                        (-1, 0),
                        (0, 1),
                        (0, -1),
                        (1, 1),
                        (1, -1),
                        (-1, 1),
                        (-1, -1),
                    ][(next() % 8) as usize];
                    let dark = next() % 2 == 0;
                    let limit = (next() % 70) as u32;
                    assert_eq!(
                        plane.run_len(x, y, d, dark, limit),
                        walk(plane, x, y, d, dark, limit),
                        "({x}, {y}) {d:?} dark={dark} limit={limit}"
                    );
                }
            }
        }
    }
}
