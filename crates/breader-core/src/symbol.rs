use crate::{Quad, Symbology};

#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub symbology: Symbology,
    /// Decoded payload bytes, before any charset interpretation.
    pub data: Vec<u8>,
    /// Payload as text when the charset could be determined unambiguously.
    pub text: Option<String>,
    pub content: Content,
    pub quad: Quad,
    pub orientation_deg: f32,
    /// Symbology identifier per ISO/IEC 15424, e.g. "]Q1", "]E0".
    pub aim_id: [u8; 3],
    pub meta: SymbolMeta,
    /// Decoder-specific quality estimate in [0, 1]; never used to accept a result.
    pub quality: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum Content {
    #[default]
    Unparsed,
    Text,
    Binary,
    Url,
    Gs1(Vec<Gs1Element>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gs1Element {
    pub ai: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SymbolMeta {
    Qr(QrMeta),
    Linear(LinearMeta),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EcLevel {
    L,
    M,
    Q,
    H,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrMeta {
    pub version: u8,
    pub ec_level: EcLevel,
    pub mask: u8,
    pub errors_corrected: u32,
    /// ECI designators in order of appearance.
    pub eci: Vec<u32>,
    pub mirrored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinearMeta {
    /// EAN-2 / EAN-5 supplement digits, if present.
    pub addon: Option<String>,
    /// Whether an optional check character was present and verified.
    pub checksum_verified: bool,
    /// Independent scanlines that produced this exact result.
    pub lines_agreeing: u16,
}
