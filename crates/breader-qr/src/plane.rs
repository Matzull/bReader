//! Binary image seen in one polarity: `get` returns true for symbol-dark
//! modules whether the symbol is dark-on-light or light-on-dark.

use breader_core::BitMatrix;

#[derive(Debug, Clone, Copy)]
pub(crate) struct Plane<'a> {
    raw: &'a BitMatrix,
    inverted: bool,
}

impl<'a> Plane<'a> {
    pub fn new(raw: &'a BitMatrix, inverted: bool) -> Self {
        Self { raw, inverted }
    }

    pub fn raw(&self) -> &'a BitMatrix {
        self.raw
    }

    pub fn inverted(&self) -> bool {
        self.inverted
    }

    pub fn width(&self) -> usize {
        self.raw.width()
    }

    pub fn height(&self) -> usize {
        self.raw.height()
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.raw.get(x, y) != self.inverted
    }
}
