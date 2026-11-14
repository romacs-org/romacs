//! Lists romacs-vm's test files for `modules!` (src/lib.rs): each
//! `tests/FILE.rs` but those `tests/all/standalone.txt` names, as the
//! `#[path]` module declarations the `all` target holds.
//!
//! This is romacs-vm's test list, built here rather than by romacs-vm's own
//! build script: Cargo rebuilds a package's library whenever its build
//! script runs again, and a script that watches `tests/` runs again on every
//! edit under it. This package is a dev-dependency of romacs-vm, so such an
//! edit rebuilds romacs-vm's test units (its library compiled for its unit
//! tests among them), not the library the other packages and the binary
//! link.
use std::fmt::Write as _;
use std::path::Path;

fn main() {
    let tests = Path::new(env!("CARGO_MANIFEST_DIR")).join("../romacs-vm/tests");
    // The directory, which Cargo scans whole: any edit under it lists again.
    println!("cargo::rerun-if-changed={}", tests.display());
    let mut files: Vec<String> = std::fs::read_dir(&tests)
        .unwrap_or_else(|e| panic!("{}: {e}", tests.display()))
        .map(|entry| entry.expect("tests/ entry").path())
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .map(|path| path.file_stem().and_then(|s| s.to_str()).expect("a UTF-8 test file name").to_owned())
        .collect();
    files.sort();
    let listed = std::fs::read_to_string(tests.join("all/standalone.txt")).expect("tests/all/standalone.txt");
    let alone = standalone(&listed).unwrap_or_else(|e| panic!("tests/all/standalone.txt: {e}"));
    if let Some(gone) = alone.iter().find(|name| !files.contains(name)) {
        panic!("tests/all/standalone.txt: {gone} names no test file");
    }
    let mut text = String::new();
    for name in files.iter().filter(|name| !alone.contains(name)) {
        writeln!(text, "#[path = \"../{name}.rs\"]\nmod {name};").expect("writing to a String");
    }
    let out = Path::new(&std::env::var_os("OUT_DIR").expect("OUT_DIR")).join("modules.rs");
    std::fs::write(&out, text).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
}

/// The files of a standalone list's TEXT: one `FILE: reason` a line, each
/// file named once and with a reason; `#` begins a comment line.
fn standalone(text: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for line in text.lines().filter(|line| !line.trim().is_empty() && !line.starts_with('#')) {
        match line.split_once(": ") {
            Some((name, reason)) if !reason.trim().is_empty() && !names.iter().any(|n| n == name) => {
                names.push(name.to_owned());
            }
            _ => return Err(format!("`{line}` is not one `FILE: reason` for a file named once")),
        }
    }
    Ok(names)
}
