#![no_main]

use breader_ecc::{Gf256, QR_FIELD};
use libfuzzer_sys::fuzz_target;

fn syndromes_zero(field: &Gf256, block: &[u8], ec_len: usize) -> bool {
    // A block is a codeword iff re-encoding its data part reproduces its check part.
    let k = block.len() - ec_len;
    let mut check = vec![0u8; ec_len];
    field.encode(&block[..k], ec_len, &mut check).unwrap();
    check == block[k..]
}

fuzz_target!(|data: &[u8]| {
    let [field_sel, ec, n_erasures, rest @ ..] = data else {
        return;
    };
    let dm = Gf256::new(0x12d, 1);
    let field: &Gf256 = if field_sel & 1 == 0 { &QR_FIELD } else { &dm };
    let ec_len = usize::from(*ec);
    let n_erasures = usize::from(*n_erasures);
    let n = rest.len().min(255);
    let (block_bytes, erasure_bytes) = rest.split_at(n);
    let mut block = block_bytes.to_vec();
    let erasures: Vec<usize> = erasure_bytes
        .iter()
        .take(n_erasures)
        .map(|&e| usize::from(e))
        .collect();
    let original = block.clone();
    if let Ok(changed) = field.decode(&mut block, ec_len, &erasures) {
        assert!(syndromes_zero(field, &block, ec_len), "not a codeword");
        let diff = block.iter().zip(&original).filter(|(a, b)| a != b).count();
        assert_eq!(diff, changed);
    }
});
