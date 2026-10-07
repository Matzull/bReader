use crate::{Quad, Symbology};

#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    pub symbology: Symbology,
    /// Decoded payload bytes, before any charset interpretation.
    pub data: Vec<u8>,
    /// Payload as text when the charset could be determined unambiguously.
    pub text: Option<String>,
    /// Charset of each byte range of `data`, in order and contiguous. Empty
    /// means the whole payload uses the symbology's default interpretation.
    pub segments: Vec<DataSegment>,
    pub content: Content,
    pub quad: Quad,
    pub orientation_deg: f32,
    /// Symbology identifier per ISO/IEC 15424, e.g. "]Q1", "]E0".
    pub aim_id: [u8; 3],
    pub meta: SymbolMeta,
    /// Decoder-specific quality estimate in [0, 1]; never used to accept a result.
    pub quality: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataSegment {
    pub start: usize,
    pub end: usize,
    pub charset: Charset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Charset {
    /// Characters from a restricted set that is ASCII by construction
    /// (QR numeric/alphanumeric modes, Code 39, digits).
    Ascii,
    /// ISO-8859-1, the default of Code 128 (with FNC4) and the ISO default of QR byte mode.
    Latin1,
    /// QR kanji mode: Shift JIS double-byte characters.
    ShiftJis,
    /// Bytes under an explicit ECI designator.
    Eci(u32),
    /// QR byte mode without ECI; resolved by a documented heuristic.
    Undeclared,
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
