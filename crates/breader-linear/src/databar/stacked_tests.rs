//! Stage-level tests of the rows of stacked GS1 DataBar symbols: the rows
//! the scanline decoders read from rendered symbols, joined by
//! [`super::stacked::assemble`] with hand-made placements.

use breader_core::Symbology;

use super::DataBarDecoder;
use super::omni::{PART_BOTTOM, PART_TOP};
use super::stacked::{Assembly, Geom, assemble};
use crate::stack_testutil::databar_stacked_grid;
use crate::testutil::*;

type Part = (Vec<u8>, Geom);

/// Parts read from one grid row rendered on its own (both reading
/// directions, as the reader reads it), placed `x` pixels to the right at
/// height `y`.
fn row_parts(row: &[bool], px: f32, x: f32, y: f32) -> Vec<Part> {
    let opts = line_options(|_| {});
    decode_all_parts(&DataBarDecoder, &Render::px(px).profile(row), &opts)
        .into_iter()
        .filter(|h| h.part && !h.inverted)
        .map(|h| {
            (
                h.text,
                Geom {
                    from: (h.start + x, y),
                    to: (h.end + x, y),
                    module: h.module,
                },
            )
        })
        .collect()
}

/// The data rows of a stacked grid.
fn data_rows(kind: u8, grid: &[Vec<bool>]) -> Vec<&Vec<bool>> {
    grid.iter()
        .enumerate()
        .filter(|(y, _)| match kind {
            0 => y % 2 == 0,
            1 => *y == 0 || *y == 4,
            _ => y % 4 == 0,
        })
        .map(|(_, r)| r)
        .collect()
}

/// All parts of a symbol, its rows `pitch` pixels apart.
fn symbol_parts(kind: u8, text: &str, columns: usize, px: f32, pitch: f32) -> Vec<Part> {
    let grid = databar_stacked_grid(kind, text, columns);
    let mut parts = Vec::new();
    for (i, row) in data_rows(kind, &grid).into_iter().enumerate() {
        parts.extend(row_parts(row, px, 0.0, pitch * i as f32));
    }
    parts
}

fn join(parts: &[Part]) -> Vec<Assembly> {
    let view: Vec<(&[u8], Geom)> = parts.iter().map(|(t, g)| (t.as_slice(), *g)).collect();
    assemble(&view)
}

fn element(body: &str) -> Vec<u8> {
    format!("01{body}{}", check_digit(body)).into_bytes()
}

fn moved(p: &Part, dx: f32, dy: f32, scale: f32) -> Part {
    let g = p.1;
    (
        p.0.clone(),
        Geom {
            from: (g.from.0 + dx, g.from.1 + dy),
            to: (g.to.0 + dx, g.to.1 + dy),
            module: g.module * scale,
        },
    )
}

#[test]
fn stacked_and_stacked_omnidirectional_rows_join() {
    let mut rng = Rng(0x57ac_ed01);
    for kind in [0u8, 1] {
        for i in 0..60 {
            let body = if i == 0 {
                "0000000000000".to_string()
            } else {
                rng.digits(13)
            };
            let parts = symbol_parts(kind, &body, 1, 2.0 + rng.unit() * 3.0, 60.0);
            // One top row and one bottom row, and nothing else.
            assert_eq!(parts.len(), 2, "{kind} {body}: {parts:?}");
            let joined = join(&parts);
            assert_eq!(joined.len(), 1, "{kind} {body}");
            assert_eq!(joined[0].text, element(&body));
            assert_eq!(
                (joined[0].symbology, joined[0].rows, joined[0].segments),
                (Symbology::DataBar, 2, 0)
            );
        }
    }
}

