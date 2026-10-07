//! GS1 element strings: Application Identifier table, parser and HRI.
//!
//! The table is written from the GS1 General Specifications. Entries that
//! should be re-checked against the current edition are marked `REVIEW`.

use breader_core::Gs1Element;
use std::fmt;

const GS: u8 = 0x1d;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Gs1Error {
    Empty,
    UnknownAi,
    /// The value is shorter than the minimum or longer than the maximum.
    BadLength,
    /// A character outside the format of the AI (digit or GS1 character set).
    BadCharacter,
    EmptyValue,
    CheckDigit,
    BadDate,
    /// A separator with no element after it.
    TrailingSeparator,
}

impl fmt::Display for Gs1Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "empty element string",
            Self::UnknownAi => "unknown application identifier",
            Self::BadLength => "value length out of range",
            Self::BadCharacter => "character not allowed for this application identifier",
            Self::EmptyValue => "application identifier without a value",
            Self::CheckDigit => "check digit mismatch",
            Self::BadDate => "invalid date",
            Self::TrailingSeparator => "separator at the end of the element string",
        })
    }
}

impl std::error::Error for Gs1Error {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Check {
    None,
    /// The last digit of the whole value checks the preceding ones.
    Full,
    /// The digit at this length (1-based) checks the preceding ones; more data may follow.
    Prefix(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Date {
    None,
    /// YYMMDD.
    Ymd,
    /// YYMMDDHHMM.
    YmdHm,
    /// YYMMDD or YYMMDDYYMMDD.
    YmdRange,
    /// YYMMDD optionally followed by HHMM.
    YmdOptTime,
}

#[derive(Debug, Clone, Copy)]
struct Spec {
    ai: &'static str,
    min: u8,
    max: u8,
    /// Leading characters that must be digits; the rest use the GS1 character set.
    digits: u8,
    check: Check,
    date: Date,
}

const ALL: u8 = u8::MAX;

const fn n(ai: &'static str, min: u8, max: u8) -> Spec {
    Spec {
        ai,
        min,
        max,
        digits: ALL,
        check: Check::None,
        date: Date::None,
    }
}

const fn x(ai: &'static str, max: u8) -> Spec {
    Spec {
        ai,
        min: 1,
        max,
        digits: 0,
        check: Check::None,
        date: Date::None,
    }
}

/// `digits` numeric characters followed by alphanumeric ones.
const fn nx(ai: &'static str, digits: u8, min: u8, max: u8) -> Spec {
    Spec {
        ai,
        min,
        max,
        digits,
        check: Check::None,
        date: Date::None,
    }
}

impl Spec {
    const fn chk(mut self, check: Check) -> Self {
        self.check = check;
        self
    }

    const fn date(mut self, date: Date) -> Self {
        self.date = date;
        self
    }
}

/// `n` in an AI stands for any digit (decimal point position).
const AIS: &[Spec] = &[
    n("00", 18, 18).chk(Check::Full),
    n("01", 14, 14).chk(Check::Full),
    n("02", 14, 14).chk(Check::Full),
    x("10", 20),
    n("11", 6, 6).date(Date::Ymd),
    n("12", 6, 6).date(Date::Ymd),
    n("13", 6, 6).date(Date::Ymd),
    n("15", 6, 6).date(Date::Ymd),
    n("16", 6, 6).date(Date::Ymd),
    n("17", 6, 6).date(Date::Ymd),
    n("20", 2, 2),
    x("21", 20),
    x("22", 20),
    x("235", 28),
    x("240", 30),
    x("241", 30),
    n("242", 1, 6),
    x("243", 20),
    x("250", 30),
    x("251", 30),
    nx("253", 13, 13, 30).chk(Check::Prefix(13)),
    x("254", 20),
    n("255", 13, 25).chk(Check::Prefix(13)),
    n("30", 1, 8),
    // Measures, decimal point position in the last digit of the AI.
    n("310n", 6, 6),
    n("311n", 6, 6),
    n("312n", 6, 6),
    n("313n", 6, 6),
    n("314n", 6, 6),
    n("315n", 6, 6),
    n("316n", 6, 6),
    n("320n", 6, 6),
    n("321n", 6, 6),
    n("322n", 6, 6),
    n("323n", 6, 6),
    n("324n", 6, 6),
    n("325n", 6, 6),
    n("326n", 6, 6),
    n("327n", 6, 6),
    n("328n", 6, 6),
    n("329n", 6, 6),
    n("330n", 6, 6),
    n("331n", 6, 6),
    n("332n", 6, 6),
    n("333n", 6, 6),
    n("334n", 6, 6),
    n("335n", 6, 6),
    n("336n", 6, 6),
    n("337n", 6, 6),
    n("340n", 6, 6),
    n("341n", 6, 6),
    n("342n", 6, 6),
    n("343n", 6, 6),
    n("344n", 6, 6),
    n("345n", 6, 6),
    n("346n", 6, 6),
    n("347n", 6, 6),
    n("348n", 6, 6),
    n("349n", 6, 6),
    n("350n", 6, 6),
    n("351n", 6, 6),
    n("352n", 6, 6),
    n("353n", 6, 6),
    n("354n", 6, 6),
    n("355n", 6, 6),
    n("356n", 6, 6),
    n("357n", 6, 6),
    n("360n", 6, 6),
    n("361n", 6, 6),
    n("362n", 6, 6),
    n("363n", 6, 6),
    n("364n", 6, 6),
    n("365n", 6, 6),
    n("366n", 6, 6),
    n("367n", 6, 6),
    n("368n", 6, 6),
    n("369n", 6, 6), // REVIEW: existence of 369n
    n("37", 1, 8),
    n("390n", 1, 15),
    n("391n", 4, 18),
    n("392n", 1, 15),
    n("393n", 4, 18),
    n("394n", 4, 4),
    n("395n", 6, 6),
    x("400", 30),
    x("401", 30),
    n("402", 17, 17).chk(Check::Full),
    x("403", 30),
    n("410", 13, 13).chk(Check::Full),
    n("411", 13, 13).chk(Check::Full),
    n("412", 13, 13).chk(Check::Full),
    n("413", 13, 13).chk(Check::Full),
    n("414", 13, 13).chk(Check::Full),
    n("415", 13, 13).chk(Check::Full),
    n("416", 13, 13).chk(Check::Full),
    n("417", 13, 13).chk(Check::Full),
    x("420", 20),
    nx("421", 3, 4, 12),
    n("422", 3, 3),
    n("423", 3, 15),
    n("424", 3, 3),
    n("425", 3, 15),
    n("426", 3, 3),
    x("427", 3),
    n("7001", 13, 13),
    x("7002", 30),
    n("7003", 10, 10).date(Date::YmdHm),
    n("7004", 1, 4),
    x("7005", 12),
    n("7006", 6, 6).date(Date::Ymd),
    n("7007", 6, 12).date(Date::YmdRange),
    x("7008", 3),
    x("7009", 10),
    x("7010", 2),
    n("7011", 6, 10).date(Date::YmdOptTime), // REVIEW: AI 7011 format
    x("7020", 20),
    x("7021", 20),
    x("7022", 20),
    x("7023", 30),
    nx("7030", 3, 4, 30),
    nx("7031", 3, 4, 30),
    nx("7032", 3, 4, 30),
    nx("7033", 3, 4, 30),
    nx("7034", 3, 4, 30),
    nx("7035", 3, 4, 30),
    nx("7036", 3, 4, 30),
    nx("7037", 3, 4, 30),
    nx("7038", 3, 4, 30),
    nx("7039", 3, 4, 30),
    n("8001", 14, 14),
    x("8002", 20),
    nx("8003", 14, 14, 30).chk(Check::Prefix(14)),
    x("8004", 30),
    n("8005", 6, 6),
    n("8006", 18, 18).chk(Check::Prefix(14)),
    x("8007", 34),
    n("8008", 8, 12), // REVIEW: date/time not validated
    x("8009", 50),
    x("8010", 30),
    n("8011", 1, 12),
    x("8012", 20),
    x("8013", 25), // REVIEW: check characters not validated
    n("8017", 18, 18).chk(Check::Full),
    n("8018", 18, 18).chk(Check::Full),
    n("8019", 1, 10),
    x("8020", 25),
    n("8026", 18, 18).chk(Check::Prefix(14)), // REVIEW
    x("8110", 70),
    n("8111", 4, 4),
    x("8112", 70),
    x("8200", 70),
    x("90", 30),
    x("91", 90),
    x("92", 90),
    x("93", 90),
    x("94", 90),
    x("95", 90),
    x("96", 90),
    x("97", 90),
    x("98", 90),
    x("99", 90),
];

/// Two-digit AI prefixes whose elements have a predefined fixed length and
/// therefore need no separator after them.
const PREDEFINED: &[&str] = &[
    "00", "01", "02", "03", "04", "11", "12", "13", "14", "15", "16", "17", "18", "19", "20", "31",
    "32", "33", "34", "35", "36", "41",
];

fn matches_ai(pattern: &str, ai: &[u8]) -> bool {
    pattern.len() == ai.len()
        && pattern
            .bytes()
            .zip(ai)
            .all(|(p, &a)| p == a || (p == b'n' && a.is_ascii_digit()))
}

fn lookup(rest: &[u8]) -> Option<&'static Spec> {
    (2..=4).find_map(|len| {
        let ai = rest.get(..len)?;
        if !ai.iter().all(u8::is_ascii_digit) {
            return None;
        }
        AIS.iter().find(|s| matches_ai(s.ai, ai))
    })
}

fn is_predefined(ai: &[u8]) -> bool {
    PREDEFINED.iter().any(|p| ai.starts_with(p.as_bytes()))
}

/// GS1 AI encodable character set 82.
fn is_gs1_char(b: u8) -> bool {
    matches!(b, b'!' | b'"' | b'%'..=b'?' | b'A'..=b'Z' | b'_' | b'a'..=b'z')
}

fn check_digit_ok(digits: &[u8]) -> bool {
    let Some((&last, body)) = digits.split_last() else {
        return false;
    };
    let sum: u32 = body
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| u32::from(d - b'0') * if i % 2 == 0 { 3 } else { 1 })
        .sum();
    u32::from(last - b'0') == (10 - sum % 10) % 10
}

fn two(d: &[u8]) -> u32 {
    u32::from(d[0] - b'0') * 10 + u32::from(d[1] - b'0')
}

fn valid_ymd(d: &[u8]) -> bool {
    let (year, month, day) = (two(d), two(&d[2..]), two(&d[4..]));
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        // The century is unknown, so February 29 is accepted in any year
        // that is a multiple of 4.
        2 if year % 4 == 0 => 29,
        2 => 28,
        _ => return false,
    };
    day <= days
}

