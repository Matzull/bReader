//! The up-scale retry rescans small frames that yielded nothing.

use breader::{Effort, ImageView, Reader, ScanOptions, Symbology};
use qrcode::{Color, EcLevel, QrCode, Version};

const TEXT: &str = "HTTPS://EXAMPLE.ORG/1";

/// A version 2 QR code at one pixel per module, `quiet` modules of white around it.
fn tiny_qr(quiet: usize) -> (Vec<u8>, usize) {
    let code = QrCode::with_version(TEXT.as_bytes(), Version::Normal(2), EcLevel::L).unwrap();
    let dim = code.width();
    let side = dim + 2 * quiet;
    let mut px = vec![255u8; side * side];
    for (i, c) in code.to_colors().into_iter().enumerate() {
        if c == Color::Dark {
            px[(quiet + i / dim) * side + quiet + i % dim] = 0;
        }
    }
    (px, side)
}

fn scan(px: &[u8], side: usize, effort: Effort, retry: Option<usize>) -> Vec<breader::Symbol> {
    let mut options = ScanOptions::with_effort(effort);
    options.upscale_retry = retry;
    let mut reader = Reader::with_options(options);
    reader.scan(ImageView::from_packed(px, side, side).unwrap())
}

#[test]
fn one_pixel_per_module_qr_reads_only_with_the_retry() {
    let (px, side) = tiny_qr(4);
    assert_eq!(side, 33);
    for effort in [Effort::Balanced, Effort::Exhaustive] {
        assert!(scan(&px, side, effort, Some(0)).is_empty(), "{effort:?}");
        let found = scan(&px, side, effort, None);
        assert_eq!(found.len(), 1, "{effort:?}");
        assert_eq!(found[0].symbology, Symbology::Qr);
        assert_eq!(found[0].text.as_deref(), Some(TEXT));
        // Corners are in the coordinates of the small frame.
        let c = found[0].quad.center();
        assert!(
            (c.x - 16.5).abs() < 2.0 && (c.y - 16.5).abs() < 2.0,
            "{c:?}"
        );
        for p in found[0].quad.corners {
            assert!(
                (3.0..30.0).contains(&p.x) && (3.0..30.0).contains(&p.y),
                "{p:?}"
            );
        }
    }
}

#[test]
fn fast_does_not_retry_unless_asked() {
    let (px, side) = tiny_qr(4);
    assert!(scan(&px, side, Effort::Fast, None).is_empty());
    assert_eq!(scan(&px, side, Effort::Fast, Some(600)).len(), 1);
}

#[test]
fn frames_over_the_limit_are_not_retried() {
    let (px, side) = tiny_qr(4);
    assert!(scan(&px, side, Effort::Balanced, Some(side - 1)).is_empty());
    assert_eq!(scan(&px, side, Effort::Balanced, Some(side)).len(), 1);
}
