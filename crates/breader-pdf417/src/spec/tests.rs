use super::*;
use crate::testutil::{EncodeOptions, Rng, render, try_render};
use rxing::pdf417::encoder::Compaction;

/// Alternating bar and space widths of a 17-module pattern.
fn widths(pattern: u32) -> Vec<u8> {
    assert_eq!(pattern >> 16, 1, "first module is a bar");
    let mut out = Vec::new();
    let mut run = 0u8;
    let mut cur = 1;
    for i in (0..17).rev() {
        let bit = pattern >> i & 1;
        if bit == cur {
            run += 1;
        } else {
            out.push(run);
            cur = bit;
            run = 1;
        }
    }
    out.push(run);
    out
}

#[test]
fn every_pattern_has_four_bars_and_four_spaces_summing_to_17() {
    for k in 0..3 {
        for value in 0..CODEWORD_COUNT as u16 {
            let w = widths(pattern(k, value));
            assert_eq!(w.len(), 8, "cluster {} value {value}", 3 * k);
            assert!(w.iter().all(|&x| (1..=6).contains(&x)), "{w:?}");
            assert_eq!(w.iter().map(|&x| u32::from(x)).sum::<u32>(), 17);
            // The last module is a space, so every pattern ends with a space.
            assert_eq!(pattern(k, value) & 1, 0);
        }
    }
}

#[test]
fn cluster_formula_matches_table() {
    for k in 0..3 {
        for value in 0..CODEWORD_COUNT as u16 {
            let w: [u8; 8] = widths(pattern(k, value)).try_into().unwrap();
            assert_eq!(
                cluster_of_widths(w),
                3 * k,
                "cluster {} value {value}",
                3 * k
            );
            assert_eq!(pattern_of_widths(w), Some(pattern(k, value)));
        }
    }
}

#[test]
fn all_patterns_are_distinct() {
    let mut all = std::collections::HashSet::new();
    for k in 0..3 {
        for value in 0..CODEWORD_COUNT as u16 {
            assert!(
                all.insert(pattern(k, value)),
                "cluster {} value {value} repeats a pattern",
                3 * k
            );
        }
    }
    assert_eq!(all.len(), 3 * 929);
}

#[test]
fn start_and_stop_are_not_codewords() {
    // 8-1-1-1-1-1-1-3 is a valid width sequence but not a codeword.
    assert_eq!(widths(0x1_FEA8), [8, 1, 1, 1, 1, 1, 1, 3]);
    assert_eq!(lookup(0x1_FEA8), None);
}

#[test]
fn lookup_finds_every_pattern_and_nothing_else() {
    for k in 0..3 {
        for value in 0..CODEWORD_COUNT as u16 {
            assert_eq!(lookup(pattern(k, value)), Some((k, value)));
        }
    }
    // Every other 17-module word that starts with a bar and could be a
    // codeword by its widths is rejected unless it is in a table.
    let mut found = 0;
    for bits in 0..1u32 << 16 {
        let p = 0x1_0000 | bits;
        let hit = lookup(p);
        found += usize::from(hit.is_some());
        if let Some((k, v)) = hit {
            assert_eq!(pattern(k, v), p);
        }
    }
    assert_eq!(found, 3 * 929);
}

#[test]
fn pattern_of_widths_rejects_bad_widths() {
    assert_eq!(pattern_of_widths([1, 1, 1, 1, 1, 1, 1, 1]), None);
    assert_eq!(pattern_of_widths([7, 1, 1, 1, 1, 1, 1, 4]), None);
    assert_eq!(pattern_of_widths([0, 2, 3, 3, 3, 3, 3, 0]), None);
    assert_eq!(pattern_of_widths([8, 1, 1, 1, 1, 1, 1, 3]), None);
    // Valid widths of the start pattern, which is not a codeword.
    assert_eq!(pattern_of_widths([8, 1, 1, 1, 1, 1, 1, 3]), None);
    assert_eq!(
        pattern_of_widths([3, 1, 1, 1, 1, 1, 3, 6]),
        Some(pattern(0, 0))
    );
}

#[test]
fn spec_examples() {
    assert_eq!(widths(pattern(0, 0)), [3, 1, 1, 1, 1, 1, 3, 6]);
    assert_eq!(ec_codewords(0), 2);
    assert_eq!(ec_codewords(8), 512);
}

