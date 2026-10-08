//! Data codewords to payload: the ASCII, C40, Text, X12, EDIFACT and Base256
//! encodation schemes with their latch, shift and special codewords.
//!
//! Decisions where ISO/IEC 16022 leaves room:
//! - Reader Programming (234) is rejected: its payload configures a reader
//!   and is never application data.
//! - Structured Append, macros 05/06 and the first-position FNC1 are only
//!   accepted before any data character.
//! - Padding must follow the standard sequence exactly (129, then the
//!   253-state randomized pads) up to the end of the data codewords.
//! - C40, Text and X12 may end without unlatch; a single remaining codeword
//!   is then read as ASCII. A shift value ending the last triple is accepted as padding
//!   (the standard pads with Shift 1, some encoders use Shift 2).
//! - EDIFACT leaves with the unlatch value (padding bits must be zero),
//!   implicitly to ASCII when one or two codewords follow a complete group
//!   of four characters (an unlatch value wins when both readings fit), or at the end when fewer than six zero bits remain.
//! - A symbol without any data character is rejected.

use breader_core::{Charset, DataSegment, Reject};

const BAD: Reject = Reject::Format;

pub(super) struct Parsed {
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub eci: Vec<u32>,
    pub structured_append: Option<(u8, u8, u16)>,
    pub aim_id: [u8; 3],
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ascii,
    C40,
    Text,
    X12,
    Edifact,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fnc1 {
    None,
    First,
    Second,
}

/// Marks `data[start..end]` as `charset`, merging with the previous range when equal.
fn mark(segments: &mut Vec<DataSegment>, start: usize, end: usize, charset: Charset) {
    if start >= end {
        return;
    }
    match segments.last_mut() {
        Some(last) if last.charset == charset && last.end == start => last.end = end,
        _ => segments.push(DataSegment {
            start,
            end,
            charset,
        }),
    }
}

/// Randomized pad codeword at 1-based position `pos` (253-state algorithm).
pub(super) fn pad_value(pos: usize) -> u8 {
    let v = 129 + (149 * pos) % 253 + 1;
    (if v > 254 { v - 254 } else { v }) as u8
}

/// Base256 randomization value at 1-based codeword position `pos` (255-state algorithm).
fn base256_mask(pos: usize) -> u8 {
    (((149 * pos) % 255) + 1) as u8
}

struct State<'a> {
    cw: &'a [u8],
    pos: usize,
    mode: Mode,
    data: Vec<u8>,
    segments: Vec<DataSegment>,
    seg_start: usize,
    charset: Charset,
    eci: Vec<u32>,
    fnc1: Fnc1,
    sa: Option<(u8, u8, u16)>,
    macro_trailer: bool,
    /// Symbol characters produced so far.
    chars: usize,
    /// The first character was a letter or a digit pair (application indicator).
    first_is_ai: bool,
    // C40/Text shift state.
    shift: u8,
    upper: bool,
}

