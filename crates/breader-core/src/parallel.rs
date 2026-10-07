//! Test hook for the optional multi-threaded kernels.

use std::cell::Cell;

thread_local! {
    static SEQUENTIAL: Cell<bool> = const { Cell::new(false) };
}

/// True while [`sequential`] is running on this thread. Modules with a
/// parallel path check it on the calling thread, before dispatching work.
#[doc(hidden)]
pub fn is_sequential() -> bool {
    SEQUENTIAL.with(Cell::get)
}

/// Runs `f` with every parallel path of the calling thread disabled, so
/// tests can compare both code paths in one build.
#[doc(hidden)]
pub fn sequential<R>(f: impl FnOnce() -> R) -> R {
    let before = SEQUENTIAL.with(|s| s.replace(true));
    let out = f();
    SEQUENTIAL.with(|s| s.set(before));
    out
}
