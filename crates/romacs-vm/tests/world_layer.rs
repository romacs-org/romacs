//! An init layer's build (`world::layer`): the plan's libraries load
//! after a fake chain, a definition that reads a per-process fact is kept
//! for restore and its variable tainted, any other form's such read, a
//! timer or a signal refuses its entry by name, and the startup function
//! runs again after them, reproducing what it computed before.
use romacs_vm::Vm;
use romacs_vm::host::{self, Pinned};
use romacs_vm::world::layer::{Entry, Kind, Plan};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

const PINNED: Pinned = Pinned { clock: UNIX_EPOCH, pid: 4242, seed: [1, 2, 3, 4] };

/// A fake loadup: its startup function records what it computed from the
/// state it found, and a `defcustom' that defines as `defvar' does.
const BOOTSTRAP: &str = "\
(defvar romacs--world-reseed-forms nil)
(defvar timer-list nil)
(defvar fence-out nil)
(defvar custom-delayed-init-variables nil)
(defmacro defcustom (symbol value &rest _) (list 'defvar symbol value))
(defun fake-startup ()
  (unwind-protect
      (progn (romacs--startup-phase t)
             (setq fence-out (if (boundp 'lib-g-flag) 'g 'none))
             (setq custom-delayed-init-variables t))
    (romacs--startup-phase nil)))
(defvar romacs-startup-function #'fake-startup)
(fake-startup)
";

const LIBRARIES: [(&str, &str); 10] = [
    // A definition reading a fenced fact, and one reading its variable.
    ("lib-a", "(defcustom lib-a-program (car exec-path) \"doc\")\n(defvar lib-a-derived (concat lib-a-program \"!\"))\n(provide 'lib-a)\n"),
    // A plain form reading a fenced fact.
    ("lib-b", "(defvar lib-b-ok 1)\n(setq lib-b-dir (expand-file-name \"x\"))\n(provide 'lib-b)\n"),
    ("lib-c", "(setq timer-list (cons 'fake timer-list))\n(provide 'lib-c)\n"),
    // A plain form reading a tainted variable.
    ("lib-d", "(setq lib-d-x lib-a-program)\n(provide 'lib-d)\n"),
    ("lib-e", "(error \"boom\")\n"),
    ("lib-f", "(defvar lib-f 1)\n(provide 'lib-f)\n"),
    // A library that changes what startup computes.
    ("lib-g", "(defvar lib-g-flag t)\n(provide 'lib-g)\n"),
    // A buffer-local definition reading a fenced fact, one reading it, and a plain form reading it.
    ("lib-h", "(defvar-local lib-h-local (car exec-path))\n(defvar lib-h-from-local (list lib-h-local))\n(defvar lib-h-in-buffer (buffer-local-value 'lib-h-local (current-buffer)))\n(provide 'lib-h)\n"),
    ("lib-i", "(setq lib-i-x (buffer-local-value 'lib-h-local (current-buffer)))\n(provide 'lib-i)\n"),
    // A definition an effect cuts short.
    ("lib-j", "(defvar-local lib-j-dir (let ((d \"/nonexistent-romacs-layer/x\")) (make-directory-internal d) d))\n(defvar lib-j-after lib-j-dir)\n(provide 'lib-j)\n"),
];

fn scratch(name: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("romacs-world-layer-{}-{unique}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rlisp")).unwrap();
    std::fs::create_dir_all(dir.join("lisp")).unwrap();
    for (name, text) in LIBRARIES {
        std::fs::write(dir.join(format!("lisp/{name}.el")), text).unwrap();
    }
    std::fs::write(dir.join("init.el"), ";; the init\n").unwrap();
    dir
}

fn plan(dir: &Path, names: &[&str]) -> Plan {
    let entries = names.iter().map(|name| Entry { kind: Kind::Require, name: (*name).to_owned(), file: None, noerror: false, load_path: 0 }).collect();
    Plan::new(&dir.join("init.el"), entries, vec![vec![dir.join("lisp").display().to_string()]]).unwrap()
}

/// Build with NAMES' libraries as the layer: the VM, the refused entries
/// and the trace.
fn build(name: &str, names: &[&str]) -> (Vm, Vec<(usize, String)>, Json) {
    let dir = scratch(name);
    std::fs::write(dir.join("rlisp/romacs-loadup-bootstrap.el"), BOOTSTRAP).unwrap();
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    vm.load_file(&dir.join("rlisp/romacs-loadup-bootstrap.el")).expect("bootstrap");
    let refused = vm.world_layer_load(&plan(&dir, names));
    let trace = host::leave_build().unwrap();
    (vm, refused, trace)
}

fn eval(vm: &mut Vm, source: &str) -> String {
    vm.eval_source(source).and_then(|v| v.display()).unwrap()
}

fn refusals<'a>(trace: &'a Json, kind: &str) -> Vec<&'a Json> {
    trace["refusals"].as_array().unwrap().iter().filter(|r| r["kind"] == kind).collect()
}

#[test]
fn definitions_that_read_the_process_are_kept_for_restore_and_other_forms_refuse_their_entry() {
    let (mut vm, refused, trace) = build("entries", &["lib-a", "lib-b", "lib-c", "lib-d", "lib-e", "lib-f"]);
    let refused: Vec<(usize, &str)> = refused.iter().map(|(at, why)| (*at, why.split(':').next().unwrap())).collect();
    assert_eq!(refused, [(1, "fence"), (2, "timer"), (3, "fence"), (4, "error")], "{}", trace["refusals"]);
    // The build names each refused entry, and lib-d's read names the tainted root.
    let named: Vec<u64> = refusals(&trace, "init-layer").iter().map(|r| r["entry"].as_u64().unwrap()).collect();
    assert_eq!(named, [1, 2, 3, 4]);
    let fences: Vec<(u64, String)> = refusals(&trace, "fence").iter()
        .filter_map(|r| Some((r["root"].as_u64()?, r["symbol"].as_str()?.to_owned()))).collect();
    assert!(fences.contains(&(2, "default-directory".into())), "{fences:?}");
    assert!(fences.contains(&(7, "lib-a-program".into())), "{fences:?}");
    // lib-a's definitions are kept, in load order once reversed, with their forms.
    assert_eq!(eval(&mut vm, "(mapcar (lambda (e) (list (car e) (car (cdr e)) (car (nth 2 e)))) (reverse romacs--world-reseed-forms))"),
               "((defcustom lib-a-program defcustom) (defvar lib-a-derived defvar))");
    assert_eq!(eval(&mut vm, "(nth 2 (nth 2 (car romacs--world-reseed-forms)))"), "(concat lib-a-program \"!\")");
    let kept: Vec<&str> = trace["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "layer-reseed")
        .map(|r| r["symbol"].as_str().unwrap()).collect();
    assert_eq!(kept, ["lib-a-program", "lib-a-derived"]);
    // The libraries loaded, lib-f too, and the startup function ran again after them.
    assert_eq!(eval(&mut vm, "(list (featurep 'lib-a) (featurep 'lib-f) lib-f fence-out)"), "(t t 1 none)");
    let runs = trace["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "startup" && r["on"] == true).count();
    assert_eq!(runs, 2, "{trace}");
    assert!(!refusals(&trace, "fence").iter().any(|r| r["access"] == "startup-output"), "{trace}");
}

#[test]
fn a_tainted_buffer_local_definition_is_fenced_and_a_cut_definition_is_kept_whole() {
    let (mut vm, refused, trace) = build("locals", &["lib-h", "lib-i", "lib-j"]);
    let refused: Vec<(usize, &str)> = refused.iter().map(|(at, why)| (*at, why.split(':').next().unwrap())).collect();
    assert_eq!(refused, [(1, "fence")], "{}", trace["refusals"]);
    // lib-h's buffer-local definition and its reader are kept; lib-i's plain read of it refuses.
    let kept = eval(&mut vm, "(mapcar (lambda (e) (list (car e) (car (cdr e)) (nth 3 e))) (reverse romacs--world-reseed-forms))");
    assert_eq!(kept, "((defvar-local lib-h-local nil) (defvar lib-h-from-local nil) (defvar lib-h-in-buffer nil) (defvar-local lib-j-dir t) (defvar lib-j-after nil))");
    assert_eq!(eval(&mut vm, "(list (local-variable-if-set-p 'lib-h-local) lib-j-dir lib-j-after)"), "(t nil nil)");
    let effects: Vec<&str> = trace["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "layer-effect")
        .map(|r| r["what"].as_str().unwrap()).collect();
    assert_eq!(effects.len(), 1, "{effects:?}");
    assert!(effects[0].starts_with("make directory"), "{effects:?}");
    assert!(refusals(&trace, "effect").is_empty(), "{trace}");
}

#[test]
fn a_library_that_changes_what_startup_computes_refuses_the_build() {
    let (mut vm, refused, trace) = build("outputs", &["lib-f", "lib-g"]);
    assert!(refused.is_empty(), "{refused:?}");
    assert_eq!(eval(&mut vm, "fence-out"), "g");
    let changed: Vec<&str> = refusals(&trace, "fence").iter().filter(|r| r["access"] == "startup-output")
        .map(|r| r["symbol"].as_str().unwrap()).collect();
    assert_eq!(changed, ["fence-out"]);
    assert!(refusals(&trace, "init-layer").is_empty(), "no entry is named: {trace}");
}
