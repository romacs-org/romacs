//! What the compiler enforces of the rooting rule, on these very types with a
//! stand-in engine (`rooting/standin.rs`). Each `rooting/fail/` program breaks
//! one rule (a heap value out of a leaf or past its request, the engine after
//! its dispatch, a permission into a leaf or across VMs) and its `.stderr` pins
//! WHICH error rejects it, so an unrelated failure (a typo, a missing import)
//! cannot pass for the property. `rooting/pass/` is the positive control.
//!
//! This crate depends on nothing, so these build in seconds on every lane.

#[test]
fn the_compiler_enforces_the_rules() {
    let t = trybuild::TestCases::new();
    t.pass("tests/rooting/pass/*.rs");
    t.compile_fail("tests/rooting/fail/*.rs");
}