fn valid_hm(d: &[u8]) -> bool {
    two(d) <= 23 && two(&d[2..]) <= 59
}

fn valid_date(kind: Date, v: &[u8]) -> bool {
    match kind {
        Date::None => true,
        Date::Ymd => valid_ymd(v),
        Date::YmdHm => valid_ymd(v) && valid_hm(&v[6..]),
        Date::YmdRange => match v.len() {
            6 => valid_ymd(v),
            12 => valid_ymd(v) && valid_ymd(&v[6..]),
            _ => false,
        },
        Date::YmdOptTime => match v.len() {
            6 => valid_ymd(v),
            10 => valid_ymd(v) && valid_hm(&v[6..]),
            _ => false,
        },
    }
}

fn validate(spec: &Spec, value: &[u8]) -> Result<(), Gs1Error> {
    if value.is_empty() {
        return Err(Gs1Error::EmptyValue);
    }
    if value.len() < usize::from(spec.min) || value.len() > usize::from(spec.max) {
        return Err(Gs1Error::BadLength);
    }
    let digits = usize::from(spec.digits).min(value.len());
    let (head, tail) = value.split_at(digits);
    if !head.iter().all(u8::is_ascii_digit) || !tail.iter().all(|&b| is_gs1_char(b)) {
        return Err(Gs1Error::BadCharacter);
    }
    let checked = match spec.check {
        Check::None => None,
        Check::Full => Some(value),
        Check::Prefix(n) => value.get(..usize::from(n)),
    };
    if let Some(digits) = checked
        && !check_digit_ok(digits)
    {
        return Err(Gs1Error::CheckDigit);
    }
    if !valid_date(spec.date, value) {
        return Err(Gs1Error::BadDate);
    }
    Ok(())
}