#[test]
fn stacked_rows_must_line_up_and_lie_the_right_way() {
    let parts = symbol_parts(1, "2001234567890", 1, 3.0, 60.0);
    assert_eq!(join(&parts).len(), 1);
    let (top, bottom) = (parts[0].clone(), parts[1].clone());
    assert!(top.0[0] == PART_TOP && bottom.0[0] == PART_BOTTOM);
    // Above instead of below, on the same line, too far away, far apart
    // sideways, with another module.
    for (dx, dy, scale) in [
        (0.0, -120.0, 1.0),
        (0.0, -60.0, 1.0),
        (0.0, 1200.0, 1.0),
        (180.0, 0.0, 1.0),
        (0.0, 0.0, 1.5),
    ] {
        let rows = [top.clone(), moved(&bottom, dx, dy, scale)];
        assert!(join(&rows).is_empty(), "{dx} {dy} {scale}");
    }
    // Read the other way round.
    let mut flipped = bottom.clone();
    std::mem::swap(&mut flipped.1.from, &mut flipped.1.to);
    assert!(join(&[top.clone(), flipped]).is_empty());
    // Small offsets and a 10 % module difference are fine.
    assert_eq!(join(&[top.clone(), moved(&bottom, 1.5, 3.0, 1.1)]).len(), 1);
    // The bottom row of another symbol: the checksum refuses the pair.
    let other = symbol_parts(1, "4412345678900", 1, 3.0, 60.0);
    assert!(join(&[top.clone(), other[1].clone()]).is_empty());
    assert!(join(&[other[0].clone(), bottom]).is_empty());
}

#[test]
fn expanded_stacked_rows_join_in_every_layout() {
    let texts = [
        "0100012345678905",
        "01900123456789083103001750",
        "1012A",
        "010001234567890510ABC123",
        "011234567890123110UNIVERSITY-OF-DEUSTO",
        "10123456A1234A\x1d15991231",
        "1012345678901234567890123456789012345678901234567890",
        "01988987654321061599123131030017501012A\x1d422123\x1d2123456\x1d423012345678901",
    ];
    let mut multi = 0;
    for text in texts {
        for columns in 1..=6 {
            let parts = symbol_parts(2, text, columns, 2.5, 90.0);
            let joined = join(&parts);
            let rows = databar_stacked_grid(2, text, columns).len().div_ceil(4);
            if rows == 1 {
                // One row is a symbol of its own, read by the one-row reader.
                assert!(joined.is_empty(), "{text:?} {columns}");
                continue;
            }
            multi += 1;
            assert_eq!(joined.len(), 1, "{text:?} {columns}: {joined:?}");
            assert_eq!(joined[0].text, text.as_bytes(), "{text:?} {columns}");
            assert_eq!(usize::from(joined[0].rows), rows, "{text:?} {columns}");
            assert_eq!(joined[0].symbology, Symbology::DataBarExpanded);
            assert!(joined[0].segments >= 4);
        }
    }
    assert!(multi > 25, "{multi}");
}

