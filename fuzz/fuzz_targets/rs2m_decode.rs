#![no_main]

// Input: [field, mode, ec, n_erasures, rest...]; `rest` is read as
// little-endian u16 pairs. `field` selects one of the five Aztec fields.
// - Mode bit 0 clear: an arbitrary block (values reduced to the field unless
//   bit 1 is set), followed by the erasure positions.
// - Mode bit 0 set: data symbols of a valid block, followed by (position,
//   value) pairs that overwrite it; the first n_erasures distinct positions
//   are flagged as erasures.

use breader_ecc::{AZTEC_DATA_6, AZTEC_DATA_8, AZTEC_DATA_10, AZTEC_DATA_12, AZTEC_PARAM, Gf2m};
use libfuzzer_sys::fuzz_target;

fn is_codeword<const Q: usize>(f: &Gf2m<Q>, block: &[u16], ec_len: usize) -> bool {
    let k = block.len() - ec_len;
    let mut check = vec![0u16; ec_len];
    f.encode(&block[..k], ec_len, &mut check).unwrap();
    check == block[k..]
}

fn run<const Q: usize>(f: &Gf2m<Q>, mode: u8, ec_len: usize, n_erasures: usize, words: &[u16]) {
    let max_len = Q - 1;
    let q = Q as u16;
    let (mut block, erasures): (Vec<u16>, Vec<usize>) = if mode & 1 == 0 {
        let n = words.len().min(max_len);
        let mut block: Vec<u16> = words[..n].to_vec();
        if mode & 2 == 0 {
            block.iter_mut().for_each(|w| *w %= q);
        }
        let erasures = words[n..]
            .iter()
            .take(n_erasures)
            .map(|&e| usize::from(e) % n.max(1))
            .collect();
        (block, erasures)
    } else {
        let k = words.len().min(max_len.saturating_sub(ec_len)) / 2;
        let mut block: Vec<u16> = words[..k].iter().map(|w| w % q).collect();
        let mut check = vec![0u16; ec_len];
        if f.encode(&block, ec_len, &mut check).is_err() {
            return;
        }
        block.extend(check);
        let n = block.len();
        let mut erasures = Vec::new();
        for (i, p) in words[k..].chunks_exact(2).enumerate() {
            let pos = usize::from(p[0]) % n.max(1);
            if n > 0 {
                block[pos] = p[1] % q;
                if i < n_erasures && !erasures.contains(&pos) {
                    erasures.push(pos);
                }
            }
        }
        (block, erasures)
    };
    let original = block.clone();
    match f.decode(&mut block, ec_len, &erasures) {
        Ok(c) => {
            assert!(is_codeword(f, &block, ec_len), "not a codeword");
            let diff = block.iter().zip(&original).filter(|(a, b)| a != b).count();
            assert_eq!(diff, c.errors + c.erasures);
            assert!(2 * c.errors + erasures.len() <= ec_len);
        }
        Err(_) => assert_eq!(block, original, "block must be untouched on failure"),
    }
}

fuzz_target!(|data: &[u8]| {
    let [field, mode, ec, n_erasures, rest @ ..] = data else {
        return;
    };
    let words: Vec<u16> = rest
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    let n_erasures = usize::from(*n_erasures);
    let ec = usize::from(*ec);
    match field % 5 {
        0 => run(&AZTEC_PARAM, *mode, ec % 16, n_erasures, &words),
        1 => run(&AZTEC_DATA_6, *mode, ec % 64, n_erasures, &words),
        2 => run(&AZTEC_DATA_8, *mode, ec, n_erasures, &words),
        3 => run(&AZTEC_DATA_10, *mode, ec * 4, n_erasures, &words),
        _ => run(&AZTEC_DATA_12, *mode, ec * 16, n_erasures, &words),
    }
});
