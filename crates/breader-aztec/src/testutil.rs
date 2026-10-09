//! Test helpers: rxing's encoder (a dev-dependency used to encode only), a
//! generator and fault injection. Streams the encoder cannot produce (FNC1,
//! Runes, hand-built token sequences) go through `build`.

mod scene;

pub(crate) use crate::build::*;
use crate::spec::{self, Layout};
use breader_core::BitMatrix;
use rxing::aztec::encoder::aztec_encoder;
use rxing::common::CharacterSet;
pub(crate) use scene::*;

/// xorshift64 generator.
pub(crate) struct Rng(pub u64);

impl Rng {
    pub(crate) fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// A symbol produced by an encoder.
#[derive(Debug, Clone)]
pub(crate) struct Rendered {
    pub grid: BitMatrix,
    pub layout: Layout,
    pub data_codewords: usize,
}

/// Encodes with rxing. `layers` is 0 for automatic, positive for a
/// full-range and negative for a compact symbol with that many layers.
pub(crate) fn rxing_encode(
    data: &[u8],
    ec_percent: u32,
    layers: i32,
    charset: Option<CharacterSet>,
) -> Option<Rendered> {
    let code = match charset {
        Some(cs) => aztec_encoder::encode_bytes_with_charset(data, ec_percent, layers, cs),
        None => aztec_encoder::encode_bytes(data, ec_percent, layers),
    }
    .ok()?;
    let size = code.getSize() as usize;
    let mut grid = BitMatrix::new(size, size);
    let m = code.getMatrix();
    for y in 0..size {
        for x in 0..size {
            if m.get(x as u32, y as u32) {
                grid.set(x, y, true);
            }
        }
    }
    Some(Rendered {
        grid,
        layout: Layout {
            compact: code.isCompact(),
            layers: code.getLayers() as usize,
        },
        data_codewords: code.getCodeWords() as usize,
    })
}

/// Flips the module that carries bit `index` of the spiral.
pub(crate) fn flip_bit(grid: &mut BitMatrix, layout: Layout, index: usize) {
    let (x, y) = spec::data_positions(layout)[index];
    let v = grid.get(x, y);
    grid.set(x, y, !v);
}
