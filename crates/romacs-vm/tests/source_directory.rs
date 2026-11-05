//! GNU lread.c:5724-5729 derives source-directory from the build-time
//! PATH_DUMPLOADSEARCH (the source tree's lisp directory), not the runtime
//! cwd.  Romacs's names the Romacs source tree only while there is one: the
//! build's root while it is a git checkout (a gate's build slot is a mirror
//! without `.git'), else nil (ROMACS_SOURCE_DIRECTORY aside, which
//! tests/definition_files.rs exercises in the GUI binary).
use romacs_vm::Vm;
use std::path::Path;

fn shown(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).unwrap().display().unwrap()
}

/// The value as printed: the build root, or nil in a checkout's mirror.
fn build_source_directory() -> String {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap();
    assert!(root.is_absolute());
    if root.join(".git").symlink_metadata().is_err() { return "nil".into() }
    serde_json::to_string(&format!("{}/", root.to_str().unwrap())).unwrap()
}

#[test]
fn source_directory_is_the_build_source_root_and_materializes_lazily() {
    let mut vm = Vm::new();
    assert_eq!(vm.heap_objects(), 0, "source-directory must not allocate at startup");
    let expected = build_source_directory();
    assert_eq!(shown(&mut vm, &format!("(equal source-directory {expected})")), "t");
    assert_eq!(shown(&mut vm, "(list (boundp 'source-directory) (special-variable-p 'source-directory) (multibyte-string-p source-directory) (eq source-directory (default-value 'source-directory)))"), "(t t nil t)");
    assert_eq!(shown(&mut vm, "(setq source-directory-saved source-directory) (eq source-directory-saved source-directory)"), "t");
    vm.collect_garbage();
    assert_eq!(shown(&mut vm, &format!("(equal source-directory {expected})")), "t");
    assert_eq!(shown(&mut vm, "(eq source-directory-saved source-directory)"), "t");
}

#[test]
fn source_directory_is_special_before_first_access_and_restores_its_default() {
    let mut vm = Vm::new();
    assert_eq!(shown(&mut vm, "(let ((source-directory \"/bound/\")) (list source-directory (default-value 'source-directory)))"), "(\"/bound/\" \"/bound/\")");
    let expected = build_source_directory();
    assert_eq!(shown(&mut vm, &format!("(equal source-directory {expected})")), "t");
    assert_eq!(shown(&mut vm, "(set-default 'source-directory \"/changed/\") (let ((source-directory \"/bound/\")) source-directory) source-directory"), "\"/changed/\"");
    assert_eq!(shown(&mut vm, "(condition-case e (makunbound 'source-directory) (error (car e)))"), "error");
    assert_eq!(shown(&mut vm, "source-directory"), "\"/changed/\"");
}
