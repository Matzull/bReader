//! Data codewords to bytes (ISO/IEC 15438 text, byte and numeric
//! compaction, ECI and Macro PDF417 control blocks).
//!
//! Decisions that the standard leaves to the reader or that vary between
//! editions:
//! - Default interpretation: ISO/IEC 15438:2006 makes it ISO/IEC 8859-1
//!   (ECI 000003); the 1994 and 2001 texts used code page 437. Data outside an
//!   ECI is reported as `Charset::Latin1`, the same default as Data Matrix, so
//!   `TextDecoder` treats it as a declared segment.
//! - Latch to text (900) always restarts in the Alpha submode, so the padding
//!   codewords are harmless wherever they follow.
//! - A shift that is the last sub-codeword before something that is not a
//!   text codeword is padding (encoders fill an odd sub-codeword count with 29)
//!   and is dropped.
//! - The Macro PDF417 control block is accepted anywhere, once per symbol.

use breader_core::{Charset, DataSegment, MacroPdf417, Reject};

pub(super) struct Parsed {
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub macro_pdf417: Option<MacroPdf417>,
    pub reader_init: bool,
    pub eci_used: bool,
}

const TEXT_LATCH: u16 = 900;
const BYTE_LATCH: u16 = 901;
const NUMERIC_LATCH: u16 = 902;
const BYTE_SHIFT: u16 = 913;
const READER_INIT: u16 = 921;
const MACRO_TERMINATOR: u16 = 922;
const MACRO_FIELD: u16 = 923;
const BYTE_LATCH_MULTIPLE_OF_6: u16 = 924;
const ECI_ONE_CODEWORD_HIGH: u16 = 925;
const ECI_TWO_CODEWORDS: u16 = 926;
const ECI_ONE_CODEWORD: u16 = 927;
const MACRO_BLOCK: u16 = 928;

