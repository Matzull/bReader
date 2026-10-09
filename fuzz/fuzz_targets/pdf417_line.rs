#![no_main]

// Input: [flags, widths...]; bit 0 of the flags says whether the first run is
// a bar, each following byte is a run width in tenths of a sample. Exercises
// the start pattern search, the codeword walk and the grid solver.

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((flags, rest)) = data.split_first() else {
        return;
    };
    let widths: Vec<f32> = rest.iter().map(|&b| f32::from(b) / 10.0).collect();
    let _ = breader_pdf417::fuzzing::read_line(flags & 1 == 1, &widths);
});
