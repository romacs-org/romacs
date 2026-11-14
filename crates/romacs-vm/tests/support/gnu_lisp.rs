//! A VM with GNU's Lisp, booted as GNU batch: its terminal frame, its
//! installed load path, the prelude and the loadup bootstrap.
use romacs_vm::Vm;
use std::path::{Path, PathBuf};

/// GNU batch's VM after the loadup bootstrap, with each file of THEN (a
/// path under the checkout) loaded after it.
pub fn gnu_lisp(then: &[&str]) -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    // GNU batch, as the gnu-ert runner sets it before the chain loads:
    // noninteractive, on its 80x25 terminal frame of 1x1 characters.
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
    vm.set_load_path(&load_path(&root)).unwrap();
    vm.install_lisp_prelude().unwrap();
    for name in std::iter::once(&"lisp/romacs/romacs-loadup-bootstrap.el").chain(then) {
        if let Err(error) = vm.load_file(&root.join(name)) {
            let report = vm.load_report();
            let failed = report["steps"].as_array().unwrap().iter()
                .filter(|step| step["status"] != "ok").collect::<Vec<_>>();
            panic!("{name}: {error}\n{failed:?}");
        }
    }
    vm
}

/// GNU's installed load-path (test/gnu/run.py load_path): the Lisp root,
/// its subdirectories in reverse name order with obsolete last, then
/// lisp/romacs.
fn load_path(root: &Path) -> Vec<PathBuf> {
    let lisp = root.join("lisp/gnu");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let (obsolete, rest): (Vec<&String>, Vec<&String>) = names.iter().rev().partition(|n| *n == "obsolete");
    std::iter::once(lisp.clone())
        .chain(rest.into_iter().chain(obsolete).map(|n| lisp.join(n)))
        .chain(std::iter::once(root.join("lisp/romacs")))
        .collect()
}
