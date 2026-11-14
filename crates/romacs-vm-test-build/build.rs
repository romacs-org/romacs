//! Lists romacs-vm's test files for `modules!` and `gnu_modules!`
//! (src/lib.rs): each `tests/FILE.rs` but those `tests/all/standalone.txt`
//! names, as the `#[path]` module declarations of the target that holds
//! it. A file that boots GNU's Lisp (it includes one of `GNU_LISP`'s
//! support files) is a module of `gnu`, whose executable restores that
//! boot from a world; every other file is a module of `all`.
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
    let (mut all, mut gnu) = (String::new(), String::new());
    for name in files.iter().filter(|name| !alone.contains(name)) {
        let path = tests.join(format!("{name}.rs"));
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let target = if boots_gnu_lisp(&source) { &mut gnu } else { &mut all };
        writeln!(target, "#[path = \"../{name}.rs\"]\nmod {name};").expect("writing to a String");
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    for (file, text) in [("modules.rs", all), ("gnu_modules.rs", gnu)] {
        std::fs::write(out.join(file), text).unwrap_or_else(|e| panic!("{}: {e}", out.join(file).display()));
    }
}

/// The support files whose includers boot GNU's Lisp (`support/gnu_lisp.rs`,
/// and the two built on it). `tools/lib/test_modules.py` holds the same list.
const GNU_LISP: [&str; 3] = ["support/gnu_lisp.rs", "support/gnu_ert_vm.rs", "support/gui_probe.rs"];

/// Whether a test file's SOURCE includes one of `GNU_LISP`.
fn boots_gnu_lisp(source: &str) -> bool {
    GNU_LISP.iter().any(|support| source.contains(&format!("#[path = \"{support}\"]")))
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
