//! Every integration test of this package in one executable.
//!
//! Cargo makes each `tests/FILE.rs` a test target of its own, compiled and
//! linked apart from the others. This target holds each of those files as
//! its module `FILE` (`modules.rs`), so a check builds one executable for
//! them all; the test `NAME` of `tests/FILE.rs` is `FILE::NAME` here.
//! `cargo test --test FILE` still builds and runs one file alone.
//!
//! `modules.rs` is written from the directory's files by
//! `tools/lib/test_modules.py`, which says why it is not a build script's
//! work. A file that cannot be a module is named in `standalone.txt` with
//! its reason, and a check runs its target beside this one.

// A support file is a module of every test file that uses it, as it is in
// those files' own targets.
#![allow(clippy::duplicate_mod)]

include!("modules.rs");

/// A new test file joins `modules.rs` when the tool next writes it. Until
/// then this target would pass without that file's tests, so it fails.
#[test]
fn every_test_file_is_a_module_or_stands_alone_for_a_reason() {
    use std::collections::BTreeSet;
    let tests = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let files: BTreeSet<String> = std::fs::read_dir(&tests).unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|e| e == "rs"))
        .map(|path| path.file_stem().unwrap().to_str().unwrap().to_owned())
        .collect();
    let modules: BTreeSet<&str> = include_str!("modules.rs").lines()
        .filter_map(|line| line.strip_prefix("mod ")?.strip_suffix(';'))
        .collect();
    let standalone: Vec<(&str, &str)> = include_str!("standalone.txt").lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_once(": ").unwrap_or_else(|| panic!("standalone.txt: `{line}` gives no reason")))
        .collect();
    assert!(standalone.iter().all(|(_, reason)| !reason.trim().is_empty()), "standalone.txt: a file without a reason");
    let listed: BTreeSet<&str> = modules.iter().copied().chain(standalone.iter().map(|(file, _)| *file)).collect();
    assert_eq!(listed.len(), modules.len() + standalone.len(), "a file is listed twice, in modules.rs and standalone.txt");
    assert!(!modules.is_empty(), "modules.rs declares no module");
    assert_eq!(
        files.iter().map(String::as_str).collect::<BTreeSet<_>>(), listed,
        "tests/*.rs and the files modules.rs and standalone.txt name differ: run `python3 tools/lib/test_modules.py write`");
}
