//! Strict dependency bootstrap probe: stop and discard the VM on any failed file.
use romacs_vm::Vm;
use std::{io::Write, path::PathBuf};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(&args[0]);
    let mut vm = Vm::new();
    vm.set_load_path(&[root.join("lisp/gnu"), root.join("lisp/gnu/emacs-lisp"), root.join("lisp/gnu/progmodes"), root.join("lisp/gnu/textmodes")]).unwrap();
    vm.install_lisp_prelude().unwrap();
    for name in &args[1..] {
        let path = root.join(name);
        let result = vm.load_file(&path).map(|_| ());
        let ok = result.is_ok();
        println!("{}", serde_json::json!({"file": name, "ok": ok, "error": result.err(), "report": vm.load_report()}));
        std::io::stdout().flush().unwrap();
        if !ok { std::process::exit(1); }
    }
    let result = vm.eval_source(include_str!("../../../tools/lib/ert-closure-run.el"))
        .and_then(|v| v.display());
    let ok = result.is_ok();
    println!("{}", serde_json::json!({"ert": result}));
    if !ok { std::process::exit(1); }
}
