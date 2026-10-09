use super::*;
use crate::build::Bits;

fn run(f: impl Fn(&mut Bits)) -> Result<Parsed, Reject> {
    let mut b = Bits::default();
    f(&mut b);
    parse(&b.0)
}

fn text(f: impl Fn(&mut Bits)) -> Vec<u8> {
    run(f).expect("parses").data
}

// Latches and shifts, by their codes.
const UPPER_LL: u32 = 28;
const UPPER_ML: u32 = 29;
const UPPER_DL: u32 = 30;
const UPPER_BS: u32 = 31;
const PS: u32 = 0;

#[test]
fn upper_table() {
    let d = text(|b| {
        b.push(1, 5);
        for c in 2..=27 {
            b.push(c, 5);
        }
    });
    assert_eq!(d, b" ABCDEFGHIJKLMNOPQRSTUVWXYZ");
}

#[test]
fn lower_table_and_upper_shift() {
    let d = text(|b| {
        b.push(UPPER_LL, 5);
        b.push(1, 5);
        for c in 2..=27 {
            b.push(c, 5);
        }
        b.push(28, 5).push(2, 5); // U/S then A
        b.push(2, 5); // back in lower: a
    });
    assert_eq!(d, b" abcdefghijklmnopqrstuvwxyzAa");
}

#[test]
fn mixed_table() {
    let d = text(|b| {
        b.push(UPPER_ML, 5);
        for c in 1..=27 {
            b.push(c, 5);
        }
    });
    let mut expect = vec![b' '];
    expect.extend(1..=13u8);
    expect.extend([27, 28, 29, 30, 31]);
    expect.extend(b"@\\^_`|~");
    expect.push(127);
    assert_eq!(d, expect);
}

#[test]
fn mixed_latches() {
    let d = text(|b| {
        b.push(UPPER_ML, 5);
        b.push(28, 5).push(2, 5); // L/L: a
        b.push(29, 5); // M/L
        b.push(29, 5).push(2, 5); // U/L: A
        b.push(UPPER_ML, 5);
        b.push(30, 5).push(6, 5); // P/L: !
        b.push(31, 5).push(2, 5); // U/L from punct: A
    });
    assert_eq!(d, b"aA!A");
}

#[test]
fn punct_table() {
    let d = text(|b| {
        for c in 1..=30 {
            b.push(PS, 5).push(c, 5);
        }
    });
    assert_eq!(d, b"\r\r\n. , : !\"#$%&'()*+,-./:;<=>?[]{}");
    // Latched punct ends with U/L.
    let d = text(|b| {
        b.push(UPPER_ML, 5).push(30, 5);
        b.push(6, 5).push(27, 5);
        b.push(31, 5).push(2, 5);
    });
    assert_eq!(d, b"![A");
}

#[test]
fn digit_table_and_shifts() {
    let d = text(|b| {
        b.push(UPPER_DL, 5);
        b.push(1, 4);
        for c in 2..=11 {
            b.push(c, 4);
        }
        b.push(12, 4).push(13, 4);
        b.push(15, 4).push(2, 5); // U/S then A
        b.push(0, 4).push(6, 5); // P/S then !
        b.push(2, 4); // digit 0 again
        b.push(14, 4).push(3, 5); // U/L then B
    });
    assert_eq!(d, b" 0123456789,.A!0B");
}

#[test]
fn binary_shift_after_an_upper_shift_continues_in_upper() {
    let d = text(|b| {
        b.push(UPPER_DL, 5);
        b.push(5, 4).push(5, 4); // 33
        b.push(15, 4).push(UPPER_BS, 5).push(2, 5);
        b.push(0x68, 8).push(0x69, 8);
        b.push(3, 5); // B: the mode after the bytes is Upper
    });
    assert_eq!(d, b"33hiB");
}

#[test]
fn punct_shift_returns_to_the_previous_mode() {
    let d = text(|b| {
        b.push(UPPER_LL, 5);
        b.push(2, 5); // a
        b.push(PS, 5).push(6, 5); // !
        b.push(3, 5); // b (still lower)
    });
    assert_eq!(d, b"a!b");
}

