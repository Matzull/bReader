//! High-level decoding (ISO/IEC 24778 section 7.3): the stuffed-free bit
//! stream to bytes through the Upper, Lower, Mixed, Punct and Digit modes,
//! Binary Shift, FNC1 and ECI.

use breader_core::{Charset, DataSegment, Reject};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Upper,
    Lower,
    Mixed,
    Punct,
    Digit,
}

#[derive(Debug, Clone, Copy)]
enum Tok {
    Byte(u8),
    Text(&'static [u8]),
    Latch(Mode),
    Shift(Mode),
    Binary,
    Flg,
}

const PUNCT: &[u8; 25] = b"!\"#$%&'()*+,-./:;<=>?[]{}";

fn lookup(mode: Mode, code: u32) -> Tok {
    let c = code as u8;
    match mode {
        Mode::Upper => match c {
            0 => Tok::Shift(Mode::Punct),
            1 => Tok::Byte(b' '),
            2..=27 => Tok::Byte(b'A' + c - 2),
            28 => Tok::Latch(Mode::Lower),
            29 => Tok::Latch(Mode::Mixed),
            30 => Tok::Latch(Mode::Digit),
            _ => Tok::Binary,
        },
        Mode::Lower => match c {
            0 => Tok::Shift(Mode::Punct),
            1 => Tok::Byte(b' '),
            2..=27 => Tok::Byte(b'a' + c - 2),
            28 => Tok::Shift(Mode::Upper),
            29 => Tok::Latch(Mode::Mixed),
            30 => Tok::Latch(Mode::Digit),
            _ => Tok::Binary,
        },
        Mode::Mixed => match c {
            0 => Tok::Shift(Mode::Punct),
            1 => Tok::Byte(b' '),
            2..=14 => Tok::Byte(c - 1),
            15..=19 => Tok::Byte(27 + c - 15),
            20 => Tok::Byte(b'@'),
            21 => Tok::Byte(b'\\'),
            22 => Tok::Byte(b'^'),
            23 => Tok::Byte(b'_'),
            24 => Tok::Byte(b'`'),
            25 => Tok::Byte(b'|'),
            26 => Tok::Byte(b'~'),
            27 => Tok::Byte(0x7f),
            28 => Tok::Latch(Mode::Lower),
            29 => Tok::Latch(Mode::Upper),
            30 => Tok::Latch(Mode::Punct),
            _ => Tok::Binary,
        },
        Mode::Punct => match c {
            0 => Tok::Flg,
            1 => Tok::Byte(b'\r'),
            2 => Tok::Text(b"\r\n"),
            3 => Tok::Text(b". "),
            4 => Tok::Text(b", "),
            5 => Tok::Text(b": "),
            6..=30 => Tok::Byte(PUNCT[usize::from(c) - 6]),
            _ => Tok::Latch(Mode::Upper),
        },
        Mode::Digit => match c {
            0 => Tok::Shift(Mode::Punct),
            1 => Tok::Byte(b' '),
            2..=11 => Tok::Byte(b'0' + c - 2),
            12 => Tok::Byte(b','),
            13 => Tok::Byte(b'.'),
            14 => Tok::Latch(Mode::Upper),
            _ => Tok::Shift(Mode::Upper),
        },
    }
}

/// Where the first FNC1 of the message stood.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Fnc1 {
    None,
    /// First position: the message is GS1 element strings.
    First,
    /// Second position, after an application indicator (one letter or two
    /// digits): AIM Application Identifier.
    Second,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Parsed {
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub eci: Vec<u32>,
    pub fnc1: Fnc1,
}

struct Bits<'a> {
    bits: &'a [bool],
    pos: usize,
}

impl Bits<'_> {
    fn left(&self) -> usize {
        self.bits.len() - self.pos
    }

    /// Reads `n <= 32` bits, most significant first.
    fn read(&mut self, n: usize) -> Option<u32> {
        if self.left() < n {
            return None;
        }
        let v = self.bits[self.pos..self.pos + n]
            .iter()
            .fold(0u32, |acc, &b| acc << 1 | u32::from(b));
        self.pos += n;
        Some(v)
    }
}

struct State {
    data: Vec<u8>,
    segments: Vec<DataSegment>,
    seg_start: usize,
    charset: Charset,
    eci: Vec<u32>,
    fnc1: Fnc1,
}

