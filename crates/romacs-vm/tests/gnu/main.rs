//! Every integration test of this package that boots GNU's Lisp, in one
//! executable whose allocator is the world arena, so each test process
//! restores that boot from a world instead of loading it from source
//! (`support/gnu_lisp.rs`).
//!
//! A file is a module here when it includes `support/gnu_lisp.rs` or a
//! support file built on it; every other file is a module of `all`
//! (`crates/romacs-vm-test-build`). Its tests keep their names, `FILE::NAME`,
//! and `cargo test --test FILE` still builds one file alone, booting from
//! source.

// A support file is a module of every test file that uses it, as it is in
// those files' own targets.
#![allow(clippy::duplicate_mod)]

use romacs_vm::world::arena::Arena;

#[global_allocator]
static ALLOCATOR: Arena = Arena;

romacs_vm_test_build::gnu_modules!();
