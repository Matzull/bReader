use crate::{Quad, Symbology};

/// A decoded and validated symbol.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    /// Symbology of the result.
    pub symbology: Symbology,
    /// Decoded payload bytes, before any charset interpretation.
    pub data: Vec<u8>,
    /// Payload as text when the charset could be determined unambiguously.
    pub text: Option<String>,
    /// Charset of each byte range of `data`, in order and contiguous. Empty
    /// means the whole payload uses the symbology's default interpretation.
    pub segments: Vec<DataSegment>,
    /// Classification of the payload, filled by the content post-processors.
    pub content: Content,
    /// Corners in frame pixels (see [`Quad`]).
    pub quad: Quad,
    /// Rotation of the symbol in degrees, as [`Quad::orientation_deg`].
    pub orientation_deg: f32,
    /// Symbology identifier per ISO/IEC 15424, e.g. "]Q1", "]E0".
    pub aim_id: [u8; 3],
    /// Symbology-specific details.
    pub meta: SymbolMeta,
    /// Decoder-specific quality estimate in [0, 1]; never used to accept a result.
    pub quality: f32,
}

/// A byte range of [`Symbol::data`] with its charset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataSegment {
    /// First byte of the range.
    pub start: usize,
    /// One past the last byte of the range.
    pub end: usize,
    /// Charset of the bytes in the range.
    pub charset: Charset,
}

/// Character encoding of a payload segment.
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
    /// Bytes under an explicit ECI designator (ISO/IEC 15424 assignment number).
    Eci(u32),
    /// QR byte mode without ECI; resolved by a documented heuristic.
    Undeclared,
}

/// What the payload turned out to be.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum Content {
    /// Not classified yet (no content post-processor ran).
    #[default]
    Unparsed,
    /// Human-readable text.
    Text,
    /// Non-textual bytes.
    Binary,
    /// A URL.
    Url,
    /// GS1 element strings, split into application identifiers.
    Gs1(Vec<Gs1Element>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// One GS1 element string.
pub struct Gs1Element {
    /// Application identifier, e.g. "01".
    pub ai: String,
    /// Element value without the AI.
    pub value: String,
}

/// Symbology-specific details of a [`Symbol`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SymbolMeta {
    /// QR Code details.
    Qr(QrMeta),
    /// 1D symbology details.
    Linear(LinearMeta),
    /// Data Matrix details.
    DataMatrix(DataMatrixMeta),
    /// PDF417 details.
    Pdf417(Pdf417Meta),
    /// Aztec Code details.
    Aztec(AztecMeta),
}

/// Details of a decoded Data Matrix symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataMatrixMeta {
    /// Symbol height in modules, including the finder and clock patterns.
    pub rows: u16,
    /// Symbol width in modules, including the finder and clock patterns.
    pub cols: u16,
    /// Codewords changed by Reed-Solomon correction, summed over all blocks.
    pub errors_corrected: u32,
    /// The symbol was read mirrored (transposed).
    pub mirrored: bool,
    /// ECI designators in order of appearance.
    pub eci: Vec<u32>,
    /// Structured Append: (position, total, file id) when present; position and total are 1-based.
    pub structured_append: Option<(u8, u8, u16)>,
}

/// Details of a decoded Aztec Code symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AztecMeta {
    /// Compact symbol (1 to 4 layers, or an Aztec Rune) rather than full-range.
    pub compact: bool,
    /// Number of data layers: 1 to 4 compact, 1 to 32 full-range, 0 for a Rune.
    pub layers: u8,
    /// Data codewords of the symbol, padding included (0 for a Rune).
    pub data_codewords: u16,
    /// Check codewords of the symbol (0 for a Rune).
    pub ec_codewords: u16,
    /// Codewords changed by Reed-Solomon correction that were not erasures,
    /// mode message included.
    pub errors_corrected: u32,
    /// Data codewords that were invalid by construction and recovered as erasures.
    pub erasures: u32,
    /// The symbol was read mirrored (transposed).
    pub mirrored: bool,
    /// Value 0 to 255 of an Aztec Rune; the payload is its decimal text.
    pub rune: Option<u8>,
    /// ECI designators in order of appearance.
    pub eci: Vec<u32>,
}

/// Details of a decoded PDF417 symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pdf417Meta {
    /// Number of rows, 3 to 90.
    pub rows: u16,
    /// Number of data columns, 1 to 30 (row indicators not counted).
    pub cols: u16,
    /// Error correction level, 0 to 8 (2^(level + 1) check codewords).
    pub ec_level: u8,
    /// Codewords changed by Reed-Solomon correction that were not erasures.
    pub errors_corrected: u32,
    /// Codewords that were unreadable and recovered as erasures.
    pub erasures: u32,
    /// Compact (truncated) PDF417: no right row indicators and a single-bar stop.
    pub compact: bool,
    /// Macro PDF417 control block, when the symbol is part of a file.
    pub macro_pdf417: Option<MacroPdf417>,
}

/// Macro PDF417 control block: the position of a symbol in a multi-symbol file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MacroPdf417 {
    /// Zero-based position of this symbol in the file.
    pub segment_index: u32,
    /// File identifier: the codewords of the file ID as three-digit groups.
    pub file_id: String,
    /// The terminator codeword marks this symbol as the last segment.
    pub last_segment: bool,
    /// File name.
    pub file_name: Option<String>,
    /// Total number of segments.
    pub segment_count: Option<u32>,
    /// Time stamp of the file, as encoded (seconds since 1970-01-01 UTC).
    pub timestamp: Option<u64>,
    /// Sender.
    pub sender: Option<String>,
    /// Addressee.
    pub addressee: Option<String>,
    /// File size in bytes.
    pub file_size: Option<u64>,
    /// 16-bit CRC of the file.
    pub checksum: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// QR error correction level.
pub enum EcLevel {
    /// About 7 % of codewords recoverable.
    L,
    /// About 15 %.
    M,
    /// About 25 %.
    Q,
    /// About 30 %.
    H,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Details of a decoded QR Code.
pub struct QrMeta {
    /// Symbol version, 1 to 40.
    pub version: u8,
    /// Error correction level.
    pub ec_level: EcLevel,
    /// Data mask pattern, 0 to 7.
    pub mask: u8,
    /// Codeword errors fixed by Reed-Solomon.
    pub errors_corrected: u32,
    /// ECI designators in order of appearance.
    pub eci: Vec<u32>,
    /// The symbol was read as a mirror image.
    pub mirrored: bool,
}

/// Details of a decoded 1D symbol.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LinearMeta {
    /// EAN-2 / EAN-5 supplement digits, if present.
    pub addon: Option<String>,
    /// Whether an optional check character was present and verified.
    pub checksum_verified: bool,
    /// Independent scanlines that produced this exact result.
    pub lines_agreeing: u16,
}
