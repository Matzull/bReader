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

fn make_block(field: &Gf256, rng: &mut Rng, ec: usize) -> Vec<u8> {
    let total = ec + 1 + rng.below(255 - ec);
    let mut block: Vec<u8> = (0..total).map(|_| rng.next() as u8).collect();
    let (data, check) = block.split_at_mut(total - ec);
    field.encode(data, ec, check).unwrap();
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

fn corrupt(rng: &mut Rng, block: &mut [u8], positions: &[usize]) {
    for &p in positions {
        block[p] ^= 1 + rng.below(255) as u8;
    }
}

#[test]
fn qr_annex_i_vector() {
    let data = [
        0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC,
        0x11,
    ];
    let ec = [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55];
    let mut out = [0u8; 10];
    QR_FIELD.encode(&data, 10, &mut out).unwrap();
    assert_eq!(out, ec);

    let mut block = [data.as_slice(), ec.as_slice()].concat();
    assert_eq!(QR_FIELD.decode(&mut block, 10, &[]), Ok(0));
    block[3] ^= 0x77;
    block[20] ^= 0x01;
    assert_eq!(QR_FIELD.decode(&mut block, 10, &[]), Ok(2));
    assert_eq!(&block[..16], &data);
}

fn roundtrip_cases(field: &Gf256, seed: u64, cases: usize) {
    let mut rng = Rng(seed);
    for _ in 0..cases {
        let ec = 2 + rng.below(67);
        let clean = make_block(field, &mut rng, ec);
        let n = clean.len();
        let e = rng.below(ec / 2 + 1);
        let f = rng.below(ec - 2 * e + 1).min(n);
        let positions = pick_distinct(&mut rng, n, (e + f).min(n));
        let (err_pos, era_pos) = positions.split_at(e.min(positions.len()));
        let mut block = clean.clone();
        corrupt(&mut rng, &mut block, err_pos);
        // Erased symbols may or may not actually differ.
        for &p in era_pos {
            if rng.below(4) != 0 {
                block[p] ^= 1 + rng.below(255) as u8;
            }
        }
        let expected = (0..n).filter(|&i| block[i] != clean[i]).count();
        let got = field.decode(&mut block, ec, era_pos);
        assert_eq!(got, Ok(expected), "ec={ec} n={n} e={e} f={f}");
        assert_eq!(block, clean);
    }
}

#[test]
fn qr_field_roundtrip() {
    roundtrip_cases(&QR_FIELD, 0x9e3779b97f4a7c15, 6000);
}

#[test]
fn data_matrix_field_roundtrip() {
    let dm = Gf256::new(0x12d, 1);
    roundtrip_cases(&dm, 0xdeadbeefcafef00d, 4000);
    let other = Gf256::new(0x11d, 7);
    roundtrip_cases(&other, 12345, 1000);
}

#[test]
fn beyond_capacity_never_silently_wrong() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let mut ok_count = 0;
    for _ in 0..6000 {
        let ec = 2 + rng.below(67);
        let clean = make_block(&QR_FIELD, &mut rng, ec);
        let n = clean.len();
        let k = (ec / 2 + 1 + rng.below(ec)).min(n);
        let mut block = clean.clone();
        let pos = pick_distinct(&mut rng, n, k);
        corrupt(&mut rng, &mut block, &pos);
        let corrupted = block.clone();
        match QR_FIELD.decode(&mut block, ec, &[]) {
            Ok(c) => {
                ok_count += 1;
                let mut s = [0u8; N];
                assert!(QR_FIELD.syndromes(&block, ec, &mut s));
                let diff = (0..n).filter(|&i| block[i] != corrupted[i]).count();
                assert_eq!(c, diff);
            }
            Err(e) => {
                assert_eq!(e, RsError::Uncorrectable);
                assert_eq!(block, corrupted, "block must be untouched on failure");
            }
        }
    }
    assert!(ok_count < 6000);
}

#[test]
fn invalid_inputs() {
    let f = &QR_FIELD;
    let mut b = [1u8, 2, 3, 4];
    assert_eq!(f.decode(&mut b, 5, &[]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[4]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[1, 1]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 2, &[0, 1, 2]), Err(RsError::InvalidInput));
    assert_eq!(f.decode(&mut b, 0, &[]), Ok(0));
    assert_eq!(f.decode(&mut b, 0, &[0]), Err(RsError::InvalidInput));
    let mut big = [0u8; 256];
    assert_eq!(f.decode(&mut big, 4, &[]), Err(RsError::InvalidInput));
    let mut empty: [u8; 0] = [];
    assert_eq!(f.decode(&mut empty, 0, &[]), Ok(0));
    let mut out = [0u8; 3];
    assert_eq!(f.encode(&[1, 2], 2, &mut out), Err(RsError::InvalidInput));
    assert_eq!(
        f.encode(&[0; 250], 6, &mut [0; 6]),
        Err(RsError::InvalidInput)
    );
    assert_eq!(f.encode(&[1], 0, &mut []), Ok(()));
}

#[test]
fn max_size_block() {
    let mut rng = Rng(77);
    let mut block = vec![0u8; 255];
    for b in block[..1].iter_mut() {
        *b = rng.next() as u8;
    }
    let (d, c) = block.split_at_mut(1);
    QR_FIELD.encode(d, 254, c).unwrap();
    let clean = block.clone();
    corrupt(&mut rng, &mut block, &[0, 100, 254]);
    assert_eq!(QR_FIELD.decode(&mut block, 254, &[]), Ok(3));
    assert_eq!(block, clean);
}
