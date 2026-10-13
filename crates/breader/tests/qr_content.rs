//! End-to-end text decoding and classification of rendered QR codes.

use breader::{Charset, Content, DataSegment, Gs1Element, ImageBuffer, Reader, Symbology};
use qrcode::bits::Bits;
use qrcode::{Color, EcLevel, QrCode, Version};

fn render(code: &QrCode) -> ImageBuffer {
    const PX: usize = 6;
    const QUIET: usize = 4;
    let dim = code.width();
    let side = (dim + 2 * QUIET) * PX;
    let mut img = ImageBuffer::new(side, side);
    img.as_mut_slice().fill(240);
    let colors = code.to_colors();
    for y in 0..side {
        let row = img.row_mut(y);
        for (x, p) in row.iter_mut().enumerate() {
            let (mx, my) = (x / PX, y / PX);
            if mx >= QUIET && my >= QUIET && mx < QUIET + dim && my < QUIET + dim {
                let c = colors[(my - QUIET) * dim + (mx - QUIET)];
                if c == Color::Dark {
                    *p = 20;
                }
            }
        }
    }
    img
}

fn scan(build: impl FnOnce(&mut Bits)) -> breader::Symbol {
    let mut bits = Bits::new(Version::Normal(5));
    build(&mut bits);
    bits.push_terminator(EcLevel::M).unwrap();
    let code = QrCode::with_bits(bits, EcLevel::M).unwrap();
    let img = render(&code);
    let found = Reader::new().scan(img.view());
    assert_eq!(found.len(), 1, "{found:?}");
    found.into_iter().next().unwrap()
}

#[test]
fn mixed_segments_decode_with_their_own_charsets() {
    let s = scan(|b| {
        b.push_alphanumeric_data(b"HELLO ").unwrap();
        b.push_kanji_data(&[0x93, 0xfa, 0x96, 0x7b]).unwrap();
        b.push_eci_designator(26).unwrap();
        b.push_byte_data("é€".as_bytes()).unwrap();
    });
    assert_eq!(s.symbology, Symbology::Qr);
    assert_eq!(s.text.as_deref(), Some("HELLO 日本é€"));
    assert_eq!(s.content, Content::Text);
    assert_eq!(&s.aim_id, b"]Q2");
    let seg = |start, end, charset| DataSegment {
        start,
        end,
        charset,
    };
    assert_eq!(
        s.segments,
        [
            seg(0, 6, Charset::Ascii),
            seg(6, 10, Charset::ShiftJis),
            seg(10, 15, Charset::Eci(26)),
        ]
    );
}

#[test]
fn gs1_qr_is_parsed() {
    let s = scan(|b| {
        b.push_fnc1_first_position().unwrap();
        b.push_alphanumeric_data(b"0109506000134352%10AB12")
            .unwrap();
    });
    assert_eq!(&s.aim_id, b"]Q3");
    assert_eq!(s.data, b"0109506000134352\x1d10AB12");
    let el = |ai: &str, value: &str| Gs1Element {
        ai: ai.into(),
        value: value.into(),
    };
    assert_eq!(
        s.content,
        Content::Gs1(vec![el("01", "09506000134352"), el("10", "AB12")])
    );
    if let Content::Gs1(elements) = &s.content {
        assert_eq!(breader::gs1::hri(elements), "(01)09506000134352(10)AB12");
    }
}

#[test]
fn url_qr_is_classified() {
    let s = scan(|b| b.push_byte_data(b"https://example.com/x?y=1").unwrap());
    assert_eq!(s.content, Content::Url);
    // Byte mode without an ECI designator is the undeclared charset.
    assert_eq!(&s.aim_id, b"]Q1");
}
