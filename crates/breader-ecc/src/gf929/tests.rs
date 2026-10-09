use super::*;

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn make_block(rng: &mut Rng, ec: usize, max_len: usize) -> Vec<u16> {
    let total = ec + 1 + rng.below(max_len - ec);
    let mut block: Vec<u16> = (0..total).map(|_| rng.below(929) as u16).collect();
    let (data, check) = block.split_at_mut(total - ec);
    PDF417_FIELD.encode(data, ec, check).unwrap();
    block
}

fn pick_distinct(rng: &mut Rng, n: usize, k: usize) -> Vec<usize> {
    let mut v = Vec::new();
    while v.len() < k {
        let p = rng.below(n);
        if !v.contains(&p) {
            v.push(p);
        }
    }
    v
}

/// Replaces the codeword with a different valid value.
fn corrupt(rng: &mut Rng, block: &mut [u16], p: usize) {
    block[p] = (block[p] + 1 + rng.below(928) as u16) % 929;
}

fn is_codeword(block: &[u16], ec: usize) -> bool {
    let mut s = [0u16; L];
    PDF417_FIELD.syndromes(block, ec, &mut s)
}

#[test]
fn field_tables() {
    let f = &PDF417_FIELD;
    let mut seen = [false; 929];
    for i in 0..ORDER {
        let v = usize::from(f.exp[i]);
        assert!(v != 0 && !seen[v], "exp is a permutation of 1..=928");
        seen[v] = true;
        assert_eq!(usize::from(f.log[v]), i);
    }
    for a in 1..929u16 {
        assert_eq!(mul(a, f.inv(a)), 1);
    }
    assert_eq!(f.alpha(ORDER), 1);
}

/// Check codewords for a fixed data section at levels 0 to 3. Vectors from the
/// test suite of the MIT-licensed `pdf417` encoder crate (Copyright (c) 2023
/// Sofiane); the whole-symbol cross-check against a second encoder is in
/// `breader-pdf417`.
#[test]
fn known_check_codewords() {
    let data = [
        16u16, 902, 1, 278, 827, 900, 295, 902, 2, 326, 823, 544, 900, 149, 900, 900,
    ];
    let cases: [&[u16]; 4] = [
        &[156, 765],
        &[168, 875, 63, 355],
        &[628, 715, 393, 299, 863, 601, 169, 708],
        &[
            232, 176, 793, 616, 476, 406, 855, 445, 84, 518, 522, 721, 607, 2, 42, 578,
        ],
    ];
    for (level, expected) in cases.iter().enumerate() {
        let ec = 2usize << level;
        assert_eq!(expected.len(), ec);
        let mut out = vec![0u16; ec];
        PDF417_FIELD.encode(&data, ec, &mut out).unwrap();
        assert_eq!(&out, expected, "level {level}");
        let mut block = [data.as_slice(), expected].concat();
        assert_eq!(
            PDF417_FIELD.decode(&mut block, ec, &[]),
            Ok(Correction::default())
        );
    }
    // g(x) = (x - 3)(x - 9) = x^2 + 917x + 27, so 1 * x^2 has check codewords 917 and 27.
    let mut out = [0u16; 2];
    PDF417_FIELD.encode(&[1], 2, &mut out).unwrap();
    assert_eq!(out, [917, 27]);
}

#[test]
fn roundtrip_within_capacity() {
    let mut rng = Rng(0x9e3779b97f4a7c15);
    for case in 0..3000 {
        let level = rng.below(9);
        let ec = 2usize << level;
        // Keep the larger levels rare: they are slow.
        if level > 6 && case % 8 != 0 {
            continue;
        }
        let clean = make_block(&mut rng, ec, 928.min(ec + 200));
        let n = clean.len();
        let e = rng.below(ec / 2 + 1);
        let f = rng.below(ec - 2 * e + 1).min(n - e);
        let positions = pick_distinct(&mut rng, n, e + f);
        let (err_pos, era_pos) = positions.split_at(e);
        let mut block = clean.clone();
        for &p in err_pos {
            corrupt(&mut rng, &mut block, p);
        }
        let mut changed_erasures = 0;
        for &p in era_pos {
            // An erased codeword may still hold its correct value.
            if rng.below(4) != 0 {
                corrupt(&mut rng, &mut block, p);
                changed_erasures += 1;
            }
        }
        let got = PDF417_FIELD.decode(&mut block, ec, era_pos);
        assert_eq!(
            got,
            Ok(Correction {
                errors: e,
                erasures: changed_erasures
            }),
            "ec={ec} n={n} e={e} f={f}"
        );
        assert_eq!(block, clean);
    }
}

#[test]
fn exact_capacity_all_levels() {
    let mut rng = Rng(42);
    for level in 0..=8usize {
        let ec = 2usize << level;
        let clean = make_block(&mut rng, ec, 928.min(ec + 100));
        let n = clean.len();
        // Maximum errors, maximum erasures, and a mix on the boundary.
        for (e, f) in [(ec / 2, 0), (0, ec.min(n)), (ec / 4, ec - 2 * (ec / 4))] {
            let positions = pick_distinct(&mut rng, n, e + f);
            let (err_pos, era_pos) = positions.split_at(e);
            let mut block = clean.clone();
            for &p in &positions {
                corrupt(&mut rng, &mut block, p);
            }
            let got = PDF417_FIELD.decode(&mut block, ec, era_pos).unwrap();
            assert_eq!(got.errors, err_pos.len());
            assert_eq!(got.erasures, era_pos.len());
            assert_eq!(block, clean, "level {level} e={e} f={f}");
        }
    }
}