#[test]
fn binary_shift() {
    // Short form.
    let d = text(|b| {
        b.push(2, 5);
        b.push(UPPER_BS, 5).push(3, 5);
        b.push(0xE9, 8).push(0, 8).push(0xFF, 8);
        b.push(3, 5); // back in upper: B
    });
    assert_eq!(d, b"A\xe9\x00\xffB");
    // Extended form: length 31 + value.
    let d = text(|b| {
        b.push(UPPER_BS, 5).push(0, 5).push(0, 11);
        for i in 0..31 {
            b.push(i, 8);
        }
    });
    assert_eq!(d, (0..31u8).collect::<Vec<_>>());
    let d = text(|b| {
        b.push(UPPER_BS, 5).push(0, 5).push(5, 11);
        for i in 0..36 {
            b.push(0x80 + i, 8);
        }
        b.push(2, 5);
    });
    assert_eq!(d.len(), 37);
    assert_eq!(d[36], b'A');
    // From lower and mixed.
    let d = text(|b| {
        b.push(UPPER_LL, 5).push(31, 5).push(1, 5).push(0xA0, 8);
        b.push(2, 5); // lower a
        b.push(UPPER_ML, 5).push(31, 5).push(1, 5).push(0xA1, 8);
        b.push(2, 5); // mixed ^A
    });
    assert_eq!(d, b"\xa0a\xa1\x01");
}

#[test]
fn trailing_ones_are_padding() {
    for pad in 0..12 {
        let d = text(|b| {
            b.push(2, 5).push(3, 5);
            for _ in 0..pad {
                b.push(1, 1);
            }
        });
        assert_eq!(d, b"AB", "pad {pad}");
    }
    // Digit mode ends with ones that look like U/S.
    let d = text(|b| {
        b.push(UPPER_DL, 5)
            .push(2, 4)
            .push(3, 4)
            .push(15, 4)
            .push(31, 5);
    });
    assert_eq!(d, b"01");
}

#[test]
fn invalid_streams_are_rejected() {
    // Empty and padding-only.
    assert!(run(|_| {}).is_err());
    assert!(
        run(|b| {
            b.push(31, 5);
        })
        .is_err()
    );
    // A short non-padding tail.
    assert!(
        run(|b| {
            b.push(2, 5).push(0, 3);
        })
        .is_err()
    );
    // Binary shift longer than the data.
    assert!(
        run(|b| {
            b.push(UPPER_BS, 5).push(5, 5).push(0x41, 8);
        })
        .is_err()
    );
    assert!(
        run(|b| {
            b.push(UPPER_BS, 5).push(0, 5).push(100, 11).push(0x41, 8);
        })
        .is_err()
    );
    // Latches and punct U/L inside a shift.
    assert!(
        run(|b| {
            b.push(PS, 5).push(31, 5).push(2, 5);
        })
        .is_err()
    );
    assert!(
        run(|b| {
            b.push(UPPER_LL, 5).push(28, 5).push(28, 5).push(2, 5);
        })
        .is_err()
    );
    assert!(
        run(|b| {
            b.push(UPPER_DL, 5).push(15, 4).push(30, 5).push(2, 5);
        })
        .is_err()
    );
    // Reserved FLG(7), a non-digit in an ECI, a truncated ECI.
    assert!(
        run(|b| {
            b.push(PS, 5).push(0, 5).push(7, 3).push(2, 5);
        })
        .is_err()
    );
    assert!(
        run(|b| {
            b.push(PS, 5).push(0, 5).push(1, 3).push(12, 4).push(2, 5);
        })
        .is_err()
    );
    assert!(
        run(|b| {
            b.push(PS, 5).push(0, 5).push(3, 3).push(5, 4).push(0, 4);
        })
        .is_err()
    );
    // ECI and FNC1 without any data.
    assert!(
        run(|b| {
            b.push(PS, 5).push(0, 5).push(1, 3).push(5, 4);
        })
        .is_err()
    );
}

#[test]
fn leading_letters_stay_payload() {
    // The standard's structured append header has no flag of its own, so a
    // message that starts like one is returned as written.
    let d = text(|b| {
        b.push(2, 5).push(3, 5).push(1, 5).push(4, 5);
    });
    assert_eq!(d, b"AB C");
}

#[test]
fn charset_segments_follow_eci() {
    let p = run(|b| {
        b.push(2, 5);
        b.push(PS, 5)
            .push(0, 5)
            .push(2, 3)
            .push(3 + 2, 4)
            .push(2, 4); // ECI 30? digits 3,0
        b.push(3, 5);
        b.push(PS, 5).push(0, 5).push(1, 3).push(2 + 1, 4); // ECI 1
        b.push(4, 5);
    })
    .unwrap();
    assert_eq!(p.data, b"ABC");
    assert_eq!(p.eci, vec![30, 1]);
    assert_eq!(
        p.segments,
        vec![
            DataSegment {
                start: 0,
                end: 1,
                charset: Charset::Latin1
            },
            DataSegment {
                start: 1,
                end: 2,
                charset: Charset::Eci(30)
            },
            DataSegment {
                start: 2,
                end: 3,
                charset: Charset::Eci(1)
            },
        ]
    );
}
