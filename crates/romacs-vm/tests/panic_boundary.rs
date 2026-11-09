//! A session never uses its VM after a panic (`Vm::catching' states this).
//!
//! No frame restores the VM's nesting state as a panic crosses it, so whatever
//! catches a panic and then uses the VM must go through `Vm::catching'. The
//! compiler holds most of this: `&mut Vm' is not `UnwindSafe', so `catch_unwind'
//! refuses a closure that keeps using one, and a closure that owns its VM gives
//! nothing back to use (nor does a thread's `join'). Only `AssertUnwindSafe' gets
//! past that. So this lists every `AssertUnwindSafe' in a package that can reach
//! a VM (romacs-vm and whatever depends on it), tests included, with why each is
//! safe, and fails on one it does not name, in a new package too. A session's code
//! may not call `catching' at all.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// PATH, how many, why each is safe.
const SITES: &[(&str, usize, &str)] = &[
    ("crates/romacs-vm-gui/src/main.rs", 1,
     "the evaluator thread's top: the unwinding drops the VM, and the session ends"),
    ("crates/romacs-vm/src/gc.rs", 2,
     "`Vm::catching' itself, and a mark handing its host back before re-raising"),
    ("crates/romacs-vm/src/gc_mark.rs", 4,
     "tests of a panic inside the collector, which nests no engine"),
    ("crates/romacs-vm/src/handle.rs", 1, "a test of a host panicking in an idle mark, which nests no engine"),
    ("crates/romacs-vm/src/reader.rs", 1, "a test that asserts no panic came"),
    ("crates/romacs-vm/tests/gnu_reader.rs", 1, "a test that asserts no panic came"),
    ("gnu-tests/runner/src/main.rs", 6,
     "each reports the panic and exits (`panic_exit'); what goes on after one uses `catching'"),
];

/// Packages a session's code lives in.
const SESSION: &[&str] = &["crates/romacs-vm-gui", "crates/romacs-layout2", "crates/romacs-introspect-live"];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("the repository").to_path_buf()
}

/// The package directories that are romacs-vm or depend on it.
fn reaching_packages(root: &Path) -> Vec<PathBuf> {
    let mut packages = vec![root.join("gnu-tests/runner")];
    for entry in std::fs::read_dir(root.join("crates")).unwrap().flatten() {
        let manifest = std::fs::read_to_string(entry.path().join("Cargo.toml")).unwrap_or_default();
        if manifest.lines().any(|line| line.starts_with("romacs-vm =") || line.trim() == "name = \"romacs-vm\"") {
            packages.push(entry.path());
        }
    }
    packages.sort();
    packages.dedup();
    packages
}

fn rust_files(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for path in entries.flatten().map(|entry| entry.path()) {
        if path.is_dir() { rust_files(&path, found); }
        else if path.extension().is_some_and(|e| e == "rs") { found.push(path); }
    }
}

/// How many times each source of PACKAGES says NEEDLE, this file aside.
fn occurrences(root: &Path, packages: &[PathBuf], needle: &str) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for package in packages {
        let mut files = Vec::new();
        rust_files(&package.join("src"), &mut files);
        rust_files(&package.join("tests"), &mut files);
        for file in files {
            if file.ends_with("panic_boundary.rs") { continue; }
            let n = std::fs::read_to_string(&file).unwrap().matches(needle).count();
            if n > 0 {
                counts.insert(file.strip_prefix(root).unwrap().to_string_lossy().into_owned(), n);
            }
        }
    }
    counts
}

#[test]
fn every_way_past_the_compiler_is_named() {
    let root = root();
    let found = occurrences(&root, &reaching_packages(&root), "AssertUnwindSafe(");
    let listed: BTreeMap<String, usize> = SITES.iter().map(|&(path, n, _)| (path.to_owned(), n)).collect();
    assert_eq!(found, listed, "a session never uses its VM after a panic (`Vm::catching'): \
        name each `AssertUnwindSafe' here with why it is safe, or go through `Vm::catching'");
}

#[test]
fn a_session_does_not_recover_from_a_panic() {
    let root = root();
    let session: Vec<PathBuf> = SESSION.iter().map(|package| root.join(package)).collect();
    let found = occurrences(&root, &session, ".catching(");
    assert!(found.is_empty(), "a session ends at a panic, and has no use for `Vm::catching': {found:?}");
}
