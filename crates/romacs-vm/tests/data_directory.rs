//! `data-directory' from the build tree is a directory name that equals its
//! own expansion, as GNU's is, so a file name read under it compares equal
//! to `expand-file-name' of it (find-func-tests--library-completion,
//! bug#43393's test, completes "nx/" under it to "nxml/").
use romacs_vm::Vm;

#[test]
fn data_directory_is_its_own_expansion() {
    if std::env::var_os("ROMACS_RUNTIME_ROOT").is_some() {
        // An explicit runtime root is used as given, as GNU uses EMACSDATA.
        return;
    }
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let same = vm.eval_source("(list data-directory (equal data-directory (file-name-as-directory (expand-file-name data-directory))))").unwrap().display().unwrap();
    assert!(same.ends_with(" t)"), "{same}");
    assert!(!same.contains("/../"), "{same}");
}
