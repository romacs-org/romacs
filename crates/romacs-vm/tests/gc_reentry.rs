//! Collection across Lisp reentered from native code, under forced moving
//! collections (`Vm::set_gc_stress`): every Lisp entry collects and moves
//! every live object, so a heap Value a native frame kept in a Rust local
//! across the call would become a wrong object.
//!
//! Each case prints the same under stress as without it, collections
//! happened, and every `(garbage-collect)' its callbacks made collected
//! (GNU's inhibited collection returns nil): a watcher of `setq', `let' and
//! its unwinding (global, buffer-local and forwarded bindings, a signalling
//! watcher and one setting the toplevel value), repeated macro expansion,
//! a nested load, and a prompt entered through `eval'.
use romacs_vm::{CommandEvent, HostEvent, HostFrame, HostInput, Vm};

/// A watcher recording each call with a fresh copy of the new value, and
/// whether its own `(garbage-collect)' ran.
const W: &str = "(lambda (s n o w) (setq log (cons (list o (prin1-to-string n)) log)) (setq gcs (cons (and (garbage-collect) t) gcs)))";

const CASES: &[&str] = &[
    "(progn (defvar gr-a (list 0)) (add-variable-watcher 'gr-a W) (setq gr-a (list 1 (make-string 2 ?y) (cons 2 3))) (list gr-a (nreverse log)))",
    "(progn (defvar gr-b (list 'outer (make-string 2 ?o))) (add-variable-watcher 'gr-b W) (list (let ((gr-b (list 'inner (make-string 2 ?i)))) (copy-sequence gr-b)) gr-b (nreverse log)))",
    "(progn (defvar gr-c (list 'default)) (add-variable-watcher 'gr-c W) (with-temp-buffer (make-local-variable 'gr-c) (setq gr-c (list 'local (make-string 2 ?l))) (list (let ((gr-c (list 'inner))) gr-c) gr-c (nreverse log))))",
    "(progn (add-variable-watcher 'fill-column W) (with-temp-buffer (list (let ((fill-column 33)) fill-column) fill-column (nreverse log))))",
    "(progn (defvar gr-d (list 1)) (add-variable-watcher 'gr-d (lambda (s n o w) (setq gcs (cons (and (garbage-collect) t) gcs)) (if (eq o 'unlet) (error \"No unlet %S\" (list n))))) (list (condition-case e (let ((gr-d (list 2))) 'body) (error (list e gr-d))) gr-d))",
    "(progn (defvar gr-e 1) (make-variable-buffer-local 'gr-e) (add-variable-watcher 'gr-e (lambda (s n o w) (setq gcs (cons (and (garbage-collect) t) gcs)) (if (eq o 'unlet) (set-buffer-local-toplevel-value 'gr-e (list 'toplevel (make-string 2 ?t)))))) (with-temp-buffer (setq gr-e (list 'local)) (list (let ((gr-e (list 'let))) gr-e) gr-e)))",
    "(progn (defmacro gr-m (x) (setq gcs (cons (and (garbage-collect) t) gcs)) (list 'gr-n x (make-string 2 ?m))) (defmacro gr-n (x y) (setq gcs (cons (and (garbage-collect) t) gcs)) (list 'quote (list x y (cons 1 2)))) (let ((out nil) (i 0)) (while (< i 20) (setq out (macroexpand (list 'gr-m (list i (make-string 1 ?k))))) (setq i (1+ i))) (list out (macroexpand '(gr-l 7) '((gr-l . (lambda (x) (setq gcs (cons (and (garbage-collect) t) gcs)) (list 'quote (make-list 3 x)))))))))",
];

fn vm(stress: bool) -> Vm {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    vm.eval_source("(setq log nil gcs nil)").unwrap();
    if stress { vm.set_gc_stress(1); }
    vm
}

fn eval(vm: &mut Vm, source: &str) -> String {
    match vm.eval_source(source) {
        Ok(value) => value.display().unwrap(),
        Err(e) => format!("VM-ERROR {e}"),
    }
}

/// SOURCE's printed value and its callbacks' collection results, without
/// and with stress; the stressed run must have collected throughout.
fn run(source: &str, prepare: impl Fn(&mut Vm)) -> (String, String, String) {
    let mut plain = vm(false);
    prepare(&mut plain);
    let expected = eval(&mut plain, source);
    let mut stressed = vm(true);
    prepare(&mut stressed);
    let before = stressed.heap_stats().collections;
    let actual = eval(&mut stressed, source);
    assert!(stressed.heap_stats().collections > before + 1, "stress collected: {source}");
    let gcs = eval(&mut stressed, "gcs");
    (expected, actual, gcs)
}