/// Parses an element string. Variable-length elements are terminated by GS
/// (0x1D) or the end of the data; elements of predefined fixed length may be
/// followed directly by the next AI. Any invalid or unknown part rejects the
/// whole string; a separator with nothing after it is rejected as well.
pub fn parse(data: &str) -> Result<Vec<Gs1Element>, Gs1Error> {
    let bytes = data.as_bytes();
    if bytes.is_empty() {
        return Err(Gs1Error::Empty);
    }
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let rest = &bytes[pos..];
        let spec = lookup(rest).ok_or(Gs1Error::UnknownAi)?;
        let start = pos + spec.ai.len();
        let ai = &bytes[pos..start];
        let end = if is_predefined(ai) {
            let end = start + usize::from(spec.max);
            if end > bytes.len() {
                return Err(Gs1Error::BadLength);
            }
            end
        } else {
            bytes[start..]
                .iter()
                .position(|&b| b == GS)
                .map_or(bytes.len(), |p| start + p)
        };
        let value = &bytes[start..end];
        validate(spec, value)?;
        out.push(Gs1Element {
            ai: data[pos..start].to_owned(),
            value: data[start..end].to_owned(),
        });
        pos = end;
        if bytes.get(pos) == Some(&GS) {
            pos += 1;
            if pos == bytes.len() {
                return Err(Gs1Error::TrailingSeparator);
            }
        }
    }
    Ok(out)
}

