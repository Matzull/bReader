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
        BarcodeFormat::DATA_MATRIX => Some(Symbology::DataMatrix),
        _ => None,
    }
}

impl Engine for RxingEngine {
    fn name(&self) -> &str {
        "rxing"
    }

    fn decode(&mut self, img: &GrayImage) -> Result<Vec<Decoded>, String> {
        let mut hints = DecodeHints {
            TryHarder: Some(true),
            ..Default::default()
        };
        let results = match rxing::helpers::detect_multiple_in_luma_with_hints(
            img.as_raw().clone(),
            img.width(),
            img.height(),
            &mut hints,
        ) {
            Ok(r) => r,
            Err(Exceptions::NotFoundException(_)) => Vec::new(),
            Err(e) => return Err(e.to_string()),
        };
        Ok(results
            .iter()
            .filter_map(|r| {
                map_rxing(r.getBarcodeFormat()).map(|s| (s, r.getText().as_bytes().to_vec()))
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
            .map(|(_, text)| (Symbology::Qr, text.into_bytes()))
            .collect())
    }
}
