//! Spare text buffers, so that hits do not allocate in steady state.
//!
//! Every hit owns its text. A scanline can read a few spurious rows of
//! stacked GS1 DataBar on a texture (they only become symbols when rows join),
//! and each of those would allocate its text on every scan. The aggregator
//! gives the buffers of the hits it forgets back here and the DataBar part
//! decoders take them from here. The pool is per thread, like the scan state
//! that fills it (`reader::WORK`), and bounded.

use std::cell::RefCell;

/// Buffers kept per thread; more are dropped.
const MAX_SPARE: usize = 256;

thread_local! {
    static SPARE: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

/// An empty buffer with room for `capacity` bytes, reusing a spare one when
/// there is one.
pub(crate) fn take(capacity: usize) -> Vec<u8> {
    let mut v = SPARE
        .try_with(|s| s.try_borrow_mut().ok().and_then(|mut s| s.pop()))
        .ok()
        .flatten()
        .unwrap_or_default();
    v.clear();
    v.reserve(capacity);
    v
}

/// Keeps `v` for [`take`].
pub(crate) fn give(v: Vec<u8>) {
    if v.capacity() == 0 {
        return;
    }
    let _ = SPARE.try_with(|s| {
        if let Ok(mut s) = s.try_borrow_mut()
            && s.len() < MAX_SPARE
        {
            s.push(v);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_given_buffer_comes_back_empty_and_keeps_its_room() {
        let mut v = take(40);
        v.extend_from_slice(b"abc");
        let ptr = v.as_ptr();
        give(v);
        let w = take(8);
        assert!(w.is_empty());
        assert!(w.capacity() >= 40);
        assert_eq!(w.as_ptr(), ptr);
    }
}
