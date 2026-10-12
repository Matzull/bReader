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
}
