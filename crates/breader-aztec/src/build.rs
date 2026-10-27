//! Symbol writer used by the tests and the fuzz entry points: stuffing, check
//! words, finder, mode message and spiral placement, built on the same
//! `spec` tables as the decoder (which the tests check against an
//! independent encoder).

use crate::spec::{self, Layout};
use breader_core::BitMatrix;
use breader_ecc::{AZTEC_DATA_6, AZTEC_DATA_8, AZTEC_DATA_10, AZTEC_DATA_12, AZTEC_PARAM};

/// A bit string under construction.
#[derive(Debug, Default, Clone)]
pub(crate) struct Bits(pub Vec<bool>);

impl Bits {
    pub(crate) fn push(&mut self, value: u32, n: usize) -> &mut Self {
        for i in (0..n).rev() {
            self.0.push(value >> i & 1 == 1);
        }
        self
    }
}

/// Bit stuffing and padding into codewords of `w` bits: a codeword whose
/// first `w - 1` bits are equal gets the complement as its last bit; the
/// last codeword is padded with ones.
pub(crate) fn stuff(bits: &[bool], w: usize) -> Vec<u16> {
    let get = |i: usize| bits.get(i).copied().unwrap_or(true);
    let mut out = Vec::new();
    let mut i = 0;
    while i < bits.len() {
        let first = get(i);
        let equal = (1..w - 1).all(|k| get(i + k) == first);
        let mut word = 0u16;
        for k in 0..w - 1 {
            word = word << 1 | u16::from(get(i + k));
        }
        if equal {
            word = word << 1 | u16::from(!first);
            i += w - 1;
        } else {
            word = word << 1 | u16::from(get(i + w - 1));
            i += w;
        }
        out.push(word);
    }
    out
}

/// The smallest layout whose data codewords can hold `words` words of size
/// `w(layout)` plus `min_ec` check words. `bits` is stuffed per layout.
pub(crate) fn fit(bits: &[bool], min_ec: usize) -> Option<(Layout, Vec<u16>)> {
    // The mode message counts the data codewords less one: a symbol holds at
    // least one.
    if bits.is_empty() {
        return None;
    }
    let layouts = (1..=4)
        .map(|layers| Layout {
            compact: true,
            layers,
        })
        .chain((1..=32).map(|layers| Layout {
            compact: false,
            layers,
        }));
    for layout in layouts {
        let words = stuff(bits, layout.word_size());
        let ok_size = words.len() + min_ec <= layout.total_codewords();
        let ok_compact = !layout.compact || words.len() <= 64;
        if ok_size && ok_compact {
            return Some((layout, words));
        }
    }
    None
}

fn rs_encode(w: usize, data: &[u16], ec: usize) -> Vec<u16> {
    let mut check = vec![0u16; ec];
    match w {
        4 => AZTEC_PARAM.encode(data, ec, &mut check),
        6 => AZTEC_DATA_6.encode(data, ec, &mut check),
        8 => AZTEC_DATA_8.encode(data, ec, &mut check),
        10 => AZTEC_DATA_10.encode(data, ec, &mut check),
        _ => AZTEC_DATA_12.encode(data, ec, &mut check),
    }
    .unwrap();
    check
}

/// Mode message bits of a symbol: layers and data codewords, or a Rune value
/// when `rune` is set (with the rune mask applied).
pub(crate) fn mode_message(layout: Layout, data_words: usize, rune: Option<u8>) -> Vec<bool> {
    let mut bits = Bits::default();
    let (words, check) = if let Some(v) = rune {
        bits.push(u32::from(v), 8);
        (2, spec::COMPACT_MODE_CHECK)
    } else if layout.compact {
        bits.push(layout.layers as u32 - 1, 2)
            .push(data_words as u32 - 1, 6);
        (2, spec::COMPACT_MODE_CHECK)
    } else {
        bits.push(layout.layers as u32 - 1, 5)
            .push(data_words as u32 - 1, 11);
        (4, spec::FULL_MODE_CHECK)
    };
    let w4: Vec<u16> = bits
        .0
        .chunks(4)
        .map(|c| c.iter().fold(0u16, |a, &b| a << 1 | u16::from(b)))
        .collect();
    debug_assert_eq!(w4.len(), words);
    let mut all = w4.clone();
    all.extend(rs_encode(4, &w4, check));
    let mut out: Vec<bool> = all
        .iter()
        .flat_map(|&w| (0..4).rev().map(move |i| w >> i & 1 == 1))
        .collect();
    if rune.is_some() {
        for (i, b) in out.iter_mut().enumerate() {
            *b ^= spec::RUNE_MASK >> (27 - i) & 1 == 1;
        }
    }
    out
}

