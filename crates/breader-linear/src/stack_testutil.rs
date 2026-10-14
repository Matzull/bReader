//! Test helpers for stacked GS1 DataBar symbols.

use breader_core::ImageBuffer;

/// Rows of a GS1 DataBar Stacked (`kind` 0, `text` the 13-digit body),
/// Stacked Omnidirectional (1) or Expanded Stacked (2, with `columns` blocks
/// per row) from the independent encoder: each grid row is one module tall.
pub fn databar_stacked_grid(kind: u8, text: &str, columns: usize) -> Vec<Vec<bool>> {
    use anyd::codes::databar::DataBarEncoder;
    use anyd::output::Encoding;
    use anyd::traits::Encode;
    let enc = DataBarEncoder::new();
    let symbol = match kind {
        0 => enc.build_stacked(text.as_bytes()),
        1 => enc.build_stacked_omni(text.as_bytes()),
        _ => enc.build_expanded_stacked(text.as_bytes(), columns),
    }
    .unwrap();
    let Encoding::Matrix(m) = enc.encode(&symbol).unwrap() else {
        panic!("matrix expected");
    };
    (0..m.height())
        .map(|y| (0..m.width()).map(|x| m.get(x, y)).collect())
        .collect()
}

/// Heights in modules of the rows of a grid from [`databar_stacked_grid`]:
/// data rows `data` tall, separator rows one module.
pub fn stacked_row_heights(kind: u8, rows: usize, data: f32) -> Vec<f32> {
    (0..rows)
        .map(|y| {
            let is_data = match kind {
                0 => y % 2 == 0,
                1 => y == 0 || y == 4,
                _ => y % 4 == 0,
            };
            if is_data { data } else { 1.0 }
        })
        .collect()
}

/// A grid drawn with rows `heights` modules tall.
#[derive(Debug, Clone, Copy)]
pub struct GridPlacement<'a> {
    pub heights: &'a [f32],
    pub module_px: f32,
    pub center: (f32, f32),
    pub angle_deg: f32,
    pub dark: u8,
}

/// Draws `grid` (true = dark) with 3x3 supersampling, centred and turned
/// clockwise by the angle of the placement.
pub fn draw_grid(img: &mut ImageBuffer, grid: &[Vec<bool>], p: GridPlacement<'_>) {
    let (w, h) = (img.width(), img.height());
    let (s, c) = p.angle_deg.to_radians().sin_cos();
    let cols = grid[0].len();
    let total: f32 = p.heights.iter().sum::<f32>() * p.module_px;
    let (half_w, half_h) = (cols as f32 * p.module_px / 2.0, total / 2.0);
    for y in 0..h {
        for x in 0..w {
            let mut covered = 0;
            for sy in 0..3 {
                for sx in 0..3 {
                    let px = x as f32 + (sx as f32 + 0.5) / 3.0 - 0.5 - p.center.0;
                    let py = y as f32 + (sy as f32 + 0.5) / 3.0 - 0.5 - p.center.1;
                    let u = px * c + py * s + half_w;
                    let v = -px * s + py * c + half_h;
                    if u < 0.0 || v < 0.0 || v >= total {
                        continue;
                    }
                    let mut acc = 0.0;
                    let mut row = 0;
                    for (r, &hh) in p.heights.iter().enumerate() {
                        acc += hh * p.module_px;
                        if v < acc {
                            row = r;
                            break;
                        }
                    }
                    let idx = (u / p.module_px) as usize;
                    covered += usize::from(idx < cols && grid[row][idx]);
                }
            }
            if covered > 0 {
                let a = covered as f32 / 9.0;
                let base = f32::from(img.row_mut(y)[x]);
                img.row_mut(y)[x] = (base + (f32::from(p.dark) - base) * a).round() as u8;
            }
        }
    }
}
