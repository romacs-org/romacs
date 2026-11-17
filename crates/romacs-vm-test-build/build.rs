//! Lists romacs-vm's test files for `modules!` and `gnu_modules!`
//! (src/lib.rs), as `list.rs` writes a package's lists. A file that boots
//! GNU's Lisp (it includes one of `GNU_LISP`'s support files) is a module of
//! `gnu`, whose executable restores that boot from a world; every other file
//! is a module of `all`.
//!
//! This is romacs-vm's test list, built here rather than by romacs-vm's own
//! build script: Cargo rebuilds a package's library whenever its build
//! script runs again, and a script that watches `tests/` runs again on every
//! edit under it. This package is a dev-dependency of romacs-vm, so such an
//! edit rebuilds romacs-vm's test units (its library compiled for its unit
//! tests among them), not the library the other packages and the binary
//! link.
use std::path::Path;

#[path = "list.rs"]
mod list;

fn main() {
    let tests = Path::new("../romacs-vm/tests");
    list::write(tests, &["modules.rs", "gnu_modules.rs"],
                |source| if boots_gnu_lisp(source) { "gnu_modules.rs" } else { "modules.rs" });
}

/// The support files whose includers boot GNU's Lisp (`support/gnu_lisp.rs`,
/// and the two built on it). `tools/lib/test_modules.py` holds the same list.
const GNU_LISP: [&str; 3] = ["support/gnu_lisp.rs", "support/gnu_ert_vm.rs", "support/gui_probe.rs"];

/// Whether a test file's SOURCE includes one of `GNU_LISP`.
fn boots_gnu_lisp(source: &str) -> bool {
    GNU_LISP.iter().any(|support| source.contains(&format!("#[path = \"{support}\"]")))
}