fn draw_finder(grid: &mut BitMatrix, compact: bool) {
    let c = (grid.width() / 2) as i32;
    let core: i32 = if compact { 5 } else { 7 };
    for dy in -core + 1..core {
        for dx in -core + 1..core {
            let d = dx.abs().max(dy.abs()) as usize;
            if spec::bullseye_dark(d) {
                grid.set((c + dx) as usize, (c + dy) as usize, true);
            }
        }
    }
    for (dx, dy) in [
        (-core, -core),
        (-core + 1, -core),
        (-core, -core + 1),
        (core, -core),
        (core, -core + 1),
        (core, core - 1),
    ] {
        grid.set((c + dx) as usize, (c + dy) as usize, true);
    }
}

fn draw_mode(grid: &mut BitMatrix, compact: bool, bits: &[bool]) {
    let c = (grid.width() / 2) as i32;
    for (&(dx, dy), &b) in spec::mode_positions(compact).iter().zip(bits) {
        grid.set((c + dx) as usize, (c + dy) as usize, b);
    }
}

fn draw_reference_grid(grid: &mut BitMatrix) {
    let size = grid.width();
    let c = size / 2;
    for y in 0..size {
        for x in 0..size {
            if spec::on_grid_line(size, x, y) {
                let other = if x.abs_diff(c).is_multiple_of(16) {
                    y.abs_diff(c)
                } else {
                    x.abs_diff(c)
                };
                // Lines through the center are part of the bullseye already.
                if other.is_multiple_of(2) && x.abs_diff(c).max(y.abs_diff(c)) > 7 {
                    grid.set(x, y, true);
                }
            }
        }
    }
}

/// Renders a symbol from `data_words` (already stuffed codewords of the
/// layout's size) with correct mode message, check words and finder.
pub(crate) fn render(layout: Layout, data_words: &[u16]) -> BitMatrix {
    render_flagged(layout, data_words, false)
}

/// Like [`render`]; `reader_init` sets the top bit of the codeword count
/// field of the mode message (a reader initialisation symbol, one layer).
pub(crate) fn render_flagged(layout: Layout, data_words: &[u16], reader_init: bool) -> BitMatrix {
    let flag = match (reader_init, layout.compact) {
        (false, _) => 0,
        (true, true) => 1 << 5,
        (true, false) => 1 << 10,
    };
    let w = layout.word_size();
    let total = layout.total_codewords();
    let mut words = data_words.to_vec();
    words.extend(rs_encode(w, data_words, total - data_words.len()));
    let mut bits = vec![false; layout.start_pad()];
    for &word in &words {
        bits.extend((0..w).rev().map(|i| word >> i & 1 == 1));
    }
    let size = layout.size();
    let mut grid = BitMatrix::new(size, size);
    for (&(x, y), &b) in spec::data_positions(layout).iter().zip(&bits) {
        grid.set(x, y, b);
    }
    if !layout.compact {
        draw_reference_grid(&mut grid);
    }
    draw_finder(&mut grid, layout.compact);
    draw_mode(
        &mut grid,
        layout.compact,
        &mode_message(layout, data_words.len() + flag, None),
    );
    grid
}

/// An Aztec Rune symbol.
pub(crate) fn render_rune(value: u8) -> BitMatrix {
    let layout = Layout {
        compact: true,
        layers: 0,
    };
    let mut grid = BitMatrix::new(spec::RUNE_SIZE, spec::RUNE_SIZE);
    draw_finder(&mut grid, true);
    draw_mode(&mut grid, true, &mode_message(layout, 0, Some(value)));
    grid
}

/// Renders a high-level bit stream into the smallest symbol with at least
/// `min_ec` check words.
#[cfg(test)]
pub(crate) fn render_stream(bits: &Bits, min_ec: usize) -> Option<(Layout, BitMatrix)> {
    let (layout, words) = fit(&bits.0, min_ec)?;
    Some((layout, render(layout, &words)))
}
