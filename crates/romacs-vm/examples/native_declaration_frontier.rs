use romacs_vm::Vm;
fn main() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let target = std::env::args().nth(1).expect("target");
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.set_load_path(&[root.join("gnu-lisp"), root.join("gnu-lisp/emacs-lisp")])
        .unwrap();
    let mut files = vec![
        "emacs-lisp/backquote.el",
        "emacs-lisp/byte-run.el",
        "subr.el",
        "emacs-lisp/macroexp.el",
        "emacs-lisp/pcase.el",
        "emacs-lisp/gv.el",
        "emacs-lisp/inline.el",
        "keymap.el",
    ];
    files.push(&target);
    for file in files {
        let path = root.join("gnu-lisp").join(file);
        let result = vm.load_file(&path).map(drop);
        println!(
            "{}",
            serde_json::json!({"file": file, "ok": result.is_ok(), "error": result.as_ref().err(), "report": vm.load_report()})
        );
        if !result.is_ok() {
            break;
        }
    }
}
