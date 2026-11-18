//! Counts the bytes each thread allocates, for tests that must show an event was not copied:
//! a copy of a large event allocates its size, and a timer would only hint at it. The test
//! binary's allocator forwards every call to `System` unchanged.
#![allow(unsafe_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ALLOCATED: Cell<usize> = const { Cell::new(0) };
}

/// Bytes allocated on this thread so far. A current-thread runtime runs a test's tasks on the
/// test's own thread, so other tests running alongside do not count.
pub(crate) fn allocated() -> usize {
    ALLOCATED.try_with(Cell::get).unwrap_or(0)
}

fn count(bytes: usize) {
    let _ = ALLOCATED.try_with(|allocated| allocated.set(allocated.get().saturating_add(bytes)));
}

struct Counting;

// SAFETY: each method only records a size, then passes its arguments to `System`'s, whose
// contract the caller already meets.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: as above.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count(layout.size());
        // SAFETY: as above.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count(new_size);
        // SAFETY: as above.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: as above.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;