impl State<'_> {
    fn emit(&mut self, c: u8) {
        let c = if self.upper {
            self.upper = false;
            c | 0x80
        } else {
            c
        };
        self.data.push(c);
        self.chars += 1;
    }

    fn next(&mut self) -> Result<u8, Reject> {
        let c = *self.cw.get(self.pos).ok_or(BAD)?;
        self.pos += 1;
        Ok(c)
    }

    fn set_charset(&mut self, charset: Charset) {
        let end = self.data.len();
        mark(&mut self.segments, self.seg_start, end, self.charset);
        self.seg_start = end;
        self.charset = charset;
    }

    fn no_data_yet(&self) -> bool {
        self.chars == 0 && self.data.is_empty()
    }

    fn ascii(&mut self) -> Result<(), Reject> {
        let at = self.pos;
        let c = self.next()?;
        match c {
            1..=128 => {
                let ch = c - 1;
                if self.chars == 0 {
                    self.first_is_ai = ch.is_ascii_alphabetic();
                }
                self.emit(ch);
            }
            129 => {
                for (j, &p) in self.cw.iter().enumerate().skip(at + 1) {
                    if p != pad_value(j + 1) {
                        return Err(BAD);
                    }
                }
                self.pos = self.cw.len();
            }
            130..=229 => {
                let v = c - 130;
                if self.chars == 0 {
                    self.first_is_ai = true;
                }
                self.emit(b'0' + v / 10);
                self.chars -= 1;
                self.emit(b'0' + v % 10);
            }
            230 => self.mode = Mode::C40,
            231 => self.base256()?,
            232 => {
                if self.fnc1 == Fnc1::None && self.no_data_yet() && !self.macro_trailer {
                    self.fnc1 = Fnc1::First;
                } else if self.fnc1 == Fnc1::None
                    && self.chars == 1
                    && self.first_is_ai
                    && !self.macro_trailer
                {
                    self.fnc1 = Fnc1::Second;
                } else {
                    self.emit(0x1d);
                }
            }
            233 => {
                if at != 0 {
                    return Err(BAD);
                }
                let ind = self.next()?;
                let (f1, f2) = (self.next()?, self.next()?);
                let (position, total) = ((ind >> 4) + 1, 17 - (ind & 15));
                if ind & 15 == 0 || position > total || !(1..=254).contains(&f1) {
                    return Err(BAD);
                }
                if !(1..=254).contains(&f2) {
                    return Err(BAD);
                }
                self.sa = Some((position, total, u16::from(f1) << 8 | u16::from(f2)));
            }
            234 => return Err(BAD),
            235 => {
                let n = self.next()?;
                if !(1..=128).contains(&n) {
                    return Err(BAD);
                }
                self.upper = true;
                if self.chars == 0 {
                    self.first_is_ai = false;
                }
                self.emit(n - 1);
            }
            236 | 237 => {
                if !self.no_data_yet() || self.fnc1 != Fnc1::None || self.macro_trailer {
                    return Err(BAD);
                }
                let header: &[u8] = if c == 236 {
                    b"[)>\x1e05\x1d"
                } else {
                    b"[)>\x1e06\x1d"
                };
                let keep = self.charset;
                self.set_charset(Charset::Ascii);
                self.data.extend_from_slice(header);
                self.set_charset(keep);
                self.macro_trailer = true;
            }
            238 => self.mode = Mode::X12,
            239 => self.mode = Mode::Text,
            240 => self.mode = Mode::Edifact,
            241 => {
                let n = self.eci_value()?;
                self.eci.push(n);
                self.set_charset(Charset::Eci(n));
            }
            _ => return Err(BAD),
        }
        Ok(())
    }

    fn eci_value(&mut self) -> Result<u32, Reject> {
        let c1 = u32::from(self.next()?);
        let value = match c1 {
            1..=127 => c1 - 1,
            128..=191 => {
                let c2 = u32::from(self.next()?);
                if !(1..=254).contains(&c2) {
                    return Err(BAD);
                }
                (c1 - 128) * 254 + c2 - 1 + 127
            }
            192..=207 => {
                let (c2, c3) = (u32::from(self.next()?), u32::from(self.next()?));
                if !(1..=254).contains(&c2) || !(1..=254).contains(&c3) {
                    return Err(BAD);
                }
                (c1 - 192) * 64516 + (c2 - 1) * 254 + (c3 - 1) + 16383
            }
            _ => return Err(BAD),
        };
        if value > 999_999 {
            return Err(BAD);
        }
        Ok(value)
    }

    fn base256(&mut self) -> Result<(), Reject> {
        fn read(s: &mut State<'_>) -> Result<usize, Reject> {
            let pos = s.pos + 1;
            let c = s.next()?;
            Ok(usize::from(c.wrapping_sub(base256_mask(pos))))
        }
        let d1 = read(self)?;
        let len = match d1 {
            0 => self.cw.len() - self.pos,
            1..=249 => d1,
            _ => {
                let d2 = read(self)?;
                (d1 - 249) * 250 + d2
            }
        };
        if len == 0 || self.pos + len > self.cw.len() {
            return Err(BAD);
        }
        for _ in 0..len {
            let b = read(self)? as u8;
            self.emit(b);
        }
        Ok(())
    }

    fn triple(&mut self) -> Result<(), Reject> {
        let remaining = self.cw.len() - self.pos;
        if self.cw[self.pos] == 254 {
            self.pos += 1;
            return self.leave_triple();
        }
        if remaining == 1 {
            self.leave_triple()?;
            return Ok(());
        }
        let v = usize::from(self.cw[self.pos]) * 256 + usize::from(self.cw[self.pos + 1]);
        self.pos += 2;
        if v == 0 || v > 64000 {
            return Err(BAD);
        }
        let v = v - 1;
        for value in [v / 1600, (v % 1600) / 40, v % 40] {
            match self.mode {
                Mode::X12 => self.x12(value as u8)?,
                _ => self.c40(value as u8)?,
            }
        }
        Ok(())
    }

    fn leave_triple(&mut self) -> Result<(), Reject> {
        if self.upper {
            return Err(BAD);
        }
        self.shift = 0;
        self.mode = Mode::Ascii;
        Ok(())
    }

    fn x12(&mut self, v: u8) -> Result<(), Reject> {
        let c = match v {
            0 => 13,
            1 => b'*',
            2 => b'>',
            3 => b' ',
            4..=13 => b'0' + v - 4,
            14..=39 => b'A' + v - 14,
            _ => return Err(BAD),
        };
        self.emit(c);
        Ok(())
    }

    fn c40(&mut self, v: u8) -> Result<(), Reject> {
        let text = self.mode == Mode::Text;
        let shift = std::mem::take(&mut self.shift);
        match shift {
            0 => match v {
                0..=2 => self.shift = v + 1,
                3 => self.emit(b' '),
                4..=13 => self.emit(b'0' + v - 4),
                14..=39 => self.emit(if text { b'a' } else { b'A' } + v - 14),
                _ => return Err(BAD),
            },
            1 => self.emit(v),
            2 => match v {
                0..=14 => self.emit(33 + v),
                15..=21 => self.emit(58 + v - 15),
                22..=26 => self.emit(91 + v - 22),
                27 => self.emit(0x1d),
                30 => {
                    if self.upper {
                        return Err(BAD);
                    }
                    self.upper = true;
                }
                _ => return Err(BAD),
            },
            _ => match v {
                0 => self.emit(96),
                1..=26 => self.emit(if text { 64 + v } else { 96 + v }),
                27..=31 => self.emit(123 + v - 27),
                _ => return Err(BAD),
            },
        }
        Ok(())
    }

    fn edifact(&mut self) -> Result<(), Reject> {
        let total = self.cw.len() * 8;
        let start = self.pos * 8;
        let mut bit = start;
        let bits_at = |from: usize, n: usize| -> u32 {
            let mut v = 0;
            for k in from..from + n {
                v = v << 1 | u32::from(self.cw[k / 8] >> (7 - k % 8) & 1);
            }
            v
        };
        let mut produced = Vec::new();
        let mut left = false;
        while bit + 6 <= total {
            // After a complete group of four characters the last one or two
            // codewords are ASCII: no room is left for an unlatch.
            let v = bits_at(bit, 6);
            if v != 31
                && (bit - start).is_multiple_of(24)
                && bit > start
                && (8..=16).contains(&(total - bit))
            {
                left = true;
                break;
            }
            bit += 6;
            if v == 31 {
                let pad = (8 - bit % 8) % 8;
                if bits_at(bit, pad) != 0 {
                    return Err(BAD);
                }
                bit += pad;
                left = true;
                break;
            }
            produced.push(if v < 32 { v as u8 + 64 } else { v as u8 });
        }
        if !left {
            // Fewer than six bits remain: they must be zero.
            if bits_at(bit, total - bit) != 0 {
                return Err(BAD);
            }
            bit = total;
        }
        for c in produced {
            self.emit(c);
        }
        self.pos = bit / 8;
        self.mode = Mode::Ascii;
        Ok(())
    }
}