#[test]
fn indicator_formulas_and_parsing_roundtrip() {
    for rows in MIN_ROWS..=MAX_ROWS {
        for (cols, level) in [(1, 0), (7, 3), (30, 8), (12, 5)] {
            let l = Layout::new(rows, cols, level).unwrap();
            let mut seen = (None, None, None);
            for row in 0..rows {
                for (left, value) in [
                    (true, l.left_indicator(row)),
                    (false, l.right_indicator(row)),
                ] {
                    assert!(value < 900);
                    match parse_indicator(row, left, value).unwrap() {
                        IndicatorField::RowsDiv3(x) => seen.0 = Some(x),
                        IndicatorField::LevelRowsMod3 { level, rows_mod3 } => {
                            seen.1 = Some((level, rows_mod3));
                        }
                        IndicatorField::ColsMinus1(x) => seen.2 = Some(x),
                    }
                }
            }
            if rows >= 3 {
                let (d, (lv, m), c) = (seen.0.unwrap(), seen.1.unwrap(), seen.2.unwrap());
                assert_eq!(combine_fields(d, lv, m, c), Some(l));
            }
        }
    }
}

#[test]
fn indicator_parsing_rejects_out_of_range() {
    // Wrong row group.
    assert_eq!(parse_indicator(0, true, 30), None);
    assert_eq!(parse_indicator(3, true, 5), None);
    // Level field above 26 in the level and row-count role (row 1 left).
    assert!(parse_indicator(1, true, 26).is_some());
    assert_eq!(parse_indicator(1, true, 27), None);
    assert_eq!(parse_indicator(1, true, 29), None);
    // Codeword values of the special range.
    assert_eq!(parse_indicator(0, true, 900), None);
    assert_eq!(parse_indicator(89, false, 928), None);
    assert_eq!(
        combine_fields(29, 0, 2, 0),
        Some(Layout::new(90, 1, 0).unwrap())
    );
    assert_eq!(combine_fields(29, 0, 2, 30), None);
    assert_eq!(combine_fields(0, 0, 0, 0), None);
    assert_eq!(combine_fields(0, 9, 2, 0), None);
    assert_eq!(Layout::new(91, 3, 2), None);
    assert_eq!(Layout::new(10, 31, 2), None);
}

/// Row indicators of symbols rendered by an independent encoder (read back
/// through the tables) equal the formulas, in every cluster.
#[test]
fn indicators_match_encoded_symbols() {
    let mut rng = Rng(99);
    for case in 0..60 {
        let len = 1 + rng.below(if case % 6 == 0 { 600 } else { 120 });
        let msg: String = (0..len)
            .map(|_| char::from(b'A' + rng.below(26) as u8))
            .collect();
        let opts = EncodeOptions {
            compaction: Compaction::TEXT,
            level: rng.below(9) as u32,
            compact: rng.below(3) == 0,
            cols: (rng.below(2) == 0).then(|| 1 + rng.below(12) as u32),
            ..EncodeOptions::default()
        };
        // Level 8 symbols cannot hold the longer messages in 30 columns.
        let Some(r) = try_render(&msg, opts) else {
            continue;
        };
        let l = r.layout;
        assert_eq!(r.left.len(), l.rows);
        assert_eq!(r.cells.len(), l.rows * l.cols);
        assert_eq!(
            usize::from(r.cells[0]),
            l.rows * l.cols - ec_codewords(l.ec_level)
        );
        for row in 0..l.rows {
            assert_eq!(r.left[row], l.left_indicator(row), "{l:?} row {row}");
            if !r.compact {
                assert_eq!(r.right[row], l.right_indicator(row), "{l:?} row {row}");
            }
        }
    }
}

#[test]
fn indicators_cover_all_levels_and_both_forms() {
    for level in 0..=8u32 {
        for compact in [false, true] {
            let opts = EncodeOptions {
                compaction: Compaction::TEXT,
                level,
                compact,
                cols: Some(10),
                ..EncodeOptions::default()
            };
            let r = render("PDF417 ROW INDICATORS 0123456789", opts);
            let l = r.layout;
            assert_eq!(l.ec_level, level as u8);
            for row in 0..l.rows {
                assert_eq!(r.left[row], l.left_indicator(row));
                if !compact {
                    assert_eq!(r.right[row], l.right_indicator(row));
                }
            }
        }
    }
}
