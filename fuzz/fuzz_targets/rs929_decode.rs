#![no_main]

// Input: [mode, ec_lo, ec_hi, n_erasures, rest...]; `rest` is read as
// little-endian u16 pairs.
// - Mode bit 0 clear: an arbitrary block (values reduced modulo 929 unless bit
//   1 is set), followed by the erasure positions.
// - Mode bit 0 set: data codewords of a valid block, followed by (position,
//   value) pairs that overwrite it; the first n_erasures distinct positions
//   are flagged as erasures.

use breader_ecc::PDF417_FIELD;
use libfuzzer_sys::fuzz_target;

fn is_codeword(block: &[u16], ec_len: usize) -> bool {
    // A block is a codeword iff re-encoding its data part reproduces its check part.
    let k = block.len() - ec_len;
    let mut check = vec![0u16; ec_len];
    PDF417_FIELD
        .encode(&block[..k], ec_len, &mut check)
        .unwrap();
    check == block[k..]
}

fuzz_target!(|data: &[u8]| {
    let [mode, ec_lo, ec_hi, n_erasures, rest @ ..] = data else {
        return;
    };
    let ec_len = usize::from(u16::from_le_bytes([*ec_lo, *ec_hi])) % 520;
    let n_erasures = usize::from(*n_erasures);
    let words: Vec<u16> = rest
        .chunks_exact(2)
        .map(|p| u16::from_le_bytes([p[0], p[1]]))
        .collect();
    let (mut block, erasures): (Vec<u16>, Vec<usize>) = if mode & 1 == 0 {
        let n = words.len().min(928);
        let mut block: Vec<u16> = words[..n].to_vec();
        if mode & 2 == 0 {
            block.iter_mut().for_each(|w| *w %= 929);
        }
        let erasures = words[n..]
            .iter()
            .take(n_erasures)
            .map(|&e| usize::from(e) % n.max(1))
            .collect();
        (block, erasures)
    } else {
        let k = words.len().min(928usize.saturating_sub(ec_len)) / 2;
        let mut block: Vec<u16> = words[..k].iter().map(|w| w % 929).collect();
        let mut check = vec![0u16; ec_len];
        if PDF417_FIELD.encode(&block, ec_len, &mut check).is_err() {
            return;
        }
        block.extend(check);
        let n = block.len();
        let mut erasures = Vec::new();
        for (i, p) in words[k..].chunks_exact(2).enumerate() {
            let pos = usize::from(p[0]) % n.max(1);
            if n > 0 {
                block[pos] = p[1] % 929;
                if i < n_erasures && !erasures.contains(&pos) {
                    erasures.push(pos);
                }
            }
        }
        (block, erasures)
    };
    let original = block.clone();
    match PDF417_FIELD.decode(&mut block, ec_len, &erasures) {
        Ok(c) => {
            assert!(is_codeword(&block, ec_len), "not a codeword");
            let diff = block.iter().zip(&original).filter(|(a, b)| a != b).count();
            assert_eq!(diff, c.errors + c.erasures);
            assert!(2 * c.errors + erasures.len() <= ec_len);
        }
        Err(_) => assert_eq!(block, original, "block must be untouched on failure"),
    }
});
