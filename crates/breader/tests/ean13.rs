//! End-to-end check of the default pipeline on a rendered EAN-13.

use breader::{
    Content, EanAddonPolicy, Effort, ImageBuffer, ImageView, LinearMeta, Reader, ScanOptions,
    SymbolMeta, Symbology,
};

const L_CODE: [&str; 10] = [
    "0001101", "0011001", "0010011", "0111101", "0100011", "0110001", "0101111", "0111011",
    "0110111", "0001011",
];
const PARITY: [&str; 10] = [
    "LLLLLL", "LLGLGG", "LLGGLG", "LLGGGL", "LGLLGG", "LGGLLG", "LGGGLL", "LGLGLG", "LGLGGL",
    "LGGLGL",
];

fn bits(s: &str) -> Vec<bool> {
    s.bytes().map(|b| b == b'1').collect()
}

fn digit(d: u8, parity: u8) -> Vec<bool> {
    let l = bits(L_CODE[d as usize]);
    match parity {
        b'L' => l,
        b'G' => l.iter().rev().map(|b| !b).collect(),
        _ => l.iter().map(|b| !b).collect(),
    }
}

fn ean13(text: &str) -> Vec<bool> {
    let d: Vec<u8> = text.bytes().map(|b| b - b'0').collect();
    let parity = PARITY[d[0] as usize].as_bytes();
    let mut m = bits("101");
    for i in 0..6 {
        m.extend(digit(d[1 + i], parity[i]));
    }
    m.extend(bits("01010"));
    for &x in &d[7..] {
        m.extend(digit(x, b'R'));
    }
    m.extend(bits("101"));
    m
}

/// Light page with the symbol's bars drawn as dark columns.
fn render(modules: &[bool], module_px: usize, height: usize, quiet: usize) -> (ImageBuffer, usize) {
    let width = modules.len() * module_px + 2 * quiet * module_px;
    let mut img = ImageBuffer::new(width, height + 40);
    img.as_mut_slice().fill(235);
    for y in 20..20 + height {
        let row = img.row_mut(y);
        for (i, &bar) in modules.iter().enumerate() {
            if bar {
                let x0 = (quiet + i) * module_px;
                row[x0..x0 + module_px].fill(25);
            }
        }
    }
    (img, width)
}

#[test]
fn reads_a_rendered_ean13_with_default_options() {
    let (img, width) = render(&ean13("5901234123457"), 3, 110, 12);
    let mut reader = Reader::new();
    let symbols = reader.scan(img.view());
    assert_eq!(symbols.len(), 1, "{symbols:?}");
    let s = &symbols[0];
    assert_eq!(s.symbology, Symbology::Ean13);
    assert_eq!(s.data, b"5901234123457");
    assert_eq!(s.text.as_deref(), Some("5901234123457"));
    assert_eq!(s.content, Content::Text);
    assert_eq!(&s.aim_id, b"]E0");
    assert!(s.orientation_deg < 2.0 || s.orientation_deg > 358.0);
    let SymbolMeta::Linear(LinearMeta {
        addon,
        checksum_verified,
        lines_agreeing,
    }) = &s.meta
    else {
        panic!("linear metadata expected");
    };
    assert_eq!(addon, &None);
    assert!(checksum_verified);
    assert!(*lines_agreeing >= 2);
    let center = s.quad.center();
    assert!((center.x - width as f32 / 2.0).abs() < 10.0, "{center:?}");
    assert!(center.y > 20.0 && center.y < 130.0, "{center:?}");
}

#[test]
fn reads_the_same_symbol_upside_down_and_from_a_strided_view() {
    let modules: Vec<bool> = ean13("4006381333931").into_iter().rev().collect();
    let (img, width) = render(&modules, 2, 90, 12);
    let height = img.height();
    // Copy into a wider buffer so the view has a row stride larger than the width.
    let stride = width + 17;
    let mut data = vec![200u8; stride * height];
    for y in 0..height {
        data[y * stride..y * stride + width].copy_from_slice(img.view().row(y));
    }
    let view = ImageView::new(&data, width, height, stride).unwrap();
    let mut reader = Reader::with_options(ScanOptions::with_effort(Effort::Fast));
    let symbols = reader.scan(view);
    assert_eq!(symbols.len(), 1, "{symbols:?}");
    assert_eq!(symbols[0].data, b"4006381333931");
    let o = symbols[0].orientation_deg;
    assert!((o - 180.0).abs() < 2.0, "{o}");
}

#[test]
fn blank_and_unrelated_images_yield_nothing() {
    let mut reader = Reader::new();
    let blank = ImageBuffer::new(300, 200);
    assert!(reader.scan(blank.view()).is_empty());
    // A valid structure with a broken check digit is not a symbol.
    let (img, _) = render(&ean13("5901234123458"), 3, 110, 12);
    assert!(reader.scan(img.view()).is_empty());
}

#[test]
fn addon_policy_is_honored_by_the_facade() {
    let mut modules = ean13("9780735200449");
    modules.extend([false; 9]);
    // EAN-2 "12": 12 % 4 = 0 selects the parity pattern LL.
    modules.extend(bits("1011"));
    modules.extend(digit(1, b'L'));
    modules.extend(bits("01"));
    modules.extend(digit(2, b'L'));
    let (img, _) = render(&modules, 3, 110, 12);
    let mut reader = Reader::with_options(ScanOptions {
        ean_addon: EanAddonPolicy::Optional,
        ..ScanOptions::default()
    });
    let symbols = reader.scan(img.view());
    assert_eq!(symbols.len(), 1, "{symbols:?}");
    assert_eq!(symbols[0].data, b"9780735200449");
    let SymbolMeta::Linear(meta) = &symbols[0].meta else {
        panic!("linear metadata expected");
    };
    assert_eq!(meta.addon.as_deref(), Some("12"));
    assert_eq!(&symbols[0].aim_id, b"]E3");
}
