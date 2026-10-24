//! Measurement-only strict ordered source loader. Never advances after a failure.
use romacs_vm::Vm;
use std::{io::Write, path::PathBuf};
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = PathBuf::from(&args[0]);
    let mut vm = Vm::new();
    vm.set_load_path(&[
        root.join("lisp"),
        root.join("lisp/emacs-lisp"),
        root.join("lisp/progmodes"),
        root.join("lisp/textmodes"),
    ])
    .unwrap();
    vm.install_lisp_prelude().unwrap();
    for name in &args[1..] {
        if let Some(source) = name.strip_prefix("--eval=") {
            let result = vm.eval_source(source).and_then(|v| v.display());
            println!("{}", serde_json::json!({"eval": source, "result": result}));
            std::io::stdout().flush().unwrap();
            if result.is_err() {
                std::process::exit(2);
            }
            continue;
        }
        let result = vm.load_file(&root.join(name)).map(|_| ());
        let ok = result.is_ok();
        println!(
            "{}",
            serde_json::json!({"file": name, "ok": ok, "error": result.err(), "report": vm.load_report()})
        );
        std::io::stdout().flush().unwrap();
        if !ok {
            std::process::exit(1);
        }
    }
    for symbol in [
        "find-file",
        "find-file-noselect",
        "command-execute",
        "execute-extended-command",
    ] {
        let result = vm
            .eval_source(&format!("(fboundp '{symbol})"))
            .and_then(|v| v.display());
        println!(
            "{}",
            serde_json::json!({"symbol": symbol, "fboundp": result})
        );
    }
}
