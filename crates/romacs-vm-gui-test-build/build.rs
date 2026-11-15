//! Lists romacs-vm-gui's test files for `modules!` (src/lib.rs), as
//! romacs-vm-test-build's `list.rs` writes a package's lists: every file
//! but the standalone ones is a module of `all`.
//!
//! A crate of its own, not a second list of romacs-vm-test-build: a
//! proc-macro crate builds again whenever its build script runs, and
//! the script runs on every edit under the tests it watches, so one crate
//! for both packages would rebuild each package's test units on an edit of
//! the other's tests.
use std::path::Path;

#[path = "../romacs-vm-test-build/list.rs"]
mod list;

fn main() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("../romacs-vm-gui/tests");
    list::write(&tests, &["modules.rs"], |_| "modules.rs");
}
