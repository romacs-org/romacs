//! func-arity (eval.c) over subrs, special forms, macros, closures and
//! lambda lists, including malformed ones, and `apply' given only the
//! spread list (GNU calls its first element on the rest).
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(progn (defmacro arity-macro (a &rest b) a) (mapcar (lambda (f) (condition-case e (func-arity f) (error e))) (list 'car 'if 'arity-macro 'list 'mapcar 'apply 'setq 'defalias (lambda (a &optional b) a) (lambda (a b &rest c) a) (lambda () 1) '(lambda (x &optional y z)) '(macro lambda (a &rest b)))))
(mapcar (lambda (f) (condition-case e (func-arity f) (error e))) (list '(lambda (x . y)) '(lambda) '(lambda (1)) nil 'romacs-no-such-function 3 "s" '(foo)))
(progn (defalias 'arity-alias 'arity-target) (defun arity-target (a &optional b c) a) (list (func-arity 'arity-alias) (func-arity (symbol-function 'arity-target))))
(list (apply '(+ 1 2)) (funcall 'apply (list 'list 1 2)) (apply (list 'list)) (condition-case e (apply nil) (error e)) (condition-case e (apply '(nil 1)) (error e)) (apply '+ 1 '(2 3)) (let ((f 'apply)) (funcall f '(list 1))))
"###;

#[test]
fn func_arity_and_single_argument_apply_match_pinned_gnu() {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in CASES.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
