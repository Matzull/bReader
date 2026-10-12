//! Bit stream of a GS1 DataBar Expanded symbol (ISO/IEC 24724, encodation
//! methods) to a GS1 element string.
//!
//! The data characters of the symbol, 12 bits each, form the stream. Bit 0
//! is the linkage flag; the following bits select the encodation method:
//!
//! * `1`: `(01)` with a 13-digit GTIN body, then general purpose data;
//! * `00`: general purpose data;
//! * `0100`, `0101`: `(01)` with a GTIN starting with 9 and a weight
//!   `(3103)`, `(3202)` or `(3203)`;
//! * `01100`, `01101`: `(01)` with a GTIN starting with 9, `(392n)` or
//!   `(393n)` with a currency code, then general purpose data;
//! * `0111000` to `0111111`: `(01)` with a GTIN starting with 9, a weight
//!   `(310n)` or `(320n)` and a date `(11)`, `(13)`, `(15)` or `(17)`.
//!
//! General purpose data switch between numeric, alphanumeric and ISO/IEC
//! 646 encodation; FNC1 becomes the group separator GS.

const GS: u8 = 0x1d;

/// Bits of the stream, one per byte.
pub(super) struct Bits<'a>(pub &'a [u8]);

impl Bits<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn get(&self, at: usize, n: usize) -> u32 {
        self.0[at..at + n]
            .iter()
            .fold(0, |v, &b| v << 1 | u32::from(b))
    }
}

/// Element string of a symbol of `n` characters (check character included)
/// whose data characters make up `bits`. `None` for anything the encodation
/// rules do not allow, including a symbol linked to a 2D component.
pub(super) fn element_string(bits: &Bits<'_>, n: usize) -> Option<Vec<u8>> {
    if bits.len() != 12 * (n - 1) || bits.get(0, 1) != 0 {
        return None;
    }
    // Number of characters odd, and more than 14.
    let size_field = u32::from(n % 2 == 1) << 1 | u32::from(n > 14);
    let mut out = Vec::with_capacity(bits.len() / 3 + 24);
    let general_from;
    if bits.get(1, 1) == 1 {
        if bits.get(2, 2) != size_field {
            return None;
        }
        out.extend_from_slice(b"01");
        let first = bits.get(4, 4);
        if first > 9 {
            return None;
        }
        let at = out.len();
        out.push(b'0' + first as u8);
        push_gtin_digits(bits, 8, &mut out);
        push_check_digit(&mut out, at);
        general_from = 48;
    } else if bits.get(2, 1) == 0 {
        if bits.get(3, 2) != size_field {
            return None;
        }
        general_from = 5;
    } else if matches!(bits.get(1, 4), 0b0100 | 0b0101) {
        if n != 6 {
            return None;
        }
        push_gtin9(bits, 5, &mut out);
        let weight = bits.get(45, 15);
        if bits.get(1, 4) == 0b0100 {
            out.extend_from_slice(b"3103");
            push_number(&mut out, weight, 6);
        } else {
            if weight < 10_000 {
                out.extend_from_slice(b"3202");
                push_number(&mut out, weight, 6);
            } else {
                out.extend_from_slice(b"3203");
                push_number(&mut out, weight - 10_000, 6);
            }
        }
        return Some(out);
    } else if matches!(bits.get(1, 5), 0b01100 | 0b01101) {
        if bits.get(6, 2) != size_field {
            return None;
        }
        push_gtin9(bits, 8, &mut out);
        let x = bits.get(48, 2);
        if bits.get(1, 5) == 0b01100 {
            out.extend_from_slice(b"392");
            push_number(&mut out, x, 1);
            general_from = 50;
        } else {
            out.extend_from_slice(b"393");
            push_number(&mut out, x, 1);
            let currency = bits.get(50, 10);
            if currency > 999 {
                return None;
            }
            push_number(&mut out, currency, 3);
            general_from = 60;
        }
    } else if (0b0111000..=0b0111111).contains(&bits.get(1, 7)) {
        if n != 8 {
            return None;
        }
        let method = bits.get(1, 7);
        push_gtin9(bits, 8, &mut out);
        let weight = bits.get(48, 20);
        out.extend_from_slice(if method & 1 == 0 { b"310" } else { b"320" });
        push_number(&mut out, weight / 100_000, 1);
        push_number(&mut out, weight % 100_000, 6);
        let date = bits.get(68, 16);
        // 38400 stands for no date at all.
        if date != 38_400 {
            out.extend_from_slice(match (method >> 1) & 3 {
                0 => b"11",
                1 => b"13",
                2 => b"15",
                _ => b"17",
            });
            let (yy, mm, dd) = (date / 384, date % 384 / 32 + 1, date % 32);
            if mm > 12 {
                return None;
            }
            push_number(&mut out, yy, 2);
            push_number(&mut out, mm, 2);
            push_number(&mut out, dd, 2);
        }
        return Some(out);
    } else {
        return None;
    }
    general(bits, general_from, Mode::Numeric, &mut 64, &mut out)?;
    // An odd digit at the end shares its pair with FNC1.
    while out.last() == Some(&GS) {
        out.pop();
    }
    Some(out)
}

