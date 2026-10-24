//! Real source-loader progress probe; no inherited evaluator is linked.
use romacs_vm::Vm;
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let Some(file) = args.first() else {
        eprintln!("usage: romacs-vm-load FILE [LOAD_PATH ...]");
        std::process::exit(2)
    };
    let mut vm = Vm::new();
    let paths = if args.len() > 1 {
        args[1..]
            .iter()
            .map(std::path::PathBuf::from)
            .collect::<Vec<_>>()
    } else {
        vec![std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../gnu-lisp")]
    };
    vm.set_load_path(&paths).unwrap();
    let result = vm.load_file(std::path::Path::new(file)).map(|_| ());
    let mut report = vm.load_report();
    report["file"] = file.clone().into();
    report["ok"] = result.is_ok().into();
    if let Err(error) = result {
        report["error"] = error.into();
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
    if report["ok"] != true {
        std::process::exit(1);
    }
}
