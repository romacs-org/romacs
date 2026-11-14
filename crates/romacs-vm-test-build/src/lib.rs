//! romacs-vm's test sources made when its tests build, so none is committed:
//! `modules!()`, the `all` target's module list (build.rs lists it), and
//! `primh_harness!()` and `varh_harness!()`, the replay harnesses of
//! tools/primh and tools/varh rendered for this crate.
//!
//! A harness's template is provenance-hashed: each recording names the
//! template's SHA-256, so the template stays as recorded and is rendered here
//! instead. The rendered code includes the template itself (to check that
//! hash), so a changed template rebuilds the test that expands it.

use std::path::PathBuf;

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

/// The primh replay harness (tools/primh/replay.rs.in) for this crate: its
/// helpers, `replay(EVIDENCE, SPEC)` and the harness's own mutation tests.
/// The invoking file, in romacs-vm's tests/, adds one test per primitive.
///
/// # Panics
///
/// When given arguments, or when the template cannot be read or rendered.
#[proc_macro]
pub fn primh_harness(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    assert!(input.is_empty(), "primh_harness!() takes no arguments");
    let text = template("tools/primh/replay.rs.in");
    // Without it the replays would compare a declaration's line again.
    assert_eq!(text.matches(PINNED_LINE).count(), 1, "the primh template compares a declaration's Rust location once");
    adapt(&text).parse().expect("the primh template renders as Rust")
}

/// The varh replay harness (tools/varh/replay.rs.in) for this crate, its
/// inputs named from romacs-vm's tests/ as `varh.py rust` names them.
///
/// # Panics
///
/// When given arguments, or when the template cannot be read or rendered.
#[proc_macro]
pub fn varh_harness(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    assert!(input.is_empty(), "varh_harness!() takes no arguments");
    let mut text = template("tools/varh/replay.rs.in");
    for (key, path) in [("@EVIDENCE@", "evidence/varh.json"), ("@CASES@", "cases.json"), ("@HARNESS@", "varh.py"),
                        ("@DRIVER@", "gnu_driver.el"), ("@TEMPLATE@", "replay.rs.in")] {
        text = text.replace(key, &format!("\"../tools/varh/{path}\""));
    }
    adapt(&text).parse().expect("the varh template renders as Rust")
}

/// The recorded template's comparison of a declaration's Rust location,
/// file and line.
const PINNED_LINE: &str = "    assert_eq!(spec[\"rust\"], current[\"rust\"], \"declaration rust stale\");\n";

/// The template at RELATIVE in romacs-vm, the package whose tests expand
/// these macros.
fn template(relative: &str) -> String {
    let package = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo names the package"));
    let path = package.join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A recorded template, as written for the VM it was recorded against
/// (crate `romacs_vm2`, `vm.display`), adapted to this crate: its name, its
/// value-display API, and a declaration's Rust location compared by file,
/// not line, since a line moves whenever code above it does.
fn adapt(template: &str) -> String {
    const RECORDED: &str = "use romacs_vm2::Vm;";
    const FILE_ONLY: &str = "    let file = |v: &Value| v.as_str().map(|s| s.rsplit_once(':').map_or(s, |(f, _)| f).to_owned());\n    \
                             assert_eq!(file(&spec[\"rust\"]), file(&current[\"rust\"]), \"declaration rust stale\");\n";
    assert_eq!(template.matches(RECORDED).count(), 1, "a template imports its VM once");
    template.replacen(RECORDED, "use romacs_vm::Vm;", 1).replacen(PINNED_LINE, FILE_ONLY, 1).replace("vm.display(v)", "v.display()")
}
