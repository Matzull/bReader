//! rMQR bitstream: segments, ECI, FNC1, terminator and padding.

use super::spec::count_bits;
use crate::spec::ALNUM;
use breader_core::{Charset, DataSegment, Reject};

pub(super) struct Parsed {
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub eci: Vec<u32>,
    pub aim_id: [u8; 3],
}

const BAD: Reject = Reject::Checksum;

/// Pad codewords alternate between these two values.
const PAD: [u8; 2] = [0xEC, 0x11];

/// Mode indicators (3 bits).
const END: u32 = 0;
const NUMERIC: u32 = 1;
const ALPHANUMERIC: u32 = 2;
const BYTE: u32 = 3;
const KANJI: u32 = 4;
const FNC1_FIRST: u32 = 5;
const FNC1_SECOND: u32 = 6;
const ECI: u32 = 7;

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    end: usize,
}

impl Reader<'_> {
    fn remaining(&self) -> usize {
        self.end - self.pos
    }

    fn bit(&self, at: usize) -> bool {
        self.bytes[at / 8] >> (7 - at % 8) & 1 == 1
    }

    fn take(&mut self, n: usize) -> Result<u32, Reject> {
        if n > 24 || self.remaining() < n {
            return Err(Reject::Length);
        }
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | u32::from(self.bit(self.pos));
            self.pos += 1;
        }
        Ok(v)
    }

    fn all_zero(&self, from: usize, to: usize) -> bool {
        (from..to).all(|i| !self.bit(i))
    }

    /// Everything after the terminator must be what an encoder writes: zero
    /// bits up to a codeword boundary, then alternating pad codewords.
    fn padding_ok(&self) -> bool {
        let boundary = self.pos.next_multiple_of(8).min(self.end);
        if !self.all_zero(self.pos, boundary) {
            return false;
        }
        let mut p = boundary;
        let mut k = 0;
        while p + 8 <= self.end {
            let byte = (p..p + 8).fold(0u8, |a, i| a << 1 | u8::from(self.bit(i)));
            if byte != PAD[k % 2] {
                return false;
            }
            k += 1;
            p += 8;
        }
        true
    }
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