#[test]
fn expanded_stacked_rows_need_every_row_in_place() {
    let text = "01900123456789083103001750";
    let columns = 1;
    let grid = databar_stacked_grid(2, text, columns);
    let rows = data_rows(2, &grid);
    assert!(rows.len() >= 3);
    let mut per_row: Vec<Vec<Part>> = rows
        .iter()
        .enumerate()
        .map(|(i, r)| row_parts(r, 2.5, 0.0, 90.0 * i as f32))
        .collect();
    let all = |per_row: &[Vec<Part>]| per_row.concat();
    assert_eq!(join(&all(&per_row)).len(), 1);
    // A row missing.
    for skip in 0..per_row.len() {
        let kept: Vec<Part> = per_row
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != skip)
            .flat_map(|(_, p)| p.clone())
            .collect();
        assert!(join(&kept).is_empty(), "row {skip} missing");
    }
    // Two rows swapped, a row shifted sideways or placed at the wrong
    // distance, a row twice as large, the stack read from the bottom.
    let swapped = {
        let mut p = per_row.clone();
        for part in &mut p[1] {
            part.1.from.1 += 90.0;
            part.1.to.1 += 90.0;
        }
        for part in &mut p[2] {
            part.1.from.1 -= 90.0;
            part.1.to.1 -= 90.0;
        }
        p
    };
    assert!(join(&all(&swapped)).is_empty());
    let shifted: Vec<Vec<Part>> = per_row
        .iter()
        .enumerate()
        .map(|(i, p)| {
            p.iter()
                .map(|x| {
                    if i == 1 {
                        moved(x, 40.0, 0.0, 1.0)
                    } else {
                        x.clone()
                    }
                })
                .collect()
        })
        .collect();
    assert!(join(&all(&shifted)).is_empty());
    let scaled: Vec<Vec<Part>> = per_row
        .iter()
        .enumerate()
        .map(|(i, p)| {
            p.iter()
                .map(|x| {
                    if i == 1 {
                        moved(x, 0.0, 0.0, 1.6)
                    } else {
                        x.clone()
                    }
                })
                .collect()
        })
        .collect();
    assert!(join(&all(&scaled)).is_empty());
    let upside_down: Vec<Vec<Part>> = per_row
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let y = 90.0 * (per_row.len() - 1 - i) as f32;
            p.iter()
                .map(|x| moved(x, 0.0, y - 90.0 * i as f32, 1.0))
                .collect()
        })
        .collect();
    assert!(join(&all(&upside_down)).is_empty());
    // Rows of another symbol of the same layout.
    let other = symbol_parts(2, "01900123456789083103001751", columns, 2.5, 90.0);
    let rows_b = {
        let mut p = per_row.clone();
        let mut b: Vec<Vec<Part>> = vec![Vec::new(); p.len()];
        for part in other {
            let i = (part.1.from.1 / 90.0).round() as usize;
            b[i].push(part);
        }
        // The last row from the other symbol: its last digit differs.
        let last = p.len() - 1;
        p[last] = b[last].clone();
        p
    };
    assert!(join(&all(&rows_b)).is_empty());
    per_row.clear();
}

#[test]
fn a_damaged_stacked_symbol_is_never_misread() {
    let mut rng = Rng(0xbadc_0ffe);
    let cases: [(u8, &str, usize); 4] = [
        (0, "2001234567890", 1),
        (1, "4412345678900", 1),
        (2, "01900123456789083103001750", 2),
        (2, "10123456A1234A\x1d15991231", 1),
    ];
    for (kind, text, columns) in cases {
        let grid = databar_stacked_grid(kind, text, columns);
        let good: Vec<u8> = if kind == 2 {
            text.as_bytes().to_vec()
        } else {
            element(text)
        };
        let data = data_rows(kind, &grid);
        for r in 0..data.len() {
            for at in 0..data[r].len() {
                let mut parts = Vec::new();
                for (i, row) in data.iter().enumerate() {
                    let mut row = (*row).clone();
                    if i == r {
                        row[at] = !row[at];
                    }
                    parts.extend(row_parts(
                        &row,
                        2.0 + 2.0 * rng.unit(),
                        0.0,
                        80.0 * i as f32,
                    ));
                }
                for a in join(&parts) {
                    assert_eq!(a.text, good, "{text:?} row {r} module {at}");
                }
            }
        }
    }
}

#[test]
fn noise_makes_no_rows_that_join() {
    let mut rng = Rng(0x5eed_f00d);
    let opts = line_options(|_| {});
    let mut parts = 0;
    for _ in 0..3000 {
        let n = 30 + rng.below(240) as usize;
        let widths: Vec<f32> = (0..n)
            .map(|_| (1 + rng.below(6)) as f32 * (0.8 + 0.4 * rng.unit()))
            .collect();
        let runs = crate::runs::Runs::from_widths(true, &widths, 20.0, 20.0);
        let mut out = Vec::new();
        crate::decoder::LinearDecoder::decode(&DataBarDecoder, &runs, &opts, &mut out);
        let rows: Vec<Part> = out
            .into_iter()
            .filter(|h| h.part)
            .enumerate()
            .map(|(i, h)| {
                (
                    h.text,
                    Geom {
                        from: (h.start, 50.0 * i as f32),
                        to: (h.end, 50.0 * i as f32),
                        module: h.module,
                    },
                )
            })
            .collect();
        parts += rows.len();
        assert!(join(&rows).is_empty());
    }
    // Rows are rare in noise: it is the checksum that counts, not luck.
    assert!(parts < 300, "{parts}");
}
