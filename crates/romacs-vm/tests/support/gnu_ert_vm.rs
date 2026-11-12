//! A VM booted as the gnu-ert runner boots it: GNU batch's terminal
//! frame and load path, then BOOTSTRAP, help-fns, cl-print and ERT.
use romacs_vm::Vm;
use std::path::Path;

pub fn bootstrapped_with(root: &Path, bootstrap: &str) -> Vm {
    let mut vm = Vm::new();
    // GNU batch, as the gnu-ert runner sets it before the chain loads:
    // noninteractive, on its 80x25 terminal frame of 1x1 characters.
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    // GNU's installed load-path (test/gnu/run.py load_path): the Lisp root,
    // its subdirectories in reverse name order with obsolete last, then lisp/romacs.
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("lisp/romacs"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    for name in [bootstrap, "lisp/gnu/help-fns.el", "lisp/romacs/romacs-cl-print.el", "lisp/gnu/emacs-lisp/ert.el"] {
        if let Err(error) = vm.load_file(&root.join(name)).map(|_| ()) {
            let report = vm.load_report();
            let failed = report["steps"].as_array().unwrap().iter()
                .filter(|step| step["status"] != "ok").collect::<Vec<_>>();
            panic!("{name}: {error}\n{failed:?}");
        }
    }
    vm
}
