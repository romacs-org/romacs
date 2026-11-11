//! A session never uses its VM after a panic (`Vm::catching` states this).
//!
//! No frame restores the VM's nesting state as a panic crosses it, so whatever
//! catches a panic and then uses the VM must go through `Vm::catching`. For a VM
//! to outlive a panic, another thread must hold it or a catch must stop the panic
//! on its own. `Vm` is not `Send` (checked below), so no other thread can, short of
//! an `unsafe impl Send` on what holds it, which nothing has and which would break
//! this rule. So in every package that can reach a VM (romacs-vm and whatever
//! depends on it), this lists each `catch_unwind` with why it is safe and fails on
//! one it does not name. A session's code may not call `catching` at all.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// PATH, how many, why each is safe.
const SITES: &[(&str, usize, &str)] = &[
    ("crates/romacs-vm-gui/src/main.rs", 2,
     "`main` holds no VM; the evaluator thread's top: the unwinding drops the VM, and the session ends"),
    ("crates/romacs-vm/src/gc.rs", 2,
     "`Vm::catching` itself, and a mark handing its host back before re-raising"),
    ("crates/romacs-vm/src/gc_mark.rs", 4, "tests of a panic inside the collector, which nests no engine"),
    ("crates/romacs-vm/src/handle.rs", 1, "a test of a host panicking in an idle mark, which nests no engine"),
    ("crates/romacs-vm/src/prim.rs", 2, "tests of the declaration parser, with no VM"),
    ("crates/romacs-vm/src/twin.rs", 1,
     "a mark twin's forked child: its VM is a copy no other process uses, and it sends the panic as its report and ends (`exit_at_once`)"),
    ("crates/romacs-vm/tests/buffer_lifecycle.rs", 1, "a VM made in the closure, which the unwinding drops"),
    ("crates/romacs-vm/tests/bulk_core.rs", 1, "validates a corpus, with no VM"),
    ("crates/romacs-vm/tests/gnu_reader.rs", 1, "a test that asserts no panic came"),
    ("crates/romacs-vm/tests/gnutls.rs", 1, "the VM is on the thread the bound gave up on"),
    ("crates/romacs-vm/tests/support/bulk.rs", 1, "a VM made in the closure, which the unwinding drops"),
    ("gnu-tests/runner/src/main.rs", 7,
     "its import, and six that each report the panic and exit (`panic_exit`); what goes on after one uses `catching`"),
];

/// The primh tests' one form, repeated in each: a declaration, with no VM.
const VM_FREE: &str = "catch_unwind(|| validate_declaration(";

/// Packages a session's code lives in.
const SESSION: &[&str] = &["crates/romacs-vm-gui", "crates/romacs-layout"];

/// Compiles only while `Vm` is not `Send`: were it, both impls would apply and
/// the call would be ambiguous.
trait AmbiguousIfSend<Which> {
    fn check() {}
}
impl<T: ?Sized> AmbiguousIfSend<()> for T {}
impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
const _: fn() = || <romacs_vm::Vm as AmbiguousIfSend<_>>::check();

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2).expect("the repository").to_path_buf()
}

/// Every file under DIR named by WANTED, past build output, hidden directories
/// and symbolic links.
fn files_under(dir: &Path, wanted: &impl Fn(&Path) -> bool, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            if name != "target" && !name.starts_with('.') { files_under(&path, wanted, found); }
        } else if wanted(&path) {
            found.push(path);
        }
    }
}

fn says_word(text: &str, word: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric() && c != '-' && c != '_').any(|w| w == word)
}

/// The package directories that are romacs-vm or that a chain of manifests leads
/// to it from. A manifest "leads" when it says the name at all: a comment only
/// widens the search.
fn reaching_packages(root: &Path) -> Vec<PathBuf> {
    let mut manifests = Vec::new();
    files_under(root, &|path| path.ends_with("Cargo.toml"), &mut manifests);
    let manifests: Vec<(&Path, String, String)> = manifests.iter().filter_map(|path| {
        let text = std::fs::read_to_string(path).ok()?;
        let name = text.lines().find_map(|line| line.strip_prefix("name = \""))?.trim_end_matches('"').to_owned();
        Some((path.parent()?, name, text))
    }).collect();
    let mut names = vec!["romacs-vm".to_owned()];
    loop {
        let more: Vec<String> = manifests.iter()
            .filter(|(_, name, text)| !names.contains(name) && names.iter().any(|n| says_word(text, n)))
            .map(|(_, name, _)| name.clone())
            .collect();
        if more.is_empty() { break; }
        names.extend(more);
    }
    let mut packages: Vec<PathBuf> = manifests.iter()
        .filter(|(_, name, _)| names.contains(name))
        .map(|(dir, _, _)| dir.to_path_buf())
        .collect();
    packages.sort();
    packages
}

/// Each source file of PACKAGES but this one, relative to ROOT, with its comments out.
fn sources(root: &Path, packages: &[PathBuf]) -> Vec<(String, String)> {
    let mut files = Vec::new();
    for package in packages {
        files_under(package, &|path| path.extension().is_some_and(|e| e == "rs"), &mut files);
    }
    files.sort();
    files.dedup();
    files.iter().filter(|file| !file.ends_with("panic_boundary.rs")).filter_map(|file| {
        // What is not UTF-8 is not Rust the compiler would take.
        let text = std::fs::read_to_string(file).ok()?;
        let code: Vec<&str> = text.lines().filter(|line| !line.trim_start().starts_with("//")).collect();
        Some((file.strip_prefix(root).ok()?.to_string_lossy().into_owned(), code.join("\n")))
    }).collect()
}

#[test]
fn every_catch_is_named() {
    let root = root();
    let found: BTreeMap<String, usize> = sources(&root, &reaching_packages(&root)).into_iter()
        .map(|(path, code)| (path, code.replace(VM_FREE, "").matches("catch_unwind").count()))
        .filter(|&(_, n)| n > 0)
        .collect();
    let listed: BTreeMap<String, usize> = SITES.iter().map(|&(path, n, _)| (path.to_owned(), n)).collect();
    assert_eq!(found, listed, "a session never uses its VM after a panic (`Vm::catching`): \
        name each `catch_unwind` here with why it is safe, or go through `Vm::catching`");
}

#[test]
fn a_session_does_not_recover_from_a_panic() {
    let root = root();
    let session: Vec<PathBuf> = SESSION.iter().map(|package| root.join(package)).collect();
    let found: Vec<String> = sources(&root, &session).into_iter()
        .filter(|(_, code)| code.contains("catching("))
        .map(|(path, _)| path)
        .collect();
    assert!(found.is_empty(), "a session ends at a panic, and has no use for `Vm::catching`: {found:?}");
}
