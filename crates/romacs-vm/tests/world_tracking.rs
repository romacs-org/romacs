//! The P tracking of a world build (DESIGN-M1 §6.2, E5): outside startup, a
//! read or write of a value restore rebuilds is not captured, naming the
//! root and the site; a `let` is exempt, the value it saved is not (F2).
//! Without build mode the same forms behave exactly as before.
use romacs_vm::Vm;
use romacs_vm::host::{self, Pinned};
use serde_json::Value as Json;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

const PINNED: Pinned = Pinned { clock: UNIX_EPOCH, pid: 4242, seed: [1, 2, 3, 4] };

/// A fake loadup whose startup sets two delayed variables from its inputs:
/// a variable, a property, a plist and a function. A built-in variable is
/// delayed too, and one is read through `buffer-local-value'.
const BOOTSTRAP: &str = "\
(defvar custom-delayed-init-variables '(tracking-a tracking-b transient-mark-mode))
(defvar tracking-a nil)
(defvar tracking-b nil)
(defvar tracking-input \"/in/\")
(defun tracking-locate () tracking-input)
(put 'tracking-a 'standard-value (list \"std\"))
(put 'tracking-plist 'p 1)
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq tracking-a (concat (tracking-locate) (car (get 'tracking-a 'standard-value))) tracking-b (symbol-plist 'tracking-plist))
           (setq startup-saw (list (length process-environment) default-directory (random 3)
                                   (buffer-local-value 'mark-even-if-inactive (current-buffer))))
           (setq custom-delayed-init-variables t))
  (romacs--startup-phase nil))
";

fn scratch(name: &str) -> PathBuf {
    // libtest runs a binary's tests as threads of one process: the pid
    // alone would let two tests share, and remove, one directory.
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let unique = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("romacs-world-tracking-{}-{unique}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("lisp/session")).unwrap();
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
    let bootstrap = write(&dir.join("lisp/session/romacs-loadup-bootstrap.el"), bootstrap);
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

/// The tracking notes as (line, root, access, symbol).
fn tracks(trace: &Json) -> Vec<(u64, u64, String, String)> {
    trace["uncaptured"].as_array().unwrap().iter().filter(|r| r["kind"] == "per-process")
        .map(|r| (r["site"]["line"].as_u64().unwrap(), r["root"].as_u64().unwrap(),
                  r["access"].as_str().unwrap().to_owned(), r["symbol"].as_str().unwrap().to_owned()))
        .collect()
}

fn lines(trace: &Json) -> Vec<u64> {
    let mut lines: Vec<u64> = trace["uncaptured"].as_array().unwrap().iter().map(|r| r["site"]["line"].as_u64().unwrap()).collect();
    lines.dedup();
    lines
}

/// Each line is noted with ROOT, ACCESS and SYMBOL; the trace has no other
/// note.
fn assert_uncaptured(cases: &[(&str, u64, &str, &str)]) {
    let preload: String = cases.iter().map(|(form, ..)| format!("{form}\n")).collect();
    let (_, trace) = run("uncaptured", &preload, true);
    let trace = trace.unwrap();
    let found = tracks(&trace);
    for (line, (form, root, access, symbol)) in (1..).zip(cases) {
        assert!(found.contains(&(line, *root, (*access).into(), (*symbol).into())), "{form}: {found:?}");
    }
    assert_eq!(lines(&trace), (1..=cases.len() as u64).collect::<Vec<_>>(), "{trace}");
}

#[test]
fn reads_of_tracked_values_are_noted_compiled_and_interpreted() {
    assert_uncaptured(&[
        ("(defvar x1 (length tracking-a))", 3, "read", "tracking-a"),
        ("(symbol-value 'tracking-a)", 3, "read", "tracking-a"),
        ("(default-value 'tracking-a)", 3, "read", "tracking-a"),
        ("(funcall (eval '(lambda () tracking-a) nil))", 3, "read", "tracking-a"),
        ("(eval 'tracking-a nil)", 3, "read", "tracking-a"),
        ("(buffer-local-value 'tracking-a (current-buffer))", 3, "read", "tracking-a"),
        ("(buffer-local-value 'transient-mark-mode (current-buffer))", 3, "read", "transient-mark-mode"),
        ("(defvar x2 (length process-environment))", 1, "read", "process-environment"),
        ("(let ((n 0)) (dolist (e initial-environment) (setq n (1+ n))))", 1, "read", "initial-environment"),
        ("(expand-file-name \"foo\")", 2, "read", "default-directory"),
        // No such login: GNU expands the name against the buffer's directory.
        ("(expand-file-name \"~nosuchuser-tracking/a\")", 2, "read", "default-directory"),
        // The buffer's directory has a handler, which an absolute name meets.
        ("(progn (defun tracking-dir-handler (op &rest args) (let ((inhibit-file-name-handlers '(tracking-dir-handler)) (inhibit-file-name-operation op)) (apply op args))) (let ((file-name-handler-alist '((\"/\\\\'\" . tracking-dir-handler)))) (expand-file-name \"/a\")))", 2, "read", "default-directory"),
        // So does its expansion, whose handler is given the directory.
        ("(let ((file-name-handler-alist '((\"\\\\`/tracking-b\\\\'\" . tracking-dir-handler)))) (expand-file-name \"/a/../tracking-b\"))", 2, "read", "default-directory"),
        ("(let ((default-directory default-directory)) nil)", 2, "read", "default-directory"),
        ("(defvar x3 (random 100))", 5, "read", "random state"),
    ]);
}

#[test]
fn writes_of_tracked_values_are_noted() {
    assert_uncaptured(&[
        ("(setq tracking-a \"/x\")", 3, "write", "tracking-a"),
        ("(set 'tracking-a \"/x\")", 3, "write", "tracking-a"),
        ("(makunbound 'tracking-b)", 3, "write", "tracking-b"),
        ("(set-default 'tracking-a 1)", 3, "write", "tracking-a"),
        ("(make-local-variable 'tracking-a)", 3, "write", "tracking-a"),
        // A built-in's forwarding is the one the tracking keeps: localizing it
        // goes through the tracking, and ends (it looped once).
        ("(make-local-variable 'transient-mark-mode)", 3, "write", "transient-mark-mode"),
        ("(make-variable-buffer-local 'mark-even-if-inactive)", 6, "write", "mark-even-if-inactive"),
        ("(setq process-environment nil)", 1, "write", "process-environment"),
        ("(setq default-directory \"/\")", 2, "write", "default-directory"),
    ]);
}

#[test]
fn the_value_a_let_saved_stays_tracked() {
    assert_uncaptured(&[
        ("(let ((tracking-a nil)) (default-toplevel-value 'tracking-a))", 3, "saved", "tracking-a"),
        ("(let ((tracking-a nil)) (set-default-toplevel-value 'tracking-a \"/x\"))", 3, "saved", "tracking-a"),
        ("(let ((tracking-a nil)) (let ((tracking-a 1)) (default-toplevel-value 'tracking-a)))", 3, "saved", "tracking-a"),
        ("(let ((tracking-a nil)) (defvar tracking-a \"a@b\"))", 3, "saved", "tracking-a"),
        ("(let ((default-directory \"/\")) (buffer-local-toplevel-value 'default-directory))", 2, "saved", "default-directory"),
        ("(let ((default-directory \"/\")) (set-buffer-local-toplevel-value 'default-directory \"/x/\"))", 2, "saved", "default-directory"),
    ]);
}

/// A watcher would be handed the root's value a `let` moves aside and
/// back, so a let of a watched root is noted.
#[test]
fn a_let_of_a_watched_root_is_not_captured() {
    assert_uncaptured(&[
        ("(progn (add-variable-watcher 'tracking-a (lambda (&rest _) nil)) (let ((tracking-a 1)) nil))", 3, "watched", "tracking-a"),
    ]);
}

/// An interactive session's own startup run supersedes the chain's: it may
/// write what the first run read and compute the delayed variables again,
/// which it tracks anew, and its own inputs note writes after it. A third
/// run, or a second from any other file, is noted.
#[test]
fn a_session_startup_supersedes_the_chain_startup() {
    let dir = scratch("session");
    let session = write(&dir.join("lisp/session/romacs-gui-loadup.el"), "\
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq tracking-input \"/session/\")
           (setq tracking-a (concat tracking-input \"s\") tracking-b nil))
  (romacs--startup-phase nil))
");
    let other = write(&dir.join("other.el"), "(romacs--startup-phase t)\n");
    let preload = format!("(load {:?} nil t t)\n(defvar seen tracking-a)\n(setq tracking-input \"/later/\")\n(load {:?} nil t t)\n",
                          session.display().to_string(), other.display().to_string());
    let (mut vm, trace) = run_with("session", BOOTSTRAP, &preload, true);
    let trace = trace.unwrap();
    let found = tracks(&trace);
    assert!(found.contains(&(2, 3, "read".into(), "tracking-a".into())), "{found:?}");
    assert!(found.contains(&(3, 6, "write".into(), "tracking-input".into())), "{found:?}");
    assert!(!found.iter().any(|(line, ..)| *line == 1), "the session's run was noted: {found:?}");
    let switches: Vec<_> = trace["uncaptured"].as_array().unwrap().iter().filter(|r| r["kind"] == "startup").collect();
    assert_eq!(switches.len(), 1, "{trace}");
    assert_eq!(vm.eval_source("(equal tracking-a \"/session/s\")").and_then(|v| v.display()).unwrap(), "t");
}

#[test]
fn lets_buffers_and_startup_pass() {
    let preload = "\
(setq p1 (let ((tracking-a \"/l/\")) (list tracking-a (symbol-value 'tracking-a))))
(setq p2 (let ((tracking-a nil)) (setq tracking-a 2) tracking-a))
(setq p3 (let ((default-directory \"/\")) (expand-file-name \"x\")))
(setq p4 (let ((default-directory \"/\")) (let ((b (current-buffer))) (save-current-buffer (set-buffer (get-buffer-create \" other\")) (set-buffer b) (expand-file-name \"y\")))))
(setq p5 (buffer-name (get-buffer-create \"tracking-new\")))
(setq p6 (getenv-internal \"HOME\"))
(setq p7 (list (expand-file-name \"/a/../b\") (stringp (expand-file-name \"~/x\")) (expand-file-name \"/c\" nil)))
(setq p8 (list (boundp 'tracking-b) (boundp 'process-environment) (boundp 'default-directory)))
";
    let (mut vm, trace) = run("pass", preload, true);
    let trace = trace.unwrap();
    assert!(trace["uncaptured"].as_array().unwrap().is_empty(), "{trace}");
    // A tracked root's value is the process's; that it is bound is not.
    let shown = vm.eval_source("(list p1 p2 p3 p4 p5 (stringp p6) (length startup-saw) p7 p8)").and_then(|v| v.display()).unwrap();
    assert_eq!(shown, "((\"/l/\" \"/l/\") 2 \"/x\" \"/y\" \"tracking-new\" t 4 (\"/b\" t \"/c\") (t t t))");
}

#[test]
fn writes_to_startup_inputs_are_noted_whatever_they_store() {
    let preload = "\
(setq tracking-input \"/x/\")
(setq tracking-input \"/in/\")
(put 'tracking-a 'standard-value (list \"/y\"))
(fset 'tracking-locate (lambda () nil))
(defalias 'tracking-locate (lambda () nil))
(setplist 'tracking-plist nil)
(setq ok1 tracking-input)
(setq ok2 (let ((tracking-input \"/x/\")) tracking-input))
(put 'tracking-a 'other 1)
(setq ok3 (get 'tracking-a 'standard-value))
(makunbound 'tracking-input)
";
    let (_, trace) = run("inputs", preload, true);
    let trace = trace.unwrap();
    let notes: Vec<_> = trace["uncaptured"].as_array().unwrap().iter()
        .map(|r| (r["site"]["line"].as_u64().unwrap(), r["root"].as_u64().unwrap(), r["access"].as_str().unwrap().to_owned(), r["symbol"].as_str().unwrap().to_owned()))
        .collect();
    let expected: Vec<(u64, u64, String, String)> = [(1, "tracking-input"), (2, "tracking-input"), (3, "tracking-a"), (4, "tracking-locate"), (5, "tracking-locate"), (6, "tracking-plist"), (11, "tracking-input")]
        .into_iter().map(|(line, symbol)| (line, 6, "write".to_owned(), symbol.to_owned())).collect();
    assert_eq!(notes, expected, "{trace}");
    let inputs: Vec<_> = trace["records"].as_array().unwrap().iter().filter(|r| r["kind"] == "input")
        .map(|r| format!("{} {} {}", r["input"].as_str().unwrap(), r["symbol"].as_str().unwrap(), r["property"].as_str().unwrap_or(""))).collect();
    for input in ["variable tracking-input ", "property tracking-a standard-value", "plist tracking-plist ", "function tracking-locate ", "variable mark-even-if-inactive "] {
        assert!(inputs.iter().any(|i| i == input), "{input}: {inputs:?}");
    }
}

#[test]
fn an_input_changed_in_place_is_noted_at_the_content_check() {
    let dir = scratch("in-place");
    let bootstrap = write(&dir.join("lisp/session/romacs-loadup-bootstrap.el"), BOOTSTRAP);
    let preload = write(&dir.join("preload.el"), "(setcar (get 'tracking-a 'standard-value) \"/y\")\n");
    host::enter_build(PINNED);
    let mut vm = Vm::new();
    vm.world_begin();
    vm.install_lisp_prelude().expect("prelude");
    for file in [bootstrap, preload] {
        vm.load_file(&file).unwrap();
    }
    assert!(host::leave_build().unwrap()["uncaptured"].as_array().unwrap().is_empty());
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let notes: Vec<_> = trace["uncaptured"].as_array().unwrap().iter()
        .map(|r| (r["access"].as_str().unwrap(), r["input"].as_str().unwrap(), r["symbol"].as_str().unwrap(), r["property"].as_str().unwrap())).collect();
    assert_eq!(notes, [("changed", "property", "tracking-a", "standard-value")], "{trace}");
}

#[test]
fn tracked_variables_behave_as_without_the_tracking() {
    let probes = "\
(setq r1 (list tracking-a (symbol-value 'tracking-a) (default-value 'tracking-a) (boundp 'tracking-b)))
(setq r2 (let ((tracking-a 'l)) (list tracking-a (default-toplevel-value 'tracking-a))))
(setq r3 (progn (setq tracking-b 7) (set-default 'tracking-b (1+ tracking-b)) tracking-b))
(setq r4 (progn (makunbound 'tracking-b) (list (boundp 'tracking-b) (default-boundp 'tracking-b))))
(setq r5 (let ((default-directory \"/d/\")) (list default-directory (expand-file-name \"z\"))))
(setq r6 (list (stringp default-directory) (consp process-environment) (length exec-path)))
(setq r7 (progn (make-local-variable 'tracking-a) (setq tracking-a 'local) (list tracking-a (default-value 'tracking-a) (local-variable-p 'tracking-a))))
";
    let show = "(list r1 r2 r3 r4 r5 r6 r7 tracking-a (default-value 'tracking-a))";
    let (mut off, none) = run("same-off", probes, false);
    assert!(none.is_none());
    let (mut on, trace) = run("same-on", probes, true);
    assert!(!trace.unwrap()["uncaptured"].as_array().unwrap().is_empty());
    let off = off.eval_source(show).and_then(|v| v.display()).unwrap();
    let on = on.eval_source(show).and_then(|v| v.display()).unwrap();
    assert_eq!(on, off);
}

/// The notes as (kind, line, symbol, access, object).
fn listed(trace: &Json) -> Vec<(String, u64, String, String, String)> {
    let text = |r: &Json, key: &str| r[key].as_str().unwrap_or("").to_owned();
    trace["uncaptured"].as_array().unwrap().iter()
        .map(|r| (text(r, "kind"), r["site"]["line"].as_u64().unwrap_or(0), text(r, "symbol"), text(r, "access"), text(r, "object")))
        .collect()
}

#[test]
fn the_content_check_sees_char_tables_and_text_properties() {
    let bootstrap = "\
(defvar custom-delayed-init-variables nil)
(defvar tracking-table (make-char-table 'test))
(defvar tracking-text (copy-sequence \"abc\"))
(defvar tracking-same (make-char-table 'test 'v))
(defvar tracking-buffer (get-buffer-create \" tracking\"))
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq seen (list tracking-table tracking-text tracking-same tracking-buffer)))
  (romacs--startup-phase nil))
";
    let preload = "\
(set-char-table-range tracking-table '(?a . ?c) 'x)
(put-text-property 0 1 'face 'bold tracking-text)
(set-char-table-range tracking-same '(?a . ?c) 'v)
";
    let (mut vm, trace) = run_with("content", bootstrap, preload, true);
    let trace = trace.unwrap();
    // A buffer's state cannot be digested: it is noted as startup ends.
    assert_eq!(listed(&trace), [("undigestable".into(), 6, "tracking-buffer".into(), String::new(), "buffer".into())], "{trace}");
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let changed: Vec<_> = listed(&trace).into_iter().map(|(kind, _, symbol, access, _)| (kind, symbol, access)).collect();
    assert_eq!(changed, [("per-process".into(), "tracking-table".into(), "changed".into()), ("per-process".into(), "tracking-text".into(), "changed".into())], "{trace}");
}

/// Each in-place write changes what `aref`, `char-table-range`, the parent
/// or an extra slot reads; one table per field. A literal's ASCII slot is
/// a node detached from the root, so `aset' below 128 writes only it.
#[test]
fn the_content_check_sees_every_char_table_field() {
    let ascii = format!("#^[nil nil test #^^[3 0{}]{}]", " a".repeat(128), " b".repeat(64));
    let bootstrap = format!("\
(defvar custom-delayed-init-variables nil)
(put 'tracking-purpose 'char-table-extra-slots 1)
(defvar tracking-ascii {ascii})
(defvar tracking-default (make-char-table 'test 'v))
(defvar tracking-parent (make-char-table 'test))
(defvar tracking-child (make-char-table 'test))
(set-char-table-parent tracking-child tracking-parent)
(defvar tracking-reparent (make-char-table 'test))
(defvar tracking-extra (make-char-table 'tracking-purpose 'v))
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq seen (list tracking-ascii tracking-default tracking-child tracking-reparent tracking-extra)))
  (romacs--startup-phase nil))
");
    let preload = "\
(aset tracking-ascii 65 'z)
(set-char-table-range tracking-default nil 'w)
(aset tracking-parent 65 'q)
(set-char-table-parent tracking-reparent (make-char-table 'test 'w))
(set-char-table-extra-slot tracking-extra 0 'x)
";
    let (mut vm, trace) = run_with("char-table-fields", &bootstrap, preload, true);
    assert_eq!(listed(&trace.unwrap()), []);
    let probes = "(list (aref tracking-ascii 65) (aref tracking-ascii 128) (char-table-range tracking-default nil) (aref tracking-child 65))";
    assert_eq!(vm.eval_source(probes).and_then(|v| v.display()).unwrap(), "(z b w q)");
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let mut changed: Vec<_> = listed(&trace).into_iter().map(|(kind, _, symbol, access, _)| (kind, symbol, access)).collect();
    changed.sort();
    let expected = ["tracking-ascii", "tracking-child", "tracking-default", "tracking-extra", "tracking-reparent"]
        .map(|symbol| ("per-process".to_owned(), symbol.to_owned(), "changed".to_owned()));
    assert_eq!(changed, expected, "{trace}");
}

/// Each mutation keeps the bytes a digest without framing would see. With
/// T a symbol's tag and I the interned mark after a name, names ("a" "Tb")
/// become ("aT" "b"), and ("a" "bITc") become ("aITb" "c"); a second reach
/// of one cons moves to another; a vector becomes a record of its items.
#[test]
fn the_content_check_frames_what_it_digests() {
    let bootstrap = "\
(defvar custom-delayed-init-variables nil)
(defvar tracking-tag (concat (string 7) (make-string 7 0)))
(defvar tracking-mark (concat (string 1) tracking-tag))
(defvar tracking-names (vector 'a (intern (concat tracking-tag \"b\"))))
(defvar tracking-marked (vector 'a (intern (concat \"b\" tracking-mark \"c\"))))
(defvar tracking-shared (let ((x (list 1)) (y (list 2))) (list x y x)))
(defvar tracking-kind (list (vector 'a 'b)))
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq seen (list tracking-names tracking-marked tracking-shared tracking-kind)))
  (romacs--startup-phase nil))
";
    let preload = "\
(aset tracking-names 0 (intern (concat \"a\" tracking-tag)))
(aset tracking-names 1 'b)
(aset tracking-marked 0 (intern (concat \"a\" tracking-mark \"b\")))
(aset tracking-marked 1 'c)
(setcar (cdr (cdr tracking-shared)) (car (cdr tracking-shared)))
(setcar tracking-kind (record 'a 'b))
";
    let (mut vm, trace) = run_with("framing", bootstrap, preload, true);
    assert_eq!(listed(&trace.unwrap()), []);
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let changed: Vec<_> = listed(&trace).into_iter().map(|(kind, _, symbol, access, _)| (kind, symbol, access)).collect();
    let expected = ["tracking-names", "tracking-marked", "tracking-shared", "tracking-kind"]
        .map(|symbol| ("per-process".to_owned(), symbol.to_owned(), "changed".to_owned()));
    assert_eq!(changed, expected, "{trace}");
}

/// Each replacement or mutation keeps a table's count and entries and
/// changes one field Lisp observes: its test, weakness or size, or the free
/// list, which fixes where later entries iterate. Freeing the last slot, or
/// re-adding an identical empty table, changes nothing Lisp sees.
#[test]
fn the_content_check_sees_every_hash_table_field() {
    let bootstrap = "\
(defvar custom-delayed-init-variables nil)
(defvar tracking-test (vector (make-hash-table :test 'eq)))
(defvar tracking-weak (vector (make-hash-table :test 'eq)))
(defvar tracking-size (vector (make-hash-table :test 'eq)))
(defvar tracking-order (make-hash-table :test 'eq :size 10))
(puthash 'a 1 tracking-order)
(puthash 'b 2 tracking-order)
(remhash 'a tracking-order)
(remhash 'b tracking-order)
(defvar tracking-same (vector (make-hash-table :test 'eq)))
(defvar tracking-tail (make-hash-table :test 'eq :size 10))
(puthash 'a 1 tracking-tail)
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq seen (list tracking-test tracking-weak tracking-size tracking-order tracking-same tracking-tail)))
  (romacs--startup-phase nil))
";
    let preload = "\
(aset tracking-test 0 (make-hash-table :test 'equal))
(aset tracking-weak 0 (make-hash-table :test 'eq :weakness 'key))
(aset tracking-size 0 (make-hash-table :test 'eq :size 10))
(puthash 'a 1 tracking-order)
(puthash 'b 2 tracking-order)
(remhash 'a tracking-order)
(remhash 'b tracking-order)
(aset tracking-same 0 (make-hash-table :test 'eq))
(puthash 'b 2 tracking-tail)
(remhash 'b tracking-tail)
";
    let (mut vm, trace) = run_with("hash-table-fields", bootstrap, preload, true);
    assert_eq!(listed(&trace.unwrap()), []);
    // Two later entries iterate in the other order than in a table freed
    // as tracking-order was before startup ended.
    let probes = "\
(let ((later (lambda (table) (let ((copy (copy-hash-table table))) (puthash 'c 3 copy) (puthash 'd 4 copy) (hash-table-keys copy))))
      (before (make-hash-table :test 'eq :size 10)))
  (puthash 'a 1 before) (puthash 'b 2 before) (remhash 'a before) (remhash 'b before)
  (list (hash-table-test (aref tracking-test 0)) (hash-table-weakness (aref tracking-weak 0)) (hash-table-size (aref tracking-size 0))
        (funcall later tracking-order) (funcall later before)))";
    assert_eq!(vm.eval_source(probes).and_then(|v| v.display()).unwrap(), "(equal key 10 (d c) (c d))");
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let changed: Vec<_> = listed(&trace).into_iter().map(|(kind, _, symbol, access, _)| (kind, symbol, access)).collect();
    let expected = ["tracking-test", "tracking-weak", "tracking-size", "tracking-order"]
        .map(|symbol| ("per-process".to_owned(), symbol.to_owned(), "changed".to_owned()));
    assert_eq!(changed, expected, "{trace}");
}

/// A key mutated after its insertion keeps the hash it was stored under:
/// an equal table holding "b" under the hash of "a" misses a lookup of "b"
/// that a fresh table of the same size and entry finds. Replacing one with
/// the other is noted; the table left alone does not.
#[test]
fn the_content_check_sees_a_mutated_keys_stored_hash() {
    let bootstrap = "\
(defvar custom-delayed-init-variables nil)
(defun tracking-mutated () (let ((key (copy-sequence \"a\")) (table (make-hash-table :test 'equal))) (puthash key 1 table) (aset key 0 ?b) table))
(defvar tracking-lookup (vector (tracking-mutated)))
(defvar tracking-kept (vector (tracking-mutated)))
(unwind-protect
    (progn (romacs--startup-phase t)
           (setq seen (list tracking-lookup tracking-kept)))
  (romacs--startup-phase nil))
";
    let preload = "(aset tracking-lookup 0 (let ((table (make-hash-table :test 'equal))) (puthash (copy-sequence \"b\") 1 table) table))\n";
    let (mut vm, trace) = run_with("stored-hash", bootstrap, preload, true);
    assert_eq!(listed(&trace.unwrap()), []);
    let probes = "(mapcar (lambda (box) (gethash (copy-sequence \"b\") (aref box 0) 'missing)) (list tracking-lookup tracking-kept))";
    assert_eq!(vm.eval_source(probes).and_then(|v| v.display()).unwrap(), "(1 missing)");
    host::enter_build(PINNED);
    vm.world_check_inputs();
    let trace = host::leave_build().unwrap();
    let changed: Vec<_> = listed(&trace).into_iter().map(|(kind, _, symbol, access, _)| (kind, symbol, access)).collect();
    assert_eq!(changed, [("per-process".to_owned(), "tracking-lookup".to_owned(), "changed".to_owned())], "{trace}");
}

/// A stored hash that read a heap index digests as only that fact: an eq
/// table keyed by a cons and a vector digests alike in two fresh VMs
/// whose keys sit at other arena indices.
#[test]
fn identity_hashes_digest_alike_across_vms() {
    let bootstrap = |pad: &str| format!("\
(defvar custom-delayed-init-variables nil)
(defvar tracking-pad {pad})
(defvar tracking-heap (let ((table (make-hash-table :test 'eq))) (puthash (list 'a) 1 table) (puthash (vector 'b) 2 table) table))
(unwind-protect
    (progn (romacs--startup-phase t) (setq seen tracking-heap))
  (romacs--startup-phase nil))
");
    let digest = |name, pad| {
        let (_, trace) = run_with(name, &bootstrap(pad), "", true);
        let trace = trace.unwrap();
        let input = trace["records"].as_array().unwrap().iter().find(|r| r["kind"] == "input" && r["symbol"] == "tracking-heap").cloned();
        input.unwrap_or_else(|| panic!("{trace}"))["digest"].clone()
    };
    assert_eq!(digest("identity-bare", "nil"), digest("identity-padded", "(list (list 1) (vector 2))"));
}

#[test]
fn a_binding_across_startups_end_stays_tracked_in_backtraces() {
    let bootstrap = "\
(defvar custom-delayed-init-variables '(tracking-a transient-mark-mode))
(defvar tracking-a nil)
(defvar tracking-seen nil)
(defun tracking-locals () (setq tracking-seen (cons (backtrace--locals 2) tracking-seen)))
(defun tracking-run ()
  (let ((transient-mark-mode 'outer) (tracking-a \"/outer/\"))
    (unwind-protect
        (progn (romacs--startup-phase t) (setq custom-delayed-init-variables t))
      (romacs--startup-phase nil))
    (let ((tracking-a 'inner)) (tracking-locals))))
(tracking-run)
";
    let preload = "\
(defun tracking-new () (let ((tracking-a 'new)) (tracking-locals)))
(tracking-new)
";
    let (mut vm, trace) = run_with("backtrace", bootstrap, preload, true);
    let trace = trace.unwrap();
    // The current binding of the built-in variable and the value the inner
    // `let' saved are both the root's; the newer `let's are their own.
    assert_eq!(listed(&trace), [
        ("per-process".into(), 11, "transient-mark-mode".into(), "saved".into(), String::new()),
        ("per-process".into(), 11, "tracking-a".into(), "saved".into(), String::new()),
    ], "{trace}");
    let (mut off, _) = run_with("backtrace-off", bootstrap, preload, false);
    let shown = "(format \"%S\" tracking-seen)";
    let seen = vm.eval_source(shown).and_then(|v| v.display()).unwrap();
    assert_eq!(seen, "\"(((tracking-a . new)) ((transient-mark-mode . outer) (tracking-a . \\\"/outer/\\\") (tracking-a . inner)))\"");
    assert_eq!(seen, off.eval_source(shown).and_then(|v| v.display()).unwrap());
}

/// A tracked variable that is buffer-local reads and writes as without
/// the tracking in two buffers: a buffer's own local, the default the
/// other sees, a `let' of each, and `buffer-local-value' across them.
#[test]
fn a_tracked_buffer_local_reads_alike_in_two_buffers() {
    let bootstrap = "\
(defvar custom-delayed-init-variables '(tracking-local))
(defvar-local tracking-local nil)
(put 'tracking-local 'standard-value (list \"std\"))
(unwind-protect
    (progn (romacs--startup-phase t)
           (set-default 'tracking-local (car (get 'tracking-local 'standard-value)))
           (setq custom-delayed-init-variables t))
  (romacs--startup-phase nil))
";
    let probes = "\
(setq ta (get-buffer-create \"tracked-a\") tb (get-buffer-create \"tracked-b\"))
(setq r1 (save-current-buffer (set-buffer ta) (setq tracking-local 'in-a) (list tracking-local (local-variable-p 'tracking-local))))
(setq r2 (save-current-buffer (set-buffer tb) (list tracking-local (local-variable-p 'tracking-local) (buffer-local-value 'tracking-local ta))))
(setq r3 (save-current-buffer (set-buffer ta) (list (let ((tracking-local 'let-a)) (list tracking-local (buffer-local-value 'tracking-local tb))) tracking-local)))
(setq r4 (save-current-buffer (set-buffer tb) (list (let ((tracking-local 'let-b)) (list tracking-local (default-value 'tracking-local) (buffer-local-value 'tracking-local ta))) tracking-local)))
";
    let show = "(list r1 r2 r3 r4 (default-value 'tracking-local))";
    let (mut off, none) = run_with("local-off", bootstrap, probes, false);
    assert!(none.is_none());
    let (mut on, _) = run_with("local-on", bootstrap, probes, true);
    let off = off.eval_source(show).and_then(romacs_vm::Local::display).unwrap();
    let on = on.eval_source(show).and_then(romacs_vm::Local::display).unwrap();
    assert_eq!(off, "((in-a t) (\"std\" nil in-a) ((let-a \"std\") in-a) ((let-b let-b in-a) \"std\") \"std\")");
    assert_eq!(on, off);
}

/// `truncate-lines' is declared automatically local at its first lookup. A
/// build's startup tracks it when its redirect was resolved before startup;
/// its first set in a buffer, inside startup, still makes that buffer's
/// local, and leaves other buffers the default, with the tracking on as
/// off.
#[test]
fn a_tracked_built_in_automatic_local_is_made_local_at_its_first_set() {
    let bootstrap = "\
(defvar custom-delayed-init-variables nil)
(setq tl-seen (default-value 'truncate-lines))
(setq tl-a (get-buffer-create \"tl-a\") tl-b (get-buffer-create \"tl-b\"))
(unwind-protect
    (progn (romacs--startup-phase t)
           (save-current-buffer (set-buffer tl-a) (setq truncate-lines t))
           (setq custom-delayed-init-variables t))
  (romacs--startup-phase nil))
";
    let show = "(list (buffer-local-value 'truncate-lines tl-a) (buffer-local-value 'truncate-lines tl-b) (local-variable-p 'truncate-lines tl-a) (default-value 'truncate-lines))";
    let (mut off, none) = run_with("tl-off", bootstrap, "", false);
    assert!(none.is_none());
    let (mut on, _) = run_with("tl-on", bootstrap, "", true);
    let off = off.eval_source(show).and_then(romacs_vm::Local::display).unwrap();
    let on = on.eval_source(show).and_then(romacs_vm::Local::display).unwrap();
    assert_eq!(off, "(t nil t nil)");
    assert_eq!(on, off);
}
