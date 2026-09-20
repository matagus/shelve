//! Helpers shared by this crate's unit tests.
//!
//! Compiled into the test binary only — `main.rs` declares this module behind
//! `#[cfg(test)]` — so none of it reaches a release build.
//!
//! This exists because measuring allocations is not a three-line job. Getting a
//! trustworthy count needs a `#[global_allocator]`, a process-wide lock so
//! parallel tests do not read each other's numbers, and fixtures built outside
//! the measured region. That is machinery, not test logic, and the second
//! performance issue re-derived it from scratch in a separate worktree before
//! this module existed. The next one should be able to write the assertion
//! alone.
//!
//! Integration tests under `tests/` are separate binaries: they cannot see this
//! module, and they do not need to, because they drive the compiled program
//! rather than its internals.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// Counts allocations, so a test can assert on the *shape* of the work and not
/// only on its output.
struct CountingAllocator;

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

/// Serialises the allocation-sensitive tests.
///
/// `ALLOCATIONS` is one process-wide counter and `cargo test` runs tests on
/// several threads, so two counting tests overlapping corrupt both: each sees
/// the other's allocations, and one test's `store(0)` can land in the middle of
/// another's measurement and truncate it. See [`measurement_lock`].
static MEASUREMENT: Mutex<()> = Mutex::new(());

/// Take exclusive ownership of the allocation counter for a whole test.
///
/// Isolating only the measured region is not enough. `ALLOCATIONS` counts every
/// allocation in the process, so a neighbouring test building its CSV fixture on
/// another thread inflates this one's numbers just as surely as a second
/// measurement would. Bind the returned guard to a name at the top of each
/// allocation-sensitive test and hold it until the body finishes; that is what
/// makes the counts reproducible under `cargo test`'s default parallelism.
///
/// Poisoning is ignored on purpose: the lock guards a counter, not an invariant,
/// so a panic in one test must not wedge the rest of the suite.
pub(crate) fn measurement_lock() -> MutexGuard<'static, ()> {
    MEASUREMENT.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Run `f` and report how many allocations it made. Every measurement is a
/// fresh count, so callers never have to reason about what ran before.
///
/// The enclosing test must hold [`measurement_lock`].
pub(crate) fn count_allocations<R>(f: impl FnOnce() -> R) -> (R, usize) {
    ALLOCATIONS.store(0, Ordering::SeqCst);
    let value = f();
    (value, ALLOCATIONS.load(Ordering::SeqCst))
}

/// A `columns`-wide CSV with `rows` records and no header line.
///
/// Column 1 is the grouping key and only ever takes two values, so every width
/// and every row count builds the same two groups: the map cost is then
/// identical across the two sides of a comparison and the delta is purely the
/// thing under measurement.
///
/// There is no header line, but `csv::Reader` consumes the first record as one
/// anyway, so `rows` records parse as `rows - 1`. That is fine for every use
/// here — the comparisons are between two calls that lose the same record — and
/// worth knowing before reading an exact count as a per-row rate.
///
/// Callers build this *outside* the region they measure. Constructing the
/// fixture allocates far more than parsing it, so including it would bury the
/// signal.
pub(crate) fn build_csv(rows: usize, columns: usize) -> String {
    let mut out = String::new();
    for row in 0..rows {
        write!(out, "g{}", row % 2).expect("writing to a String cannot fail");
        for column in 1..columns {
            write!(out, ",r{row}c{column}").expect("writing to a String cannot fail");
        }
        out.push('\n');
    }
    out
}