#[test]
fn full_length_symbol() {
    let mut rng = Rng(7);
    let mut block: Vec<u16> = (0..928 - 512).map(|_| rng.below(929) as u16).collect();
    let mut check = vec![0u16; 512];
    PDF417_FIELD.encode(&block, 512, &mut check).unwrap();
    block.extend(check);
    assert_eq!(block.len(), 928);
    let clean = block.clone();
    let pos = pick_distinct(&mut rng, 928, 256);
    for &p in &pos {
        corrupt(&mut rng, &mut block, p);
    }
    assert_eq!(
        PDF417_FIELD.decode(&mut block, 512, &[]),
        Ok(Correction {
            errors: 256,
            erasures: 0
        })
    );
    assert_eq!(block, clean);
}

#[test]
fn beyond_capacity_never_silently_wrong() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let (mut accepted, mut total) = (0, 0);
    for _ in 0..8000 {
        let level = rng.below(6);
        let ec = 2usize << level;
        let clean = make_block(&mut rng, ec, 928.min(ec + 60));
        let n = clean.len();
        let e = rng.below(ec / 2 + 2);
        let f = rng.below(ec + 1).min(n.saturating_sub(e));
        if 2 * e + f <= ec {
            continue;
        }
        total += 1;
        let positions = pick_distinct(&mut rng, n, (e + f).min(n));
        let (err_pos, era_pos) = positions.split_at(e.min(positions.len()));
        let mut block = clean.clone();
        for &p in err_pos {
            corrupt(&mut rng, &mut block, p);
        }
        for &p in era_pos {
            corrupt(&mut rng, &mut block, p);
        }
        let corrupted = block.clone();
        match PDF417_FIELD.decode(&mut block, ec, era_pos) {
            Ok(c) => {
                accepted += 1;
                // Whatever was accepted is a codeword and the report is exact.
                assert!(is_codeword(&block, ec));
                assert_ne!(block, corrupted);
                assert!(2 * c.errors + era_pos.len() <= ec);
                let diff = (0..n).filter(|&i| block[i] != corrupted[i]).count();
                assert_eq!(diff, c.errors + c.erasures);
            }
            Err(e) => {
                assert_eq!(e, RsError::Uncorrectable);
                assert_eq!(block, corrupted, "block must be untouched on failure");
            }
        }
    }
    // Large levels fail almost always; small ones sometimes land on another
    // codeword, which is why the PDF417 decoder adds its own margin.
    assert!(total > 1000 && accepted < total / 2, "{accepted}/{total}");
}

#[test]
fn erasure_with_intact_value_is_not_counted() {
    let mut rng = Rng(5);
    let clean = make_block(&mut rng, 8, 40);
    let mut block = clean.clone();
    corrupt(&mut rng, &mut block, 3);
    let got = PDF417_FIELD.decode(&mut block, 8, &[3, 9, 10]).unwrap();
    assert_eq!(
        got,
        Correction {
            errors: 0,
            erasures: 1
        }
    );
    assert_eq!(block, clean);
}

#[test]
fn invalid_inputs() {
    let f = &PDF417_FIELD;
    let mut b = [1u16, 2, 3, 4];
    assert_eq!(f.decode(&mut b, 5, &[]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[4]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[1, 1]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[0, 1, 2]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 0, &[]), Ok(Correction::default()));
    assert_eq!(f.decode(&mut b, 0, &[0]), Err(RsError::InvalidInput));
    let mut out_of_field = [1u16, 929, 3, 4];
    assert_eq!(
        f.decode(&mut out_of_field, 2, &[]),
        Err(RsError::InvalidInput)
    );
    let mut big = vec![0u16; 929];
    assert_eq!(f.decode(&mut big, 4, &[]), Err(RsError::InvalidInput));
    let mut huge_ec = vec![0u16; 600];
    assert_eq!(f.decode(&mut huge_ec, 513, &[]), Err(RsError::InvalidInput));
    let mut out = [0u16; 3];
    assert_eq!(f.encode(&[1, 2], 2, &mut out), Err(RsError::InvalidInput));
    assert_eq!(
        f.encode(&[1000], 2, &mut [0; 2]),
        Err(RsError::InvalidInput)
    );
    assert_eq!(
        f.encode(&[0; 920], 16, &mut [0; 16]),
        Err(RsError::InvalidInput)
    );
    assert_eq!(f.encode(&[1], 0, &mut []), Ok(()));
}

#[test]
fn zero_data_has_zero_check() {
    let mut out = [1u16; 8];
    PDF417_FIELD.encode(&[0; 20], 8, &mut out).unwrap();
    assert_eq!(out, [0; 8]);
}
