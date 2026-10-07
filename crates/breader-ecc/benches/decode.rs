use breader_ecc::QR_FIELD;
use criterion::{Criterion, black_box, criterion_group, criterion_main};

fn make_block() -> [u8; 255] {
    let mut block = [0u8; 255];
    for (i, b) in block[..223].iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37).wrapping_add(11);
    }
    let (data, ec) = block.split_at_mut(223);
    QR_FIELD.encode(data, 32, ec).unwrap();
    block
}

fn bench_decode(c: &mut Criterion) {
    let clean = make_block();
    let mut dirty = clean;
    for k in 0..16 {
        dirty[k * 15 + 3] ^= 0x5a ^ (k as u8);
    }
    c.bench_function("decode_255_223_clean", |b| {
        b.iter(|| {
            let mut blk = clean;
            black_box(QR_FIELD.decode(black_box(&mut blk), 32, &[]).unwrap())
        })
    });
    c.bench_function("decode_255_223_16_errors", |b| {
        b.iter(|| {
            let mut blk = dirty;
            black_box(QR_FIELD.decode(black_box(&mut blk), 32, &[]).unwrap())
        })
    });
}

criterion_group!(benches, bench_decode);
criterion_main!(benches);
