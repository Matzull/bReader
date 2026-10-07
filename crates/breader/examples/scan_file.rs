//! Scans an image file and prints every symbol found.
//!
//! cargo run --example scan_file --features image -- data/synth/clean/clean_0001.png

use breader::{Content, Reader};

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: scan_file <image>");
        std::process::exit(2);
    };
    let mut reader = Reader::new();
    let symbols = match reader.scan_file(&path) {
        Ok(symbols) => symbols,
        Err(e) => {
            eprintln!("{path}: {e}");
            std::process::exit(1);
        }
    };
    if symbols.is_empty() {
        println!("no symbols found");
    }
    for s in &symbols {
        println!("{}", s.symbology.name());
        match &s.text {
            Some(text) => println!("  text: {text:?}"),
            None => println!("  data: {:02x?}", s.data),
        }
        if let Content::Gs1(elements) = &s.content {
            println!("  gs1:  {}", breader::gs1::hri(elements));
        }
        let c = s.quad.corners;
        println!(
            "  corners: ({:.0},{:.0}) ({:.0},{:.0}) ({:.0},{:.0}) ({:.0},{:.0}), rotation {:.0} deg",
            c[0].x, c[0].y, c[1].x, c[1].y, c[2].x, c[2].y, c[3].x, c[3].y, s.orientation_deg
        );
    }
}