/// Human-readable form: `(01)09506000134352(10)ABC`.
pub fn hri(elements: &[Gs1Element]) -> String {
    let mut s = String::new();
    for e in elements {
        s.push('(');
        s.push_str(&e.ai);
        s.push(')');
        s.push_str(&e.value);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn el(ai: &str, value: &str) -> Gs1Element {
        Gs1Element {
            ai: ai.into(),
            value: value.into(),
        }
    }

    #[test]
    fn table_is_well_formed() {
        for (i, s) in AIS.iter().enumerate() {
            assert!((2..=4).contains(&s.ai.len()), "{}", s.ai);
            assert!(s.min >= 1 && s.min <= s.max, "{}", s.ai);
            assert!(
                s.ai.bytes().all(|b| b.is_ascii_digit() || b == b'n'),
                "{}",
                s.ai
            );
            for t in &AIS[i + 1..] {
                // No two patterns may match the same AI.
                let overlap = s.ai.len() == t.ai.len()
                    && s.ai
                        .bytes()
                        .zip(t.ai.bytes())
                        .all(|(a, b)| a == b || a == b'n' || b == b'n');
                assert!(!overlap, "{} / {}", s.ai, t.ai);
            }
            if is_predefined(s.ai.as_bytes()) {
                assert_eq!(s.min, s.max, "predefined AI must be fixed: {}", s.ai);
            }
            if let Check::Prefix(n) = s.check {
                assert!(n <= s.max, "{}", s.ai);
            }
        }
    }

    #[test]
    fn valid_strings() {
        assert_eq!(
            parse("0109506000134352").unwrap(),
            [el("01", "09506000134352")]
        );
        // Predefined fixed lengths chain without separators.
        assert_eq!(
            parse("010950600013435217250131").unwrap(),
            [el("01", "09506000134352"), el("17", "250131")]
        );
        // Variable element in the middle needs GS; the last one does not.
        assert_eq!(
            parse("0109506000134352\x1d10AB12\x1d21XYZ").unwrap(),
            [
                el("01", "09506000134352"),
                el("10", "AB12"),
                el("21", "XYZ")
            ]
        );
        assert_eq!(
            parse("1725013110LOT1\x1d3103000123").unwrap(),
            [el("17", "250131"), el("10", "LOT1"), el("3103", "000123")]
        );
        assert_eq!(parse("00376104250021234569").unwrap().len(), 1);
        // Date with day 00 is allowed.
        assert_eq!(parse("17250100").unwrap(), [el("17", "250100")]);
        // Mixed numeric/alphanumeric and 4-digit AIs.
        assert_eq!(
            parse("421276AB-12\x1d7006250131").unwrap(),
            [el("421", "276AB-12"), el("7006", "250131")]
        );
        assert_eq!(
            parse("8003012345678901281234").unwrap(),
            [el("8003", "012345678901281234")]
        );
        // Ranges of AIs with decimal positions and 99.
        assert_eq!(parse("3922999\x1d99free-text").unwrap().len(), 2);
        assert!(parse("37100").is_ok());
        assert!(parse("7007250101250131").is_ok());
    }

    #[test]
    fn rejects_bad_strings() {
        assert_eq!(parse(""), Err(Gs1Error::Empty));
        // Wrong GTIN check digit.
        assert_eq!(parse("0109506000134353"), Err(Gs1Error::CheckDigit));
        assert_eq!(parse("00376104250021234560"), Err(Gs1Error::CheckDigit));
        // Month 13, day 32.
        assert_eq!(parse("17251301"), Err(Gs1Error::BadDate));
        assert_eq!(parse("17250132"), Err(Gs1Error::BadDate));
        assert_eq!(parse("17250431"), Err(Gs1Error::BadDate));
        assert_eq!(parse("17250001"), Err(Gs1Error::BadDate));
        assert_eq!(parse("70032501312561"), Err(Gs1Error::BadDate));
        assert_eq!(parse("70032501312359").unwrap().len(), 1);
        // Unknown AI.
        assert_eq!(parse("9"), Err(Gs1Error::UnknownAi));
        assert_eq!(parse("0412345"), Err(Gs1Error::UnknownAi));
        assert_eq!(parse("1810"), Err(Gs1Error::UnknownAi));
        assert_eq!(parse("ab12"), Err(Gs1Error::UnknownAi));
        assert_eq!(parse("\x1d10A"), Err(Gs1Error::UnknownAi));
        // Trailing separator and empty values.
        assert_eq!(parse("10ABC\x1d"), Err(Gs1Error::TrailingSeparator));
        assert_eq!(parse("10"), Err(Gs1Error::EmptyValue));
        assert_eq!(parse("10\x1d21A"), Err(Gs1Error::EmptyValue));
        // Lengths and characters.
        assert_eq!(parse("01095060001343"), Err(Gs1Error::BadLength));
        assert_eq!(parse("10123456789012345678901"), Err(Gs1Error::BadLength));
        assert_eq!(parse("10AB#C"), Err(Gs1Error::BadCharacter));
        assert_eq!(parse("10AB\u{e9}"), Err(Gs1Error::BadCharacter));
        assert_eq!(parse("3012A"), Err(Gs1Error::BadCharacter));
        // A fixed non-predefined AI cannot run into the next element.
        assert_eq!(parse("422276").unwrap().len(), 1);
        assert_eq!(parse("42227610A"), Err(Gs1Error::BadLength));
    }

    #[test]
    fn hri_format() {
        assert_eq!(hri(&[]), "");
        assert_eq!(
            hri(&[el("01", "09506000134352"), el("10", "AB")]),
            "(01)09506000134352(10)AB"
        );
        assert_eq!(
            hri(&parse("0109506000134352\x1d10AB").unwrap()),
            "(01)09506000134352(10)AB"
        );
    }

    #[test]
    fn random_input_never_panics() {
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x
        };
        let alphabet = b"0123456789\x1dAB ()%#";
        for _ in 0..20000 {
            let len = (next() % 40) as usize;
            let s: String = (0..len)
                .map(|_| {
                    if next() % 8 == 0 {
                        char::from((next() % 128) as u8)
                    } else {
                        char::from(alphabet[(next() % alphabet.len() as u64) as usize])
                    }
                })
                .collect();
            let _ = parse(&s);
        }
        for _ in 0..2000 {
            let len = (next() % 40) as usize;
            let s: String = (0..len)
                .map(|_| char::from_u32((next() % 0x800) as u32).unwrap_or('x'))
                .collect();
            let _ = parse(&s);
        }
    }
}