fn assert_case(source: &str, prepare: impl Fn(&mut Vm)) {
    let (expected, actual, gcs) = run(source, prepare);
    assert!(!expected.starts_with("VM-ERROR"), "{source}\n  {expected}");
    assert_eq!(actual, expected, "{source}");
    assert!(gcs.starts_with("(t") && !gcs.contains("nil"), "every callback collected: {gcs}\n  {source}");
}

#[test]
fn watchers_and_expanders_collect_across_moving_collections() {
    for case in CASES {
        assert_case(&case.replace("W", W), |_| {});
    }
}

#[test]
fn a_nested_load_collects_across_moving_collections() {
    let dir = std::env::temp_dir().join(format!("romacs-gc-reentry-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let inner = dir.join("gr-inner.el");
    let outer = dir.join("gr-outer.el");
    std::fs::write(&inner, "(setq gr-inner (list 'inner (make-string 2 ?b)))\n(setq gcs (cons (and (garbage-collect) t) gcs))\n(provide 'gr-inner)\n").unwrap();
    std::fs::write(&outer, format!(
        "(setq gr-before (list 'outer (make-string 2 ?a)))\n(load {:?} nil t)\n(require 'gr-inner)\n(setq gcs (cons (and (garbage-collect) t) gcs))\n(setq gr-after (list gr-before gr-inner))\n",
        inner.to_str().unwrap()
    )).unwrap();
    let source = format!("(list (load {:?} nil t) gr-after (featurep 'gr-inner))", outer.to_str().unwrap());
    assert_case(&source, |_| {});
    std::fs::remove_dir_all(&dir).unwrap();
}

struct Keys(std::collections::VecDeque<HostEvent>);

impl HostInput for Keys {
    fn input_pending(&mut self) -> bool { false }
    fn redisplay(&mut self, _: HostFrame) -> Result<Vec<(u64, usize)>, String> { Ok(vec![]) }
    fn next_event(&mut self, _: Option<std::time::Duration>) -> Result<HostEvent, String> {
        self.0.pop_front().ok_or_else(|| "host script exhausted".to_string())
    }
}

/// A prompt entered through `eval' waits with the evaluating engines
/// suspended; a command typed there collects, and redisplay between keys
/// runs with collection allowed.
#[test]
fn a_prompt_entered_through_eval_collects_across_moving_collections() {
    let key = |c: char| HostEvent::Key(CommandEvent::Character(c as i64));
    let source = "(eval '(list (read-from-minibuffer \"P: \") (length gcs)))";
    assert_case(source, |vm| {
        vm.set_frame_geometry(800, 600, 20, 10).unwrap();
        // Install the Lisp half of the minibuffer first (host_input.rs).
        assert_eq!(eval(vm, "(condition-case e (read-string \"\") (error (car e)))"), "end-of-file");
        eval(vm, "(define-key minibuffer-local-map \"!\" (lambda () (interactive) (setq gcs (cons (and (garbage-collect) t) gcs)) (insert (make-string 1 ?!))))");
        vm.set_host_input(Some(Box::new(Keys(
            vec![key('a'), key('!'), key('b'), key('!'), HostEvent::Key(CommandEvent::Character(13))].into(),
        ))));
    });
}

/// Refreshing a prepared call whose macro guard went stale runs Lisp to
/// remake its lambda; a heap argument passed in survives that collection.
#[test]
fn a_refreshed_prepared_call_keeps_its_arguments_across_collection() {
    let mut vm = vm(true);
    vm.eval_source("(defmacro gr-pm (x) (list 'car x))").unwrap();
    let handle = vm.prepare_source("(lambda (x) (list (gr-pm x) x))").unwrap();
    let arg = vm.eval_source("(list (copy-sequence \"ppp\") (quote tail))").unwrap().root();
    // Redefining the macro invalidates the prepared guard.
    vm.eval_source("(defmacro gr-pm (x) (list 'cdr x))").unwrap();
    let before = vm.heap_stats().collections;
    let value = vm.invoke_prepared(handle, &[romacs_vm::Arg::Rooted(&arg)]).unwrap().display().unwrap();
    assert!(vm.heap_stats().collections > before, "the refresh collected");
    assert_eq!(value, "((tail) (\"ppp\" tail))");
}