impl State {
    fn close_segment(&mut self) {
        let end = self.data.len();
        if end > self.seg_start {
            self.segments.push(DataSegment {
                start: self.seg_start,
                end,
                charset: self.charset,
            });
            self.seg_start = end;
        }
    }

    fn fnc1(&mut self) {
        let d = &self.data;
        let first = d.is_empty() && self.fnc1 == Fnc1::None;
        let second = self.fnc1 == Fnc1::None
            && match d.len() {
                1 => d[0].is_ascii_alphabetic(),
                2 => d.iter().all(u8::is_ascii_digit),
                _ => false,
            };
        if first {
            self.fnc1 = Fnc1::First;
        } else if second {
            self.fnc1 = Fnc1::Second;
        } else {
            self.data.push(0x1d);
        }
    }

    /// Handles the 3-bit count and the digits after a FLG code.
    fn flg(&mut self, r: &mut Bits) -> Result<(), Reject> {
        match r.read(3).ok_or(Reject::Format)? {
            0 => self.fnc1(),
            n @ 1..=6 => {
                let mut value = 0u32;
                for _ in 0..n {
                    let digit = r.read(4).ok_or(Reject::Format)?;
                    if !(2..=11).contains(&digit) {
                        return Err(Reject::Format);
                    }
                    value = value * 10 + digit - 2;
                }
                self.close_segment();
                self.charset = Charset::Eci(value);
                self.eci.push(value);
            }
            _ => return Err(Reject::Format),
        }
        Ok(())
    }

    /// One token read after a shift: a character, FLG or Binary Shift, nothing else.
    fn shifted(&mut self, r: &mut Bits, to: Mode) -> Result<Option<Mode>, Reject> {
        let code = r.read(5).ok_or(Reject::Format)?;
        match lookup(to, code) {
            Tok::Byte(b) => self.data.push(b),
            Tok::Text(t) => self.data.extend_from_slice(t),
            Tok::Flg => self.flg(r)?,
            // Digit mode reaches Binary Shift through U/S; the data that
            // follows the bytes is in the mode the shift went to.
            Tok::Binary => {
                self.binary(r)?;
                return Ok(Some(to));
            }
            _ => return Err(Reject::Format),
        }
        Ok(None)
    }

    fn binary(&mut self, r: &mut Bits) -> Result<(), Reject> {
        let mut len = r.read(5).ok_or(Reject::Format)? as usize;
        if len == 0 {
            len = r.read(11).ok_or(Reject::Format)? as usize + 31;
        }
        if r.left() < len * 8 {
            return Err(Reject::Format);
        }
        for _ in 0..len {
            let b = r.read(8).ok_or(Reject::Format)?;
            self.data.push(b as u8);
        }
        Ok(())
    }
}

/// Decodes the unstuffed bit stream. Trailing ones are padding. Fails on an
/// invalid code sequence, a truncated token or an empty message.
pub(crate) fn parse(bits: &[bool]) -> Result<Parsed, Reject> {
    // Everything from here on is padding (the encoder pads with ones).
    let ones_from = bits.iter().rposition(|&b| !b).map_or(0, |i| i + 1);
    let mut r = Bits { bits, pos: 0 };
    let mut st = State {
        data: Vec::new(),
        segments: Vec::new(),
        seg_start: 0,
        charset: Charset::Latin1,
        eci: Vec::new(),
        fnc1: Fnc1::None,
    };
    let mut mode = Mode::Upper;
    while r.pos < ones_from {
        let width = if mode == Mode::Digit { 4 } else { 5 };
        let code = r.read(width).ok_or(Reject::Format)?;
        match lookup(mode, code) {
            Tok::Byte(b) => st.data.push(b),
            Tok::Text(t) => st.data.extend_from_slice(t),
            Tok::Latch(m) => mode = m,
            Tok::Shift(m) => {
                if let Some(back) = st.shifted(&mut r, m)? {
                    mode = back;
                }
            }
            Tok::Binary => st.binary(&mut r)?,
            Tok::Flg => st.flg(&mut r)?,
        }
    }
    if st.data.is_empty() {
        return Err(Reject::Format);
    }
    st.close_segment();
    Ok(Parsed {
        data: st.data,
        segments: st.segments,
        eci: st.eci,
        fnc1: st.fnc1,
    })
}

#[cfg(test)]
mod tests;
