//! romacs-vm-gui's test sources made when its tests build, so none is
//! committed: `modules!()`, the `all` target's module list (build.rs lists
//! it).

/// Each `tests/FILE.rs` of romacs-vm-gui that `tests/all/standalone.txt`
/// does not name, as the module `FILE` (`#[path = "../FILE.rs"] mod FILE;`).
///
/// # Panics
///
/// When given arguments: it takes none.
#[proc_macro]
pub fn modules(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    assert!(input.is_empty(), "modules!() takes no arguments");
    include_str!(concat!(env!("OUT_DIR"), "/modules.rs")).parse().expect("build.rs writes Rust")
}
