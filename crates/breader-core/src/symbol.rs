use crate::{Quad, Symbology};

/// A decoded and validated symbol.
///
/// Output only and non-exhaustive: read the fields, or build one with
/// [`Symbol::new`] (for custom readers and tests), then assign the rest.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
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

impl Symbol {
    /// A symbol with the given essentials. `text` is `None`, `segments` empty,
    /// `content` [`Content::Unparsed`], `orientation_deg` taken from `quad` and
    /// `quality` 1.0; assign the fields to change them.
    pub fn new(
        symbology: Symbology,
        data: Vec<u8>,
        quad: Quad,
        aim_id: [u8; 3],
        meta: SymbolMeta,
    ) -> Self {
        Self {
            symbology,
            data,
            text: None,
            segments: Vec::new(),
            content: Content::Unparsed,
            quad,
            orientation_deg: quad.orientation_deg(),
            aim_id,
            meta,
            quality: 1.0,
        }
    }
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
    /// QR Code and Micro QR Code details.
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
#[non_exhaustive]
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
    /// The symbol programs the reader (Reader Programming codeword first);
    /// applications that do not expect it should ignore it.
    pub reader_init: bool,
}

/// Details of a decoded Aztec Code symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// Value 0 to 255 of an Aztec Rune; the payload is its three-digit decimal text.
    pub rune: Option<u8>,
    /// Reader initialisation symbol (a one-layer symbol that flags the reader
    /// to be programmed by its data).
    pub reader_init: bool,
    /// ECI designators in order of appearance.
    pub eci: Vec<u32>,
}

/// Details of a decoded PDF417 symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    /// The symbol programs the reader (codeword 921 first); applications that
    /// do not expect it should ignore it.
    pub reader_init: bool,
}

/// Macro PDF417 control block: the position of a symbol in a multi-symbol file.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
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
#[non_exhaustive]
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
    /// Error detection only, no correction (Micro QR version M1).
    DetectionOnly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
/// Details of a decoded QR Code, Micro QR Code or rMQR Code.
pub struct QrMeta {
    /// Symbol version, 1 to 40. For [`Symbology::MicroQr`](crate::Symbology::MicroQr)
    /// 1 to 4 for M1 to M4; for [`Symbology::Rmqr`](crate::Symbology::Rmqr) 1 to 32,
    /// the version indicator plus one (R7x43 is 1, R17x139 is 32).
    pub version: u8,
    /// Modules per row.
    pub width: u16,
    /// Modules per column (equal to `width` for QR and Micro QR).
    pub height: u16,
    /// Error correction level; [`EcLevel::DetectionOnly`] for Micro QR M1.
    pub ec_level: EcLevel,
    /// Data mask pattern, 0 to 7 (0 to 3 for Micro QR; always 0 for rMQR, which has one fixed pattern).
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
#[non_exhaustive]
pub struct LinearMeta {
    /// EAN-2 / EAN-5 supplement digits, if present.
    pub addon: Option<String>,
    /// Whether an optional check character was present and verified.
    pub checksum_verified: bool,
    /// Independent scanlines that produced this exact result.
    pub lines_agreeing: u16,
    /// Codabar start and stop characters (`b'A'..=b'D'`), which `data`
    /// leaves out. `None` for other symbologies.
    pub codabar_start_stop: Option<(u8, u8)>,
    /// GS1 DataBar Expanded: symbol characters in the row, check character
    /// included (4 to 22). `None` for other symbologies.
    pub databar_segments: Option<u8>,
    /// GS1 DataBar Stacked, Stacked Omnidirectional and Expanded Stacked:
    /// rows of the symbol (2 for the first two, 2 to 11 for Expanded
    /// Stacked), which are reported as `DataBar` and `DataBarExpanded`
    /// ([D-048](../../docs/decisions.md#d-048)). `None` for one-row symbols
    /// and other symbologies.
    pub databar_rows: Option<u8>,
}

impl QrMeta {
    /// Details for a symbol of the given version, size and level; `mask` and
    /// `errors_corrected` are 0, `eci` empty and `mirrored` false until assigned.
    pub fn new(version: u8, width: u16, height: u16, ec_level: EcLevel) -> Self {
        Self {
            version,
            width,
            height,
            ec_level,
            mask: 0,
            errors_corrected: 0,
            eci: Vec::new(),
            mirrored: false,
        }
    }
}

impl DataMatrixMeta {
    /// Details for a symbol of `rows` x `cols` modules; the counters are 0, the
    /// flags false and the lists empty until assigned.
    pub fn new(rows: u16, cols: u16) -> Self {
        Self {
            rows,
            cols,
            errors_corrected: 0,
            mirrored: false,
            eci: Vec::new(),
            structured_append: None,
            reader_init: false,
        }
    }
}

impl AztecMeta {
    /// Details for a symbol of `layers` layers (0 for a Rune); the other fields
    /// are 0, false, empty or `None` until assigned.
    pub fn new(compact: bool, layers: u8) -> Self {
        Self {
            compact,
            layers,
            data_codewords: 0,
            ec_codewords: 0,
            errors_corrected: 0,
            erasures: 0,
            mirrored: false,
            rune: None,
            reader_init: false,
            eci: Vec::new(),
        }
    }
}

impl Pdf417Meta {
    /// Details for a symbol of `rows` x `cols` data columns at error correction
    /// level `ec_level`; the counters are 0, the flags false and `macro_pdf417` `None` until assigned.
    pub fn new(rows: u16, cols: u16, ec_level: u8) -> Self {
        Self {
            rows,
            cols,
            ec_level,
            errors_corrected: 0,
            erasures: 0,
            compact: false,
            macro_pdf417: None,
            reader_init: false,
        }
    }
}

impl LinearMeta {
    /// Details of a 1D read confirmed by `lines_agreeing` scanlines; the add-on
    /// and the symbology-specific fields are empty and the check unverified until assigned.
    pub fn new(lines_agreeing: u16) -> Self {
        Self {
            lines_agreeing,
            ..Self::default()
        }
    }
}

impl MacroPdf417 {
    /// A control block for segment `segment_index` of the file `file_id`; the
    /// optional fields are `None` and `last_segment` false until assigned.
    pub fn new(segment_index: u32, file_id: String) -> Self {
        Self {
            segment_index,
            file_id,
            ..Self::default()
        }
    }
}
