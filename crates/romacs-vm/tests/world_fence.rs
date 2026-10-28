//! The P fence of a world build (DESIGN-M1 §6.2, E5): outside startup, a
//! read or write of a value restore rebuilds refuses the build, naming the
//! root and the site; a `let` is exempt, the value it saved is not (F2).
//! Without build mode the same forms behave exactly as before.
use romacs_vm::Vm;
use romacs_vm::host::{self, Pinned};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

const PINNED: Pinned = Pinned { clock: UNIX_EPOCH, pid: 4242, seed: [1, 2, 3, 4] };

/// A fake loadup whose startup sets two delayed variables from its inputs:
/// a variable, a property, a plist and a function. A built-in variable is
/// delayed too, and one is read through `buffer-local-value'.
const BOOTSTRAP: &str = "\
(defvar custom-delayed-init-variables '(fence-a fence-b transient-mark-mode))
(defvar fence-a nil)
(defvar fence-b nil)
(defvar fence-input \"/in/\")
(defun fence-locate () fence-input)
(put 'fence-a 'standard-value (list \"std\"))
(put 'fence-plist 'p 1)
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq fence-a (concat (fence-locate) (car (get 'fence-a 'standard-value))) fence-b (symbol-plist 'fence-plist))
           (setq startup-saw (list (length process-environment) default-directory (random 3)
                                   (buffer-local-value 'mark-even-if-inactive (current-buffer))))
           (setq custom-delayed-init-variables t))
  (romacs--startup-phase nil))
";

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("romacs-world-fence-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("rlisp")).unwrap();
    dir
}

fn write(path: &Path, text: &str) -> PathBuf {
    std::fs::write(path, text).unwrap();
    path.to_owned()
}

/// Load the fake loadup, then PRELOAD (one case per line), with build mode
/// on when BUILDING: the VM and the trace.
fn run(name: &str, preload: &str, building: bool) -> (Vm, Option<Json>) {
    run_with(name, BOOTSTRAP, preload, building)
}

fn run_with(name: &str, bootstrap: &str, preload: &str, building: bool) -> (Vm, Option<Json>) {
    let dir = scratch(name);
    let bootstrap = write(&dir.join("rlisp/romacs-loadup-bootstrap.el"), bootstrap);
    let preload = write(&dir.join("preload.el"), preload);
    if building {
        host::enter_build(PINNED);
    }
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    for file in [bootstrap, preload] {
        vm.load_file(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    }
    (vm, host::leave_build())
}

/// The fence refusals as (line, root, access, symbol).
fn fences(trace: &Json) -> Vec<(u64, u64, String, String)> {
    trace["refusals"].as_array().unwrap().iter().filter(|r| r["kind"] == "fence")
        .map(|r| (r["site"]["line"].as_u64().unwrap(), r["root"].as_u64().unwrap(),
                  r["access"].as_str().unwrap().to_owned(), r["symbol"].as_str().unwrap().to_owned()))
        .collect()
}

fn lines(trace: &Json) -> Vec<u64> {
    let mut lines: Vec<u64> = trace["refusals"].as_array().unwrap().iter().map(|r| r["site"]["line"].as_u64().unwrap()).collect();
    lines.dedup();
    lines
}

/// Each line refuses with ROOT, ACCESS and SYMBOL; the trace has no other
/// refusal.
fn assert_refused(cases: &[(&str, u64, &str, &str)]) {
    let preload: String = cases.iter().map(|(form, ..)| format!("{form}\n")).collect();
    let (_, trace) = run("refused", &preload, true);
    let trace = trace.unwrap();
    let found = fences(&trace);
    for (line, (form, root, access, symbol)) in (1..).zip(cases) {
        assert!(found.contains(&(line, *root, (*access).into(), (*symbol).into())), "{form}: {found:?}");
    }
    assert_eq!(lines(&trace), (1..=cases.len() as u64).collect::<Vec<_>>(), "{trace}");
}

#[test]
fn reads_of_fenced_values_refuse_compiled_and_interpreted() {
    assert_refused(&[
        ("(defvar x1 (length fence-a))", 3, "read", "fence-a"),
        ("(symbol-value 'fence-a)", 3, "read", "fence-a"),
        ("(boundp 'fence-b)", 3, "read", "fence-b"),
        ("(default-value 'fence-a)", 3, "read", "fence-a"),
        ("(funcall (eval '(lambda () fence-a) nil))", 3, "read", "fence-a"),
        ("(eval 'fence-a nil)", 3, "read", "fence-a"),
        ("(buffer-local-value 'fence-a (current-buffer))", 3, "read", "fence-a"),
        ("(buffer-local-value 'transient-mark-mode (current-buffer))", 3, "read", "transient-mark-mode"),
        ("(defvar x2 (length process-environment))", 1, "read", "process-environment"),
        ("(let ((n 0)) (dolist (e initial-environment) (setq n (1+ n))))", 1, "read", "initial-environment"),
        ("(expand-file-name \"foo\")", 2, "read", "default-directory"),
        ("(let ((default-directory default-directory)) nil)", 2, "read", "default-directory"),
        ("(defvar x3 (random 100))", 5, "read", "random state"),
    ]);
}

#[test]
fn writes_of_fenced_values_refuse() {
    assert_refused(&[
        ("(setq fence-a \"/x\")", 3, "write", "fence-a"),
        ("(set 'fence-a \"/x\")", 3, "write", "fence-a"),
        ("(makunbound 'fence-b)", 3, "write", "fence-b"),
        ("(set-default 'fence-a 1)", 3, "write", "fence-a"),
        ("(make-local-variable 'fence-a)", 3, "write", "fence-a"),
        ("(setq process-environment nil)", 1, "write", "process-environment"),
        ("(setq default-directory \"/\")", 2, "write", "default-directory"),
    ]);
}

#[test]
fn the_value_a_let_saved_stays_fenced() {
    assert_refused(&[
        ("(let ((fence-a nil)) (default-toplevel-value 'fence-a))", 3, "saved", "fence-a"),
        ("(let ((fence-a nil)) (set-default-toplevel-value 'fence-a \"/x\"))", 3, "saved", "fence-a"),
        ("(let ((fence-a nil)) (let ((fence-a 1)) (default-toplevel-value 'fence-a)))", 3, "saved", "fence-a"),
        ("(let ((fence-a nil)) (defvar fence-a \"a@b\"))", 3, "saved", "fence-a"),
        ("(let ((default-directory \"/\")) (buffer-local-toplevel-value 'default-directory))", 2, "saved", "default-directory"),
        ("(let ((default-directory \"/\")) (set-buffer-local-toplevel-value 'default-directory \"/x/\"))", 2, "saved", "default-directory"),
    ]);
}

#[test]
fn lets_buffers_and_startup_pass() {
    let preload = "\
(setq p1 (let ((fence-a \"/l/\")) (list fence-a (symbol-value 'fence-a))))
(setq p2 (let ((fence-a nil)) (setq fence-a 2) fence-a))
(setq p3 (let ((default-directory \"/\")) (expand-file-name \"x\")))
(setq p4 (let ((default-directory \"/\")) (let ((b (current-buffer))) (save-current-buffer (set-buffer (get-buffer-create \" other\")) (set-buffer b) (expand-file-name \"y\")))))
(setq p5 (buffer-name (get-buffer-create \"fence-new\")))
(setq p6 (getenv-internal \"HOME\"))
";
    let (mut vm, trace) = run("pass", preload, true);
    let trace = trace.unwrap();
    assert!(trace["refusals"].as_array().unwrap().is_empty(), "{trace}");
    let shown = vm.eval_source("(list p1 p2 p3 p4 p5 (stringp p6) (length startup-saw))").and_then(|v| v.display()).unwrap();
    assert_eq!(shown, "((\"/l/\" \"/l/\") 2 \"/x\" \"/y\" \"fence-new\" t 4)");
}

#[test]
fn writes_to_startup_inputs_refuse_whatever_they_store() {
    let preload = "\
(setq fence-input \"/x/\")
(setq fence-input \"/in/\")
(put 'fence-a 'standard-value (list \"/y\"))
(fset 'fence-locate (lambda () nil))
(defalias 'fence-locate (lambda () nil))
(setplist 'fence-plist nil)
(setq ok1 fence-input)
(setq ok2 (let ((fence-input \"/x/\")) fence-input))
(put 'fence-a 'other 1)
(setq ok3 (get 'fence-a 'standard-value))
(makunbound 'fence-input)
";
    let (_, trace) = run("inputs", preload, true);
    let trace = trace.unwrap();
    let refused: Vec<_> = trace["refusals"].as_array().unwrap().iter()
        .map(|r| (r["site"]["line"].as_u64().unwrap(), r["root"].as_u64().unwrap(), r["access"].as_str().unwrap().to_owned(), r["symbol"].as_str().unwrap().to_owned()))
        .collect();
    let expected: Vec<(u64, u64, String, String)> = [(1, "fence-input"), (2, "fence-input"), (3, "fence-a"), (4, "fence-locate"), (5, "fence-locate"), (6, "fence-plist"), (11, "fence-input")]
        .into_iter().map(|(line, symbol)| (line, 6, "write".to_owned(), symbol.to_owned())).collect();
    assert_eq!(refused, expected, "{trace}");
    let inputs: Vec<_> = trace["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "input")
        .map(|r| format!("{} {} {}", r["input"].as_str().unwrap(), r["symbol"].as_str().unwrap(), r["property"].as_str().unwrap_or(""))).collect();
    for input in ["variable fence-input ", "property fence-a standard-value", "plist fence-plist ", "function fence-locate ", "variable mark-even-if-inactive "] {
        assert!(inputs.iter().any(|i| i == input), "{input}: {inputs:?}");
    }
}

#[test]
fn an_input_changed_in_place_refuses_at_the_content_check() {
    let dir = scratch("in-place");
    let bootstrap = write(&dir.join("rlisp/romacs-loadup-bootstrap.el"), BOOTSTRAP);
    let preload = write(&dir.join("preload.el"), "(setcar (get 'fence-a 'standard-value) \"/y\")\n");
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    for file in [bootstrap, preload] {
        vm.load_file(&file).unwrap();
    }
    assert!(host::leave_build().unwrap()["refusals"].as_array().unwrap().is_empty());
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let refused: Vec<_> = trace["refusals"].as_array().unwrap().iter()
        .map(|r| (r["access"].as_str().unwrap(), r["input"].as_str().unwrap(), r["symbol"].as_str().unwrap(), r["property"].as_str().unwrap())).collect();
    assert_eq!(refused, [("changed", "property", "fence-a", "standard-value")], "{trace}");
}

#[test]
fn fenced_variables_behave_as_without_the_fence() {
    let probes = "\
(setq r1 (list fence-a (symbol-value 'fence-a) (default-value 'fence-a) (boundp 'fence-b)))
(setq r2 (let ((fence-a 'l)) (list fence-a (default-toplevel-value 'fence-a))))
(setq r3 (progn (setq fence-b 7) (set-default 'fence-b (1+ fence-b)) fence-b))
(setq r4 (progn (makunbound 'fence-b) (list (boundp 'fence-b) (default-boundp 'fence-b))))
(setq r5 (let ((default-directory \"/d/\")) (list default-directory (expand-file-name \"z\"))))
(setq r6 (list (stringp default-directory) (consp process-environment) (length exec-path)))
(setq r7 (progn (make-local-variable 'fence-a) (setq fence-a 'local) (list fence-a (default-value 'fence-a) (local-variable-p 'fence-a))))
";
    let show = "(list r1 r2 r3 r4 r5 r6 r7 fence-a (default-value 'fence-a))";
    let (mut off, none) = run("same-off", probes, false);
    assert!(none.is_none());
    let (mut on, trace) = run("same-on", probes, true);
    assert!(!trace.unwrap()["refusals"].as_array().unwrap().is_empty());
    let off = off.eval_source(show).and_then(|v| v.display()).unwrap();
    let on = on.eval_source(show).and_then(|v| v.display()).unwrap();
    assert_eq!(on, off);
}

/// The refusals as (kind, line, symbol, access, object).
fn listed(trace: &Json) -> Vec<(String, u64, String, String, String)> {
    let text = |r: &Json, key: &str| r[key].as_str().unwrap_or("").to_owned();
    trace["refusals"].as_array().unwrap().iter()
        .map(|r| (text(r, "kind"), r["site"]["line"].as_u64().unwrap_or(0), text(r, "symbol"), text(r, "access"), text(r, "object")))
        .collect()
}

#[test]
fn the_content_check_sees_char_tables_and_text_properties() {
    let bootstrap = "\
(defvar custom-delayed-init-variables nil)
(defvar fence-table (make-char-table 'test))
(defvar fence-text (copy-sequence \"abc\"))
(defvar fence-same (make-char-table 'test 'v))
(defvar fence-buffer (get-buffer-create \" fence\"))
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq seen (list fence-table fence-text fence-same fence-buffer)))
  (romacs--startup-phase nil))
";
    let preload = "\
(set-char-table-range fence-table '(?a . ?c) 'x)
(put-text-property 0 1 'face 'bold fence-text)
(set-char-table-range fence-same '(?a . ?c) 'v)
";
    let (mut vm, trace) = run_with("content", bootstrap, preload, true);
    let trace = trace.unwrap();
    // A buffer's state cannot be digested: it refuses as startup ends.
    assert_eq!(listed(&trace), [("undigestable".into(), 6, "fence-buffer".into(), String::new(), "buffer".into())], "{trace}");
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let changed: Vec<_> = listed(&trace).into_iter().map(|(kind, _, symbol, access, _)| (kind, symbol, access)).collect();
    assert_eq!(changed, [("fence".into(), "fence-table".into(), "changed".into()), ("fence".into(), "fence-text".into(), "changed".into())], "{trace}");
}

/// Each in-place write changes what `aref`, `char-table-range`, the parent
/// or an extra slot reads; one table per field. A literal's ASCII slot is
/// a node detached from the root, so `aset' below 128 writes only it.
#[test]
fn the_content_check_sees_every_char_table_field() {
    let ascii = format!("#^[nil nil test #^^[3 0{}]{}]", " a".repeat(128), " b".repeat(64));
    let bootstrap = format!("\
(defvar custom-delayed-init-variables nil)
(put 'fence-purpose 'char-table-extra-slots 1)
(defvar fence-ascii {ascii})
(defvar fence-default (make-char-table 'test 'v))
(defvar fence-parent (make-char-table 'test))
(defvar fence-child (make-char-table 'test))
(set-char-table-parent fence-child fence-parent)
(defvar fence-reparent (make-char-table 'test))
(defvar fence-extra (make-char-table 'fence-purpose 'v))
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq seen (list fence-ascii fence-default fence-child fence-reparent fence-extra)))
  (romacs--startup-phase nil))
");
    let preload = "\
(aset fence-ascii 65 'z)
(set-char-table-range fence-default nil 'w)
(aset fence-parent 65 'q)
(set-char-table-parent fence-reparent (make-char-table 'test 'w))
(set-char-table-extra-slot fence-extra 0 'x)
";
    let (mut vm, trace) = run_with("char-table-fields", &bootstrap, preload, true);
    assert_eq!(listed(&trace.unwrap()), []);
    let probes = "(list (aref fence-ascii 65) (aref fence-ascii 128) (char-table-range fence-default nil) (aref fence-child 65))";
    assert_eq!(vm.eval_source(probes).and_then(|v| v.display()).unwrap(), "(z b w q)");
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let mut changed: Vec<_> = listed(&trace).into_iter().map(|(kind, _, symbol, access, _)| (kind, symbol, access)).collect();
    changed.sort();
    let expected = ["fence-ascii", "fence-child", "fence-default", "fence-extra", "fence-reparent"]
        .map(|symbol| ("fence".to_owned(), symbol.to_owned(), "changed".to_owned()));
    assert_eq!(changed, expected, "{trace}");
}

#[test]
fn a_binding_across_startups_end_stays_fenced_in_backtraces() {
    let bootstrap = "\
(defvar custom-delayed-init-variables '(fence-a transient-mark-mode))
(defvar fence-a nil)
(defvar fence-seen nil)
(defun fence-locals () (setq fence-seen (cons (backtrace--locals 2) fence-seen)))
(defun fence-run ()
  (let ((transient-mark-mode 'outer) (fence-a \"/outer/\"))
    (unwind-protect
        (progn (romacs--startup-phase t) (setq custom-delayed-init-variables t))
      (romacs--startup-phase nil))
    (let ((fence-a 'inner)) (fence-locals))))
(fence-run)
";
    let preload = "\
(defun fence-new () (let ((fence-a 'new)) (fence-locals)))
(fence-new)
";
    let (mut vm, trace) = run_with("backtrace", bootstrap, preload, true);
    let trace = trace.unwrap();
    // The current binding of the built-in variable and the value the inner
    // `let' saved are both the root's; the newer `let's are their own.
    assert_eq!(listed(&trace), [
        ("fence".into(), 11, "transient-mark-mode".into(), "saved".into(), String::new()),
        ("fence".into(), 11, "fence-a".into(), "saved".into(), String::new()),
    ], "{trace}");
    let (mut off, _) = run_with("backtrace-off", bootstrap, preload, false);
    let shown = "(format \"%S\" fence-seen)";
    let seen = vm.eval_source(shown).and_then(|v| v.display()).unwrap();
    assert_eq!(seen, "\"(((fence-a . new)) ((transient-mark-mode . outer) (fence-a . \\\"/outer/\\\") (fence-a . inner)))\"");
    assert_eq!(seen, off.eval_source(shown).and_then(|v| v.display()).unwrap());
}