/// Parses the data codewords of a symbol of version indicator `version`.
/// Structured Append does not exist in rMQR; unknown or reserved modes, empty
/// segments, data after the terminator and any padding other than 0xEC/0x11
/// alternating reject the symbol.
pub(super) fn parse(bytes: &[u8], version: usize) -> Result<Parsed, Reject> {
    let mut r = Reader {
        bytes,
        pos: 0,
        end: bytes.len() * 8,
    };
    let mut data: Vec<u8> = Vec::new();
    let mut eci: Vec<u32> = Vec::new();
    let mut segs: Vec<DataSegment> = Vec::new();
    let mut fnc1_first = false;
    let mut fnc1_second = false;
    let mut segments = 0usize;

    loop {
        let rem = r.remaining();
        if rem == 0 {
            break;
        }
        if rem < 3 {
            if !r.all_zero(r.pos, r.end) {
                return Err(BAD);
            }
            break;
        }
        let mode = r.take(3)?;
        if mode == END {
            if !r.padding_ok() {
                return Err(BAD);
            }
            break;
        }
        let start = data.len();
        match mode {
            NUMERIC => {
                let n = r.take(count_bits(version, 0))? as usize;
                if n == 0 {
                    return Err(BAD);
                }
                let mut left = n;
                while left > 0 {
                    let (digits, bits, max) = match left {
                        1 => (1, 4, 9),
                        2 => (2, 7, 99),
                        _ => (3, 10, 999),
                    };
                    let v = r.take(bits)?;
                    if v > max {
                        return Err(BAD);
                    }
                    data.extend_from_slice(format!("{v:0digits$}").as_bytes());
                    left -= digits;
                }
            }
            ALPHANUMERIC => {
                let n = r.take(count_bits(version, 1))? as usize;
                if n == 0 {
                    return Err(BAD);
                }
                let mut out = Vec::with_capacity(n);
                let mut left = n;
                while left > 0 {
                    if left >= 2 {
                        let v = r.take(11)? as usize;
                        if v >= 45 * 45 {
                            return Err(BAD);
                        }
                        out.push(ALNUM[v / 45]);
                        out.push(ALNUM[v % 45]);
                        left -= 2;
                    } else {
                        let v = r.take(6)? as usize;
                        if v >= 45 {
                            return Err(BAD);
                        }
                        out.push(ALNUM[v]);
                        left -= 1;
                    }
                }
                if fnc1_first || fnc1_second {
                    let mut i = 0;
                    while i < out.len() {
                        if out[i] == b'%' {
                            if out.get(i + 1) == Some(&b'%') {
                                data.push(b'%');
                                i += 2;
                            } else {
                                data.push(0x1D);
                                i += 1;
                            }
                        } else {
                            data.push(out[i]);
                            i += 1;
                        }
                    }
                } else {
                    data.extend_from_slice(&out);
                }
            }
            BYTE => {
                let n = r.take(count_bits(version, 2))? as usize;
                if n == 0 {
                    return Err(BAD);
                }
                if r.remaining() < n * 8 {
                    return Err(Reject::Length);
                }
                for _ in 0..n {
                    data.push(r.take(8)? as u8);
                }
            }
            KANJI => {
                let n = r.take(count_bits(version, 3))? as usize;
                if n == 0 {
                    return Err(BAD);
                }
                for _ in 0..n {
                    let v = r.take(13)?;
                    let w = ((v / 0xC0) << 8) | (v % 0xC0);
                    let w = if w + 0x8140 <= 0x9FFC {
                        w + 0x8140
                    } else {
                        w + 0xC140
                    };
                    data.push((w >> 8) as u8);
                    data.push((w & 0xFF) as u8);
                }
            }
            ECI => {
                let b = r.take(8)?;
                let v = if b & 0x80 == 0 {
                    b
                } else if b & 0xC0 == 0x80 {
                    ((b & 0x3F) << 8) | r.take(8)?
                } else if b & 0xE0 == 0xC0 {
                    ((b & 0x1F) << 16) | r.take(16)?
                } else {
                    return Err(BAD);
                };
                if v > 999_999 {
                    return Err(BAD);
                }
                eci.push(v);
            }
            FNC1_FIRST => {
                if segments != 0 || !eci.is_empty() || fnc1_second || fnc1_first {
                    return Err(BAD);
                }
                fnc1_first = true;
            }
            FNC1_SECOND => {
                if segments != 0 || !eci.is_empty() || fnc1_first || fnc1_second {
                    return Err(BAD);
                }
                let ai = r.take(8)?;
                match ai {
                    0..=99 => data.extend_from_slice(format!("{ai:02}").as_bytes()),
                    165..=190 | 197..=222 => data.push((ai - 100) as u8),
                    _ => return Err(BAD),
                }
                fnc1_second = true;
            }
            _ => return Err(BAD),
        }
        if matches!(mode, NUMERIC | ALPHANUMERIC | BYTE | KANJI) {
            segments += 1;
        }
        let charset = match mode {
            BYTE => match eci.last() {
                Some(&n) => Charset::Eci(n),
                None => Charset::Undeclared,
            },
            KANJI => Charset::ShiftJis,
            _ => Charset::Ascii,
        };
        mark(&mut segs, start, data.len(), charset);
    }
    if segments == 0 {
        return Err(BAD);
    }
    let base = if fnc1_first {
        3
    } else if fnc1_second {
        5
    } else {
        1
    };
    let n = base + u8::from(!eci.is_empty());
    Ok(Parsed {
        data,
        segments: segs,
        eci,
        aim_id: [b']', b'Q', b'0' + n],
    })
}
