use romacs_vm::Vm;
use std::path::PathBuf;

#[test]
fn macroexp_cold_frontier_advances_past_cadr_without_preloading_subr() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("gnu-lisp/emacs-lisp/macroexp.el").canonicalize().unwrap();
    let mut vm = Vm::new();
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")]).unwrap();
    let failure = vm.load_file(&path).unwrap_err();
    let report = vm.load_report();
    let step = report["steps"].as_array().unwrap().iter()
        .find(|step| step["file"] == path.to_string_lossy().as_ref() && step["ordinal"] == 6).unwrap();
    // The formerly failing form must now finish, not merely name another fault.
    assert_eq!(step["status"], "ok", "{failure}");
    // RATCHET, re-measured: form 25 (macroexp--unfold-lambda, whose body uses
    // pcase before pcase.el exists) now defines, as in GNU's interpreter.
    // Cold, byte-run's defsubst is absent, so form 39 evaluates its name.
    assert_eq!(report["completed_forms"], 38);
    let frontier = report["steps"].as_array().unwrap().iter()
        .find(|step| step["file"] == path.to_string_lossy().as_ref() && step["ordinal"] == 39).unwrap();
    assert_eq!(frontier["error"], "(void-function defsubst)", "{failure}");
}
