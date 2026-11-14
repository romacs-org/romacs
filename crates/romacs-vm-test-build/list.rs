//! A package's folded test targets' module lists, as a build script writes
//! them: each `tests/FILE.rs` but those `tests/all/standalone.txt` names,
//! as the `#[path]` module declaration of the target that holds it. Shared
//! by the build scripts of romacs-vm-test-build and
//! romacs-vm-gui-test-build, one crate a package, so an edit of one
//! package's tests rebuilds no other package's (`build.rs`).
use std::fmt::Write as _;
use std::path::Path;

/// Write, into Cargo's `OUT_DIR`, each of OUTPUTS: the module declarations
/// of the files of TESTS that PLACE (given a file's source) puts in it.
/// Cargo runs the script again on any edit under TESTS.
pub fn write(tests: &Path, outputs: &[&str], place: impl Fn(&str) -> &'static str) {
    // The directory, which Cargo scans whole: any edit under it lists again.
    println!("cargo::rerun-if-changed={}", tests.display());
    let mut files: Vec<String> = std::fs::read_dir(tests)
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
    let mut lists: Vec<(&str, String)> = outputs.iter().map(|&output| (output, String::new())).collect();
    for name in files.iter().filter(|name| !alone.contains(name)) {
        let path = tests.join(format!("{name}.rs"));
        let source = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let output = place(&source);
        let (_, text) = lists.iter_mut().find(|(o, _)| *o == output).unwrap_or_else(|| panic!("{output} is not an output"));
        writeln!(text, "#[path = \"../{name}.rs\"]\nmod {name};").expect("writing to a String");
    }
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    for (file, text) in lists {
        std::fs::write(out.join(file), text).unwrap_or_else(|e| panic!("{}: {e}", out.join(file).display()));
    }
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