pub(super) fn parse(cw: &[u8]) -> Result<Parsed, Reject> {
    let mut st = State {
        cw,
        pos: 0,
        mode: Mode::Ascii,
        data: Vec::with_capacity(cw.len() * 2),
        segments: Vec::new(),
        seg_start: 0,
        charset: Charset::Latin1,
        eci: Vec::new(),
        fnc1: Fnc1::None,
        sa: None,
        macro_trailer: false,
        chars: 0,
        first_is_ai: false,
        shift: 0,
        upper: false,
    };
    // Every iteration consumes at least one codeword, except the hand-over
    // from a triple mode to ASCII, which switches the mode.
    for _ in 0..cw.len() * 2 + 2 {
        if st.pos >= cw.len() {
            break;
        }
        match st.mode {
            Mode::Ascii => st.ascii()?,
            Mode::C40 | Mode::Text | Mode::X12 => st.triple()?,
            Mode::Edifact => st.edifact()?,
        }
    }
    if st.pos < cw.len() {
        return Err(BAD);
    }
    if st.upper {
        return Err(BAD);
    }
    if st.data.is_empty() {
        return Err(BAD);
    }
    let end = st.data.len();
    mark(&mut st.segments, st.seg_start, end, st.charset);
    if st.macro_trailer {
        st.data.extend_from_slice(b"\x1e\x04");
        let end = st.data.len();
        mark(&mut st.segments, end - 2, end, Charset::Ascii);
    }
    let kind = match st.fnc1 {
        Fnc1::None => 1,
        Fnc1::First => 2,
        Fnc1::Second => 3,
    } + if st.eci.is_empty() { 0 } else { 3 };
    Ok(Parsed {
        data: st.data,
        segments: st.segments,
        eci: st.eci,
        structured_append: st.sa,
        aim_id: [b']', b'd', b'0' + kind],
    })
}
