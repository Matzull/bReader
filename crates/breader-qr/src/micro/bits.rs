//! Micro QR bitstream: segments, terminator and padding.

use super::spec::{count_bits, mode_bits, terminator_bits};
use crate::spec::ALNUM;
use breader_core::{Charset, DataSegment, Reject};

pub(super) struct Parsed {
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
}

const BAD: Reject = Reject::Checksum;

/// Pad codewords alternate between these two values.
const PAD: [u8; 2] = [0xEC, 0x11];

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
    /// bits up to a codeword boundary, then alternating pad codewords, and a
    /// final half codeword (M1, M3) of zeros.
    fn padding_ok(&self) -> bool {
        let mut p = self.pos;
        let boundary = p.next_multiple_of(8).min(self.end);
        if !self.all_zero(p, boundary) {
            return false;
        }
        p = boundary;
        let mut k = 0;
        while p + 8 <= self.end {
            let byte = (p..p + 8).fold(0u8, |a, i| a << 1 | u8::from(self.bit(i)));
            if byte != PAD[k % 2] {
                return false;
            }
            k += 1;
            p += 8;
        }
        self.all_zero(p, self.end)
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

/// Parses the first `total` bits of `bytes` as the data bitstream of a
/// symbol of `version`. Only numeric, alphanumeric, byte and kanji segments
/// exist; any other mode indicator, a zero-length segment, bad padding or
/// trailing data after the terminator rejects the symbol.
pub(super) fn parse(bytes: &[u8], total: usize, version: u8) -> Result<Parsed, Reject> {
    let mut r = Reader {
        bytes,
        pos: 0,
        end: total,
    };
    let mut data: Vec<u8> = Vec::new();
    let mut segs: Vec<DataSegment> = Vec::new();
    let term = terminator_bits(version);

    loop {
        let rem = r.remaining();
        if rem == 0 {
            break;
        }
        let t = term.min(rem);
        if r.all_zero(r.pos, r.pos + t) {
            r.pos += t;
            if !r.padding_ok() {
                return Err(BAD);
            }
            break;
        }
        let mode = if mode_bits(version) == 0 {
            0
        } else {
            r.take(mode_bits(version))? as usize
        };
        if mode > 3 || count_bits(version, mode) == 0 {
            return Err(BAD);
        }
        let n = r.take(count_bits(version, mode))? as usize;
        if n == 0 {
            return Err(BAD);
        }
        let start = data.len();
        let charset = match mode {
            0 => {
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
                Charset::Ascii
            }
            1 => {
                let mut left = n;
                while left > 0 {
                    if left >= 2 {
                        let v = r.take(11)? as usize;
                        if v >= 45 * 45 {
                            return Err(BAD);
                        }
                        data.push(ALNUM[v / 45]);
                        data.push(ALNUM[v % 45]);
                        left -= 2;
                    } else {
                        let v = r.take(6)? as usize;
                        if v >= 45 {
                            return Err(BAD);
                        }
                        data.push(ALNUM[v]);
                        left -= 1;
                    }
                }
                Charset::Ascii
            }
            2 => {
                if r.remaining() < n * 8 {
                    return Err(Reject::Length);
                }
                for _ in 0..n {
                    data.push(r.take(8)? as u8);
                }
                Charset::Undeclared
            }
            _ => {
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
                Charset::ShiftJis
            }
        };
        mark(&mut segs, start, data.len(), charset);
    }
    if data.is_empty() {
        return Err(BAD);
    }
    Ok(Parsed {
        data,
        segments: segs,
    })
}
