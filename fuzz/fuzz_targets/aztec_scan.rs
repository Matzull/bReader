#![no_main]

// Input: [shape, width, noise_len, rest...]. Shape bit 0 set: a valid symbol
// built around `rest` with pixels flipped at the places named by the first
// `noise_len` bytes; clear: `rest` are the pixels of an image `width` wide.
// Shape bits 1-2 pick the effort.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let [shape, width, noise_len, rest @ ..] = data else {
        return;
    };
    let split = usize::from(*noise_len).min(rest.len());
    let (noise, bytes) = rest.split_at(split);
    breader_aztec::fuzzing::scan_image(
        shape & 1 != 0,
        shape >> 1,
        usize::from(*width),
        noise,
        bytes,
    );
});
