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

fn make_block<const Q: usize>(f: &Gf2m<Q>, rng: &mut Rng, ec: usize) -> Vec<u16> {
    let max = (Q - 1).min(ec + 80);
    let total = ec + 1 + rng.below(max - ec);
    let mut block: Vec<u16> = (0..total).map(|_| rng.below(Q) as u16).collect();
    let (data, check) = block.split_at_mut(total - ec);
    f.encode(data, ec, check).unwrap();
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

fn corrupt<const Q: usize>(rng: &mut Rng, block: &mut [u16], p: usize) {
    block[p] ^= 1 + rng.below(Q - 1) as u16;
}

fn is_codeword<const Q: usize>(f: &Gf2m<Q>, block: &[u16], ec: usize) -> bool {
    let mut s = vec![0u16; ec];
    f.syndromes(block, &mut s)
}

fn roundtrip<const Q: usize>(f: &Gf2m<Q>, seed: u64, cases: usize) {
    let mut rng = Rng(seed);
    for _ in 0..cases {
        let ec = 2 + rng.below(40.min(Q - 4));
        let clean = make_block(f, &mut rng, ec);
        let n = clean.len();
        let e = rng.below(ec / 2 + 1);
        let er = rng.below(ec - 2 * e + 1).min(n - e);
        let positions = pick_distinct(&mut rng, n, e + er);
        let (err_pos, era_pos) = positions.split_at(e);
        let mut block = clean.clone();
        for &p in err_pos {
            corrupt::<Q>(&mut rng, &mut block, p);
        }
        let mut changed_erasures = 0;
        for &p in era_pos {
            if rng.below(4) != 0 {
                corrupt::<Q>(&mut rng, &mut block, p);
                changed_erasures += 1;
            }
        }
        let got = f.decode(&mut block, ec, era_pos);
        assert_eq!(
            got,
            Ok(Correction {
                errors: e,
                erasures: changed_erasures
            }),
            "Q={Q} ec={ec} n={n} e={e} erasures={er}"
        );
        assert_eq!(block, clean);
    }
}

fn beyond_capacity<const Q: usize>(f: &Gf2m<Q>, seed: u64, cases: usize) {
    let mut rng = Rng(seed);
    let mut accepted = 0;
    for _ in 0..cases {
        let ec = 2 + rng.below(20.min(Q - 4));
        let clean = make_block(f, &mut rng, ec);
        let n = clean.len();
        let k = (ec / 2 + 1 + rng.below(ec)).min(n);
        let mut block = clean.clone();
        for p in pick_distinct(&mut rng, n, k) {
            corrupt::<Q>(&mut rng, &mut block, p);
        }
        let corrupted = block.clone();
        match f.decode(&mut block, ec, &[]) {
            Ok(c) => {
                accepted += 1;
                assert!(is_codeword(f, &block, ec));
                let diff = (0..n).filter(|&i| block[i] != corrupted[i]).count();
                assert_eq!(diff, c.errors);
                assert!(2 * c.errors <= ec);
            }
            Err(e) => {
                assert_eq!(e, RsError::Uncorrectable);
                assert_eq!(block, corrupted, "block must be untouched on failure");
            }
        }
    }
    assert!(accepted < cases);
}

#[test]
fn field_tables() {
    fn check<const Q: usize>(f: &Gf2m<Q>) {
        let mut seen = vec![false; Q];
        for i in 0..Q - 1 {
            let v = usize::from(f.exp[i]);
            assert!(v != 0 && !seen[v], "exp is a permutation of 1..Q");
            seen[v] = true;
            assert_eq!(usize::from(f.log[v]), i);
        }
        for a in 1..Q as u16 {
            assert_eq!(f.mul(a, f.div(1, a)), 1);
        }
        assert_eq!(f.alpha(Q - 1), 1);
    }
    check(&AZTEC_PARAM);
    check(&AZTEC_DATA_6);
    check(&AZTEC_DATA_8);
    check(&AZTEC_DATA_10);
    check(&AZTEC_DATA_12);
}

#[test]
fn generator_has_first_root_alpha_1() {
    // x^2 + 6x + 8 over GF(16) with x^4 + x + 1: (x - a)(x - a^2) = x^2 + 6x + 8.
    let f = &AZTEC_PARAM;
    let mut out = [0u16; 2];
    f.encode(&[1], 2, &mut out).unwrap();
    // 1 * x^2 mod g = 6x + 8 (char 2: subtraction is addition).
    assert_eq!(out, [6, 8]);
}

#[test]
fn roundtrip_all_fields() {
    roundtrip(&AZTEC_PARAM, 1, 3000);
    roundtrip(&AZTEC_DATA_6, 2, 3000);
    roundtrip(&AZTEC_DATA_8, 3, 3000);
    roundtrip(&AZTEC_DATA_10, 4, 2000);
    roundtrip(&AZTEC_DATA_12, 5, 1000);
}

#[test]
fn other_first_roots() {
    let f0: Gf2m<256> = Gf2m::new(0x11d, 0);
    let f7: Gf2m<256> = Gf2m::new(0x11d, 7);
    roundtrip(&f0, 11, 1500);
    roundtrip(&f7, 12, 1500);
}

#[test]
fn beyond_capacity_never_silently_wrong() {
    beyond_capacity(&AZTEC_PARAM, 21, 4000);
    beyond_capacity(&AZTEC_DATA_6, 22, 4000);
    beyond_capacity(&AZTEC_DATA_8, 23, 3000);
    beyond_capacity(&AZTEC_DATA_10, 24, 2000);
    beyond_capacity(&AZTEC_DATA_12, 25, 1000);
}

#[test]
fn exactly_at_capacity() {
    let mut rng = Rng(99);
    for ec in [3usize, 4, 5, 6, 7, 10, 24] {
        let clean = make_block(&AZTEC_DATA_8, &mut rng, ec);
        let n = clean.len();
        let mut block = clean.clone();
        for p in pick_distinct(&mut rng, n, ec / 2) {
            corrupt::<256>(&mut rng, &mut block, p);
        }
        assert_eq!(
            AZTEC_DATA_8.decode(&mut block, ec, &[]).map(|c| c.errors),
            Ok(ec / 2)
        );
        assert_eq!(block, clean);
        // All erasures, no errors.
        let mut block = clean.clone();
        let era = pick_distinct(&mut rng, n, ec);
        for &p in &era {
            block[p] = 0;
        }
        assert!(AZTEC_DATA_8.decode(&mut block, ec, &era).is_ok());
        assert_eq!(block, clean);
    }
}

#[test]
fn many_erasures_in_a_large_symbol() {
    let mut rng = Rng(5);
    let ec = 300;
    let data: Vec<u16> = (0..1000).map(|_| rng.below(4096) as u16).collect();
    let mut block = data.clone();
    block.resize(1000 + ec, 0);
    let (d, c) = block.split_at_mut(1000);
    AZTEC_DATA_12.encode(d, ec, c).unwrap();
    let clean = block.clone();
    let era = pick_distinct(&mut rng, block.len(), 120);
    for &p in &era {
        block[p] ^= 1 + rng.below(4095) as u16;
    }
    let errs = pick_distinct(&mut rng, block.len(), 90)
        .into_iter()
        .filter(|p| !era.contains(p))
        .collect::<Vec<_>>();
    for &p in &errs {
        block[p] ^= 1 + rng.below(4095) as u16;
    }
    let got = AZTEC_DATA_12.decode(&mut block, ec, &era).unwrap();
    assert_eq!(got.errors, errs.len());
    assert_eq!(block, clean);
}

#[test]
fn invalid_inputs() {
    let f = &AZTEC_PARAM;
    let mut b = [1u16, 2, 3, 4];
    assert_eq!(f.decode(&mut b, 5, &[]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[4]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[1, 1]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[0, 1, 2]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 0, &[]), Ok(Correction::default()));
    assert_eq!(f.decode(&mut b, 0, &[0]), Err(RsError::InvalidInput));
    let mut big = [0u16; 16];
    assert_eq!(f.decode(&mut big, 4, &[]), Err(RsError::InvalidInput));
    let mut wide = [16u16, 0, 0, 0];
    assert_eq!(f.decode(&mut wide, 2, &[]), Err(RsError::InvalidInput));
    let mut out = [0u16; 3];
    assert_eq!(f.encode(&[1, 2], 2, &mut out), Err(RsError::InvalidInput));
    assert_eq!(
        f.encode(&[0; 14], 2, &mut [0; 2]),
        Err(RsError::InvalidInput)
    );
    assert_eq!(f.encode(&[16], 2, &mut [0; 2]), Err(RsError::InvalidInput));
    assert_eq!(f.encode(&[1], 0, &mut []), Ok(()));
    let mut empty: [u16; 0] = [];
    assert_eq!(f.decode(&mut empty, 0, &[]), Ok(Correction::default()));
}

#[test]
fn max_size_blocks() {
    let mut rng = Rng(77);
    let mut block = vec![0u16; 15];
    block[0] = 7;
    let (d, c) = block.split_at_mut(1);
    AZTEC_PARAM.encode(d, 14, c).unwrap();
    let clean = block.clone();
    corrupt::<16>(&mut rng, &mut block, 0);
    corrupt::<16>(&mut rng, &mut block, 7);
    corrupt::<16>(&mut rng, &mut block, 14);
    assert_eq!(
        AZTEC_PARAM.decode(&mut block, 14, &[]).map(|c| c.errors),
        Ok(3)
    );
    assert_eq!(block, clean);
}
