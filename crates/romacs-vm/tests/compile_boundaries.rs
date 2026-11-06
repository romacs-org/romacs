//! What the compiler enforces: the heap boundary and snapshot safety. Each
//! `fail/` program must not compile, and its `.stderr` pins WHICH error
//! rejects it, so an unrelated failure (a typo, a missing import) cannot pass
//! for the property; `pass/` is the positive control. A `heap_boundary/fail/`
//! program holds a `Local` across a call that can collect, or reaches for the
//! raw moving index.
//!
//! One test for the whole crate: trybuild keeps one project per crate and
//! rewrites its manifest. A second test binary, run in parallel by nextest,
//! can take it over when trybuild's file lock looks stale under load; cargo
//! then builds the other's programs, and trybuild counts a program it has no
//! message for as compiled.

#[test]
fn the_compiler_enforces_the_boundaries() {
    let t = trybuild::TestCases::new();
    t.pass("tests/heap_boundary/pass/*.rs");
    t.compile_fail("tests/heap_boundary/fail/*.rs");
    t.pass("tests/snapshot_safe/pass/*.rs");
    t.compile_fail("tests/snapshot_safe/fail/*.rs");
}
