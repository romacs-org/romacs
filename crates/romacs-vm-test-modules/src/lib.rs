//! `modules!()`: romacs-vm's test files as modules of the `all` target
//! (crates/romacs-vm/tests/all/main.rs), as build.rs lists them.

/// Each `tests/FILE.rs` of romacs-vm, but the standalone ones, as the
/// module `FILE` (`#[path = "../FILE.rs"] mod FILE;`).
///
/// # Panics
///
/// When given arguments: it takes none.
#[proc_macro]
pub fn modules(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    assert!(input.is_empty(), "modules!() takes no arguments");
    include_str!(concat!(env!("OUT_DIR"), "/modules.rs")).parse().expect("build.rs writes Rust")
}
