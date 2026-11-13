//! Every integration test of this package in one executable.
//!
//! Cargo makes each `tests/FILE.rs` a test target of its own, compiled and
//! linked apart from the others. This target holds each of those files as
//! its module `FILE`, so a check builds one executable for them all; the
//! test `NAME` of `tests/FILE.rs` is `FILE::NAME` here. `cargo test --test
//! FILE` still builds and runs one file alone.
//!
//! The modules are listed when this target builds, from the directory's
//! files (crates/romacs-vm-test-modules), so a new test file edits no list.
//! A file that cannot be a module is named in `standalone.txt` with its
//! reason, and a check runs its target beside this one
//! (`tools/lib/test_modules.py targets`).

// A support file is a module of every test file that uses it, as it is in
// those files' own targets.
#![allow(clippy::duplicate_mod)]

romacs_vm_test_modules::modules!();
