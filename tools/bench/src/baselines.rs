use breader::Symbology;
use image::GrayImage;
use rxing::{BarcodeFormat, DecodeHints, Exceptions};

use crate::engine::{Decoded, Engine};

#[derive(Debug, Default)]
pub struct RxingEngine;

fn map_rxing(format: &BarcodeFormat) -> Option<Symbology> {
    match format {
        BarcodeFormat::QR_CODE => Some(Symbology::Qr),
        BarcodeFormat::EAN_13 => Some(Symbology::Ean13),
        BarcodeFormat::EAN_8 => Some(Symbology::Ean8),
        BarcodeFormat::UPC_A => Some(Symbology::UpcA),
        BarcodeFormat::UPC_E => Some(Symbology::UpcE),
        BarcodeFormat::CODE_128 => Some(Symbology::Code128),
        BarcodeFormat::CODE_39 => Some(Symbology::Code39),
        BarcodeFormat::ITF => Some(Symbology::Itf),
        BarcodeFormat::CODABAR => Some(Symbology::Codabar),
        BarcodeFormat::RSS_14 => Some(Symbology::DataBar),
        BarcodeFormat::RSS_EXPANDED => Some(Symbology::DataBarExpanded),
        BarcodeFormat::DATA_MATRIX => Some(Symbology::DataMatrix),
        BarcodeFormat::AZTEC => Some(Symbology::Aztec),
        BarcodeFormat::MICRO_QR_CODE => Some(Symbology::MicroQr),
        BarcodeFormat::RECTANGULAR_MICRO_QR_CODE => Some(Symbology::Rmqr),
        _ => None,
    }
}

impl Engine for RxingEngine {
    fn name(&self) -> &str {
        "rxing"
    }

    fn decode(&mut self, img: &GrayImage) -> Result<Vec<Decoded>, String> {
        // rxing panics or loops on some inputs; decode on a worker thread that is abandoned
        // after a timeout.
        let (raw, width, height) = (img.as_raw().clone(), img.width(), img.height());
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let mut hints = DecodeHints {
                TryHarder: Some(true),
                ..Default::default()
            };
            let result =
                rxing::helpers::detect_multiple_in_luma_with_hints(raw, width, height, &mut hints);
            let _ = tx.send(result);
        });
        let detected = rx
            .recv_timeout(std::time::Duration::from_secs(30))
            .map_err(|e| format!("rxing failed: {e}"))?;
        let results = match detected {
            Ok(r) => r,
            Err(Exceptions::NotFoundException(_)) => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(results
            .iter()
            .filter_map(|r| {
                map_rxing(r.getBarcodeFormat()).map(|s| {
                    let mut text = r.getText().as_bytes().to_vec();
                    // rxing prints the GTIN-14 of DataBar without its (01) identifier.
                    if s == Symbology::DataBar && text.len() == 14 {
                        text.splice(0..0, *b"01");
                    }
                    Decoded::new(s, text)
                })
            })
            .collect())
    }
}

#[derive(Debug, Default)]
pub struct RqrrEngine;

impl Engine for RqrrEngine {
    fn name(&self) -> &str {
        "rqrr"
    }

    fn decode(&mut self, img: &GrayImage) -> Result<Vec<Decoded>, String> {
        let (w, h) = (img.width() as usize, img.height() as usize);
        let raw = img.as_raw();
        let mut prepared = rqrr::PreparedImage::prepare_from_greyscale(w, h, |x, y| raw[y * w + x]);
        Ok(prepared
            .detect_grids()
            .iter()
            .filter_map(|g| g.decode().ok())
            .map(|(_, text)| Decoded::new(Symbology::Qr, text.into_bytes()))
            .collect())
    }
}
