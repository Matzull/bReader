//! Bitstream (segment) parsing.

use crate::spec::{ALNUM, count_bits};
use breader_core::{Charset, DataSegment, Reject};

pub(super) struct Parsed {
    pub data: Vec<u8>,
    pub segments: Vec<DataSegment>,
    pub eci: Vec<u32>,
    pub aim_id: [u8; 3],
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn remaining(&self) -> usize {
        self.bytes.len() * 8 - self.pos
    }

    fn take(&mut self, n: usize) -> Result<u32, Reject> {
        if n > 24 || self.remaining() < n {
            return Err(Reject::Length);
        }
        let mut v = 0u32;
        for _ in 0..n {
            let bit = (self.bytes[self.pos / 8] >> (7 - self.pos % 8)) & 1;
            v = (v << 1) | u32::from(bit);
            self.pos += 1;
        }
        Ok(v)
    }
}

const BAD: Reject = Reject::Checksum;

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

pub(super) fn parse(bytes: &[u8], version: u8) -> Result<Parsed, Reject> {
    let mut r = Reader { bytes, pos: 0 };
    let mut data: Vec<u8> = Vec::new();
    let mut eci: Vec<u32> = Vec::new();
    let mut segs: Vec<DataSegment> = Vec::new();
    let mut fnc1_first = false;
    let mut fnc1_second = false;
    let mut segments = 0usize;

    loop {
        if r.remaining() < 4 {
            break;
        }
        let mode = r.take(4)?;
        let seg_start = data.len();
        match mode {
            0 => break,
            1 => {
                let n = r.take(count_bits(version, 0))? as usize;
                let (mut left, mut out) = (n, Vec::with_capacity(n));
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
                    let s = format!("{v:0digits$}");
                    out.extend_from_slice(s.as_bytes());
                    left -= digits;
                }
                data.extend_from_slice(&out);
            }
            2 => {
                let n = r.take(count_bits(version, 1))? as usize;
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
            4 => {
                let n = r.take(count_bits(version, 2))? as usize;
                if r.remaining() < n * 8 {
                    return Err(Reject::Length);
                }
                for _ in 0..n {
                    data.push(r.take(8)? as u8);
                }
            }
            8 => {
                let n = r.take(count_bits(version, 3))? as usize;
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
            7 => {
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
            3 => {
                // Structured Append: sequence, total, parity. Parsed and skipped.
                r.take(16)?;
            }
            5 => {
                if segments != 0 || fnc1_second {
                    return Err(BAD);
                }
                fnc1_first = true;
            }
            9 => {
                if segments != 0 || fnc1_first {
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
        if matches!(mode, 1 | 2 | 4 | 8) {
            segments += 1;
        }
        let charset = match mode {
            4 => match eci.last() {
                Some(&n) => Charset::Eci(n),
                None => Charset::Undeclared,
            },
            8 => Charset::ShiftJis,
            _ => Charset::Ascii,
        };
        mark(&mut segs, seg_start, data.len(), charset);
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