/// Numeric groups hold at most 15 codewords (44 digits).
const NUMERIC_GROUP: usize = 15;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Text,
    /// Byte compaction; the flag is set after latch 924 (a multiple of 6 bytes).
    Byte {
        whole_groups: bool,
    },
    Numeric,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sub {
    Alpha,
    Lower,
    Mixed,
    Punct,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shift {
    None,
    /// From Lower: the next character is read from Alpha.
    Alpha,
    /// From Alpha, Lower or Mixed: the next character is read from Punct.
    Punct,
}

const MIXED: [u8; 15] = *b"&\r\t,:#-.$/+%*=^";
const PUNCT: [u8; 29] = *b";<>@[\\]_`~!\r\t,:\n-.$/\"|*()?{}'";

struct Text {
    sub: Sub,
    shift: Shift,
}

impl Text {
    fn new() -> Self {
        Self {
            sub: Sub::Alpha,
            shift: Shift::None,
        }
    }

    /// Applies one sub-codeword (0 to 29).
    fn feed(&mut self, v: u8, out: &mut Vec<u8>) -> Result<(), Reject> {
        match std::mem::replace(&mut self.shift, Shift::None) {
            Shift::Punct => {
                out.push(*PUNCT.get(usize::from(v)).ok_or(Reject::Format)?);
                return Ok(());
            }
            Shift::Alpha => {
                match v {
                    0..=25 => out.push(b'A' + v),
                    26 => out.push(b' '),
                    _ => return Err(Reject::Format),
                }
                return Ok(());
            }
            Shift::None => {}
        }
        match (self.sub, v) {
            (Sub::Alpha, 0..=25) => out.push(b'A' + v),
            (Sub::Lower, 0..=25) => out.push(b'a' + v),
            (Sub::Mixed, 0..=9) => out.push(b'0' + v),
            (Sub::Mixed, 10..=24) => out.push(MIXED[usize::from(v) - 10]),
            (Sub::Punct, 0..=28) => out.push(PUNCT[usize::from(v)]),
            (Sub::Alpha | Sub::Lower | Sub::Mixed, 26) => out.push(b' '),
            (Sub::Alpha, 27) => self.sub = Sub::Lower,
            (Sub::Lower, 27) => self.shift = Shift::Alpha,
            (Sub::Mixed, 27) => self.sub = Sub::Lower,
            (Sub::Alpha | Sub::Lower, 28) => self.sub = Sub::Mixed,
            (Sub::Mixed, 28) => self.sub = Sub::Alpha,
            (Sub::Mixed, 25) => self.sub = Sub::Punct,
            (Sub::Alpha | Sub::Lower | Sub::Mixed, 29) => self.shift = Shift::Punct,
            (Sub::Punct, 29) => self.sub = Sub::Alpha,
            _ => return Err(Reject::Format),
        }
        Ok(())
    }

    /// One text compaction codeword: two sub-codewords.
    fn codeword(&mut self, cw: u16, out: &mut Vec<u8>) -> Result<(), Reject> {
        self.feed((cw / 30) as u8, out)?;
        self.feed((cw % 30) as u8, out)
    }

    fn reset(&mut self) {
        *self = Self::new();
    }

    /// Drops a shift left over at the end of a text run (padding).
    fn end_run(&mut self) {
        self.shift = Shift::None;
    }
}

/// Codewords at the end of a byte compaction run of `len` codewords that are
/// single bytes.
fn single_bytes(len: usize, whole_groups: bool) -> Result<usize, Reject> {
    match (len % 5, whole_groups) {
        (0, true) => Ok(0),
        (_, true) => Err(Reject::Format),
        (0, false) => Ok(len.min(5)),
        (r, false) => Ok(r),
    }
}

/// A byte compaction run that ECI designators may interrupt: the data
/// codewords, and where each designator sits among them (count of data
/// codewords before it) with its value, and the index after the run. The
/// groups of five codewords run on across the designators.
type ByteRun = (Vec<u16>, Vec<(usize, u32)>, usize);

fn byte_run(cws: &[u16], start: usize) -> Result<ByteRun, Reject> {
    let (mut run, mut ecis, mut i) = (Vec::new(), Vec::new(), start);
    while let Some(&c) = cws.get(i) {
        if c < 900 {
            run.push(c);
            i += 1;
        } else if matches!(
            c,
            ECI_ONE_CODEWORD_HIGH | ECI_TWO_CODEWORDS | ECI_ONE_CODEWORD
        ) {
            let (value, used) = eci(cws, i)?;
            ecis.push((run.len(), value));
            i += used;
        } else {
            break;
        }
    }
    Ok((run, ecis, i))
}

/// Appends the bytes of a byte compaction run. Five codewords make six
/// bytes; after latch 901 the byte count is not a multiple of 6, so the last
/// one to five codewords of the run are single bytes (also when there are
/// five). After latch 924 the run holds whole groups only.
fn flush_bytes(run: &[u16], whole_groups: bool, out: &mut Vec<u8>) -> Result<(), Reject> {
    let singles = single_bytes(run.len(), whole_groups)?;
    let (groups, singles) = run.split_at(run.len() - singles);
    for g in groups.as_chunks::<5>().0 {
        let v = g.iter().fold(0u64, |acc, &c| acc * 900 + u64::from(c));
        // Six bytes hold 48 bits; five base-900 digits reach further.
        if v >> 48 != 0 {
            return Err(Reject::Format);
        }
        out.extend_from_slice(&v.to_be_bytes()[2..]);
    }
    for &c in singles {
        out.push(u8::try_from(c).map_err(|_| Reject::Format)?);
    }
    Ok(())
}

/// Appends the digits of one numeric group (1 to 15 codewords): the base 900
/// value written in decimal is a leading 1 followed by the digits.
fn numeric_group(group: &[u16], out: &mut Vec<u8>) -> Result<(), Reject> {
    // 900^15 < 10^45; digits least significant first.
    let mut digits = [0u8; 46];
    let mut len = 0usize;
    for &c in group {
        let mut carry = u32::from(c);
        for d in &mut digits[..len] {
            let t = u32::from(*d) * 900 + carry;
            *d = (t % 10) as u8;
            carry = t / 10;
        }
        while carry > 0 {
            digits[len] = (carry % 10) as u8;
            carry /= 10;
            len += 1;
        }
    }
    if len < 2 || digits[len - 1] != 1 {
        return Err(Reject::Format);
    }
    out.extend(digits[..len - 1].iter().rev().map(|d| b'0' + d));
    Ok(())
}

fn flush_numeric(run: &[u16], out: &mut Vec<u8>) -> Result<(), Reject> {
    run.chunks(NUMERIC_GROUP)
        .try_for_each(|g| numeric_group(g, out))
}

/// Digits of a numeric field of a control block.
fn numeric_value(run: &[u16]) -> Result<u64, Reject> {
    let mut digits = Vec::new();
    flush_numeric(run, &mut digits)?;
    std::str::from_utf8(&digits)
        .ok()
        .and_then(|s| s.parse().ok())
        .ok_or(Reject::Format)
}

fn text_value(run: &[u16]) -> Result<String, Reject> {
    let mut text = Text::new();
    let mut bytes = Vec::new();
    for &c in run {
        text.codeword(c, &mut bytes)?;
    }
    String::from_utf8(bytes).map_err(|_| Reject::Format)
}

/// Codewords from `i` up to the next one that is not a plain data codeword.
fn plain_run(cws: &[u16], i: usize) -> &[u16] {
    let len = cws[i..].iter().take_while(|&&c| c < 900).count();
    &cws[i..i + len]
}

/// The Macro PDF417 control block after the 928 at `cws[start - 1]`. Returns
/// the block and the index after it.
fn macro_block(cws: &[u16], start: usize) -> Result<(MacroPdf417, usize), Reject> {
    let mut i = start;
    // Segment index: five digits, numeric compaction without a latch.
    let index = match cws.get(i..i + 2) {
        Some(&[a, b]) if a < 900 && b < 900 => {
            let mut digits = Vec::new();
            numeric_group(&[a, b], &mut digits)?;
            std::str::from_utf8(&digits)
                .ok()
                .filter(|s| s.len() == 5)
                .and_then(|s| s.parse::<u32>().ok())
                .filter(|&v| v <= 99_998)
                .ok_or(Reject::Format)?
        }
        _ => return Err(Reject::Format),
    };
    i += 2;
    let id = plain_run(cws, i);
    if id.is_empty() {
        return Err(Reject::Format);
    }
    let mut block = MacroPdf417::new(index, id.iter().map(|c| format!("{c:03}")).collect());
    i += id.len();
    let mut seen = [false; 7];
    while cws.get(i) == Some(&MACRO_FIELD) {
        let field = usize::from(*cws.get(i + 1).ok_or(Reject::Format)?);
        if field >= seen.len() || std::mem::replace(&mut seen[field], true) {
            return Err(Reject::Format);
        }
        let run = plain_run(cws, i + 2);
        if run.is_empty() {
            return Err(Reject::Format);
        }
        match field {
            0 => block.file_name = Some(text_value(run)?),
            1 => {
                block.segment_count =
                    Some(u32::try_from(numeric_value(run)?).map_err(|_| Reject::Format)?)
            }
            2 => block.timestamp = Some(numeric_value(run)?),
            3 => block.sender = Some(text_value(run)?),
            4 => block.addressee = Some(text_value(run)?),
            5 => block.file_size = Some(numeric_value(run)?),
            _ => {
                block.checksum =
                    Some(u32::try_from(numeric_value(run)?).map_err(|_| Reject::Format)?)
            }
        }
        i += 2 + run.len();
    }
    if cws.get(i) == Some(&MACRO_TERMINATOR) {
        block.last_segment = true;
        i += 1;
    }
    Ok((block, i))
}

/// ECI designator after the ECI codeword `cws[i]`: (value, codewords used
/// including the escape).
fn eci(cws: &[u16], i: usize) -> Result<(u32, usize), Reject> {
    let arg = |k: usize| {
        cws.get(i + k)
            .copied()
            .filter(|&c| c < 900)
            .map(u32::from)
            .ok_or(Reject::Format)
    };
    match cws[i] {
        ECI_ONE_CODEWORD => Ok((arg(1)?, 2)),
        ECI_TWO_CODEWORDS => Ok((900 + arg(1)? * 900 + arg(2)?, 3)),
        _ => Ok((810_900 + arg(1)?, 2)),
    }
}

/// Decodes the data codewords between the length descriptor and the check
/// codewords, padding included.
pub(super) fn parse(cws: &[u16]) -> Result<Parsed, Reject> {
    let mut data = Vec::new();
    // (offset in `data`, charset) at each charset change.
    let mut charsets = vec![(0usize, Charset::Latin1)];
    let mut eci_used = false;
    let mut mode = Mode::Text;
    let mut text = Text::new();
    let mut macro_pdf417 = None;
    let mut reader_init = false;
    let mut i = 0;
    while i < cws.len() {
        let cw = cws[i];
        if let Mode::Byte { whole_groups } = mode
            && (cw < 900
                || matches!(
                    cw,
                    ECI_ONE_CODEWORD_HIGH | ECI_TWO_CODEWORDS | ECI_ONE_CODEWORD
                ))
        {
            let (run, ecis, next) = byte_run(cws, i)?;
            let base = data.len();
            flush_bytes(&run, whole_groups, &mut data)?;
            let groups = (run.len() - single_bytes(run.len(), whole_groups)?) / 5;
            for (before, value) in ecis {
                // A designator may not split a group of five codewords.
                let at = if before <= 5 * groups {
                    if before % 5 != 0 {
                        return Err(Reject::Format);
                    }
                    6 * (before / 5)
                } else {
                    6 * groups + before - 5 * groups
                };
                eci_used = true;
                charsets.push((base + at, Charset::Eci(value)));
            }
            i = next;
            continue;
        }
        if cw < 900 {
            let run = plain_run(cws, i);
            match mode {
                Mode::Text => run.iter().try_for_each(|&c| text.codeword(c, &mut data))?,
                Mode::Byte { whole_groups } => flush_bytes(run, whole_groups, &mut data)?,
                Mode::Numeric => flush_numeric(run, &mut data)?,
            }
            i += run.len();
            continue;
        }
        // Anything but a plain codeword ends the current text run.
        text.end_run();
        i += 1;
        match cw {
            TEXT_LATCH => {
                mode = Mode::Text;
                text.reset();
            }
            BYTE_LATCH => {
                mode = Mode::Byte {
                    whole_groups: false,
                }
            }
            BYTE_LATCH_MULTIPLE_OF_6 => mode = Mode::Byte { whole_groups: true },
            NUMERIC_LATCH => mode = Mode::Numeric,
            BYTE_SHIFT => {
                if mode != Mode::Text {
                    return Err(Reject::Format);
                }
                let b = cws.get(i).copied().and_then(|c| u8::try_from(c).ok());
                data.push(b.ok_or(Reject::Format)?);
                i += 1;
            }
            READER_INIT => {
                if i != 1 {
                    return Err(Reject::Format);
                }
                reader_init = true;
            }
            ECI_ONE_CODEWORD_HIGH | ECI_TWO_CODEWORDS | ECI_ONE_CODEWORD => {
                let (value, used) = eci(cws, i - 1)?;
                i += used - 1;
                eci_used = true;
                charsets.push((data.len(), Charset::Eci(value)));
            }
            MACRO_BLOCK => {
                if macro_pdf417.is_some() {
                    return Err(Reject::Format);
                }
                let (block, next) = macro_block(cws, i)?;
                macro_pdf417 = Some(block);
                i = next;
                // Compaction restarts in the default mode after the block.
                mode = Mode::Text;
                text.reset();
            }
            // Reserved codewords, and the macro terminator and field
            // marker outside a control block.
            _ => return Err(Reject::Format),
        }
    }
    if data.is_empty() {
        return Err(Reject::Format);
    }
    let mut segments: Vec<DataSegment> = Vec::new();
    for (k, &(start, charset)) in charsets.iter().enumerate() {
        let end = charsets.get(k + 1).map_or(data.len(), |next| next.0);
        if start == end {
            continue;
        }
        match segments.last_mut() {
            Some(last) if last.charset == charset => last.end = end,
            _ => segments.push(DataSegment {
                start,
                end,
                charset,
            }),
        }
    }
    Ok(Parsed {
        data,
        segments,
        macro_pdf417,
        reader_init,
        eci_used,
    })
}
