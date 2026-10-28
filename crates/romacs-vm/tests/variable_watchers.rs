//! data.c notify_variable_watchers with Lisp watch functions against pinned
//! GNU: OPERATION set, let, unlet, makunbound and defvaralias, the WHERE
//! buffer, watchers running before the write, a signalling or throwing
//! watcher, watchers that run arbitrary Lisp, and aliases.
use romacs_vm::Vm;

/// One expression per line. LOG is a watcher recording its arguments, with
/// WHERE as a buffer name, and the value the variable had when it ran.
const CASES: &str = r###"
(progn (defvar wv-a 1) (setq log nil) (add-variable-watcher 'wv-a LOG) (setq wv-a 2) (let ((wv-a 3)) (setq wv-a 4)) (set 'wv-a 5) (makunbound 'wv-a) (list (nreverse log) (boundp 'wv-a) (length (get-variable-watchers 'wv-a))))
(progn (defvar wv-b 1) (make-variable-buffer-local 'wv-b) (defvar wv-c 1) (setq log nil) (add-variable-watcher 'wv-b LOG) (add-variable-watcher 'wv-c LOG) (with-temp-buffer (setq wv-b 2) (set-default 'wv-b 3) (kill-local-variable 'wv-b) (make-local-variable 'wv-c) (setq wv-c 4) (set-default 'wv-c 5) (kill-local-variable 'wv-c) (setq wv-c 6)) (list (nreverse log) wv-b wv-c))
(progn (defvar wv-d 1) (defvar wv-e 7) (setq log nil) (add-variable-watcher 'wv-d LOG) (defvaralias 'wv-d 'wv-e) (setq wv-e 8) (list (nreverse log) wv-d (indirect-variable 'wv-d)))
(progn (defvar wv-f 1) (add-variable-watcher 'wv-f (lambda (s n o w) (if (eq n 'bad) (error "No %s" n)))) (list (condition-case e (setq wv-f 'bad) (error e)) wv-f (condition-case e (let ((wv-f 'bad)) wv-f) (error e)) wv-f (progn (setq wv-f 2) wv-f)))
(progn (defvar wv-g 1) (add-variable-watcher 'wv-g (lambda (s n o w) (if (eq n 'jump) (throw 'out (list 'thrown wv-g))))) (list (catch 'out (setq wv-g 'jump) 'not-thrown) wv-g))
(progn (defvar wv-h 0) (setq calls 0) (add-variable-watcher 'wv-h (lambda (s n o w) (setq calls (1+ calls)) (setq wv-h (* 10 n)) (funcall (lambda (x) (list x (eval '(let ((y x)) (* y 2)) t))) n))) (list (progn (setq wv-h 3) wv-h) calls (let ((wv-h 4)) wv-h) wv-h calls))
(progn (defvar wv-mode t) (defvar wv-total nil) (setq disabled nil) (fset 'wv-disable (lambda () (setq wv-mode nil) (setq disabled t))) (let ((w (lambda (_s _v op _w) (when (and (eq op 'set) wv-mode (not disabled)) (wv-disable))))) (add-variable-watcher 'wv-total w) (setq wv-total 5) (remove-variable-watcher 'wv-total w) (setq wv-total 6) (list disabled wv-mode wv-total (get-variable-watchers 'wv-total))))
(progn (defvar wv-i 1) (defvaralias 'wv-j 'wv-i) (setq log nil) (add-variable-watcher 'wv-j LOG) (setq wv-i 2) (setq wv-j 3) (list (nreverse log) (length (get-variable-watchers 'wv-i)) (equal (get-variable-watchers 'wv-i) (get-variable-watchers 'wv-j))))
"###;

const LOG: &str = "(lambda (s n o w) (setq log (cons (list s n o (if (bufferp w) (buffer-name w) w) (if (boundp s) (symbol-value s) 'void)) log)))";

#[test]
fn lisp_watchers_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        let expression = expression.replace("LOG", LOG);
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        // Collect at every safepoint, so a watcher that ran while collection
        // was inhibited is followed by collections that would expose an
        // unforwarded register.
        vm.set_collection_floor(1);
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