fn push_number(out: &mut Vec<u8>, mut value: u32, digits: usize) {
    let at = out.len();
    out.resize(at + digits, b'0');
    for d in out[at..].iter_mut().rev() {
        *d = b'0' + (value % 10) as u8;
        value /= 10;
    }
}

/// The 12 digits that follow the first one, in groups of three.
fn push_gtin_digits(bits: &Bits<'_>, at: usize, out: &mut Vec<u8>) {
    for group in 0..4 {
        push_number(out, bits.get(at + 10 * group, 10).min(999), 3);
    }
}

/// GTIN with the first digit 9 and its check digit.
fn push_gtin9(bits: &Bits<'_>, at: usize, out: &mut Vec<u8>) {
    out.extend_from_slice(b"019");
    let body = out.len() - 1;
    push_gtin_digits(bits, at, out);
    push_check_digit(out, body);
}

/// Appends the GS1 check digit of the 13 digits starting at `from`.
fn push_check_digit(out: &mut Vec<u8>, from: usize) {
    let sum: u32 = out[from..from + 13]
        .iter()
        .enumerate()
        .map(|(i, &d)| u32::from(d - b'0') * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    out.push(b'0' + ((10 - sum % 10) % 10) as u8);
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Numeric,
    Alpha,
    Iso646,
}

/// General purpose data from bit `pos` to the end of the stream.
///
/// What follows an FNC1 met in alphanumeric or ISO/IEC 646 encodation is not
/// settled by the symbols seen: some encoders resume in numeric mode, others
/// stay and latch to numeric explicitly (`000`). Both are tried, the one
/// the next bits suggest first, within a budget of `tries` branches.
fn general(
    bits: &Bits<'_>,
    mut pos: usize,
    mut mode: Mode,
    tries: &mut u32,
    out: &mut Vec<u8>,
) -> Option<()> {
    loop {
        let left = bits.len() - pos;
        if left == 0 {
            return Some(());
        }
        match mode {
            Mode::Numeric => {
                if left < 7 {
                    if left >= 4 && bits.get(pos, 4) != 0 {
                        let v = bits.get(pos, 4);
                        if v > 10 {
                            return None;
                        }
                        out.push(b'0' + (v - 1) as u8);
                        pos += 4;
                    }
                    return padding(bits, pos);
                }
                if bits.get(pos, 4) == 0 {
                    mode = Mode::Alpha;
                    pos += 4;
                    continue;
                }
                let v = bits.get(pos, 7) - 8;
                pos += 7;
                for x in [v / 11, v % 11] {
                    out.push(if x == 10 { GS } else { b'0' + x as u8 });
                }
            }
            Mode::Alpha | Mode::Iso646 => {
                if left >= 3 && bits.get(pos, 3) == 0 {
                    mode = Mode::Numeric;
                    pos += 3;
                    continue;
                }
                if left < 5 {
                    return padding(bits, pos);
                }
                let v5 = bits.get(pos, 5);
                if v5 == 4 {
                    mode = if mode == Mode::Alpha {
                        Mode::Iso646
                    } else {
                        Mode::Alpha
                    };
                    pos += 5;
                } else if (5..15).contains(&v5) {
                    out.push(b'0' + (v5 - 5) as u8);
                    pos += 5;
                } else if v5 == 15 {
                    pos += 5;
                    let explicit = pos + 3 <= bits.len() && bits.get(pos, 3) == 0;
                    let order = if explicit {
                        [mode, Mode::Numeric]
                    } else {
                        [Mode::Numeric, mode]
                    };
                    let keep = out.len();
                    for next in order {
                        if *tries == 0 {
                            return None;
                        }
                        *tries -= 1;
                        out.push(GS);
                        if general(bits, pos, next, tries, out).is_some() {
                            return Some(());
                        }
                        out.truncate(keep);
                    }
                    return None;
                } else if mode == Mode::Alpha {
                    if left < 6 {
                        return padding(bits, pos);
                    }
                    let v6 = bits.get(pos, 6);
                    out.push(match v6 {
                        32..=57 => b'A' + (v6 - 32) as u8,
                        58 => b'*',
                        59 => b',',
                        60 => b'-',
                        61 => b'.',
                        62 => b'/',
                        _ => return None,
                    });
                    pos += 6;
                } else {
                    if left < 7 {
                        return padding(bits, pos);
                    }
                    let v7 = bits.get(pos, 7);
                    match v7 {
                        64..=89 => {
                            out.push(b'A' + (v7 - 64) as u8);
                            pos += 7;
                        }
                        90..=115 => {
                            out.push(b'a' + (v7 - 90) as u8);
                            pos += 7;
                        }
                        _ => {
                            if left < 8 {
                                return padding(bits, pos);
                            }
                            out.push(match bits.get(pos, 8) {
                                232 => b'!',
                                233 => b'"',
                                234 => b'%',
                                235 => b'&',
                                236 => b'\'',
                                237 => b'(',
                                238 => b')',
                                239 => b'*',
                                240 => b'+',
                                241 => b',',
                                242 => b'-',
                                243 => b'.',
                                244 => b'/',
                                245 => b':',
                                246 => b';',
                                247 => b'<',
                                248 => b'=',
                                249 => b'>',
                                250 => b'?',
                                251 => b'_',
                                252 => b' ',
                                _ => return None,
                            });
                            pos += 8;
                        }
                    }
                }
            }
        }
    }
}

/// The bits that remain after the last character are zeros or the start of
/// the pad pattern `00100` repeated.
fn padding(bits: &Bits<'_>, from: usize) -> Option<()> {
    let rest = &bits.0[from..];
    let zeros = rest.iter().all(|&b| b == 0);
    let pad = rest
        .iter()
        .enumerate()
        .all(|(i, &b)| b == u8::from(i % 5 == 2));
    (zeros || pad).then_some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits_of(s: &str) -> Vec<u8> {
        s.bytes().map(|b| b - b'0').collect()
    }

    fn decode(s: &str, n: usize) -> Option<String> {
        element_string(&Bits(&bits_of(s)), n).map(|v| String::from_utf8(v).unwrap())
    }

    #[test]
    fn mode_switches() {
        let s = "000000010011001010101011011000101000010000000110001110100001001100000000001100011101000010101010101000000010";
        assert_eq!(decode(s, 10).as_deref(), Some("10123456A1234A15991231"));
    }

    #[test]
    fn weight_method() {
        let s = "001000000000001001110101010001101111101111010000011011010110";
        assert_eq!(decode(s, 6).as_deref(), Some("01900123456789083103001750"));
    }
}
