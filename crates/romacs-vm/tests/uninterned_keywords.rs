//! Only an interned keyword is constant (GNU lread.c intern_sym marks it
//! SYMBOL_NOWRITE): an uninterned symbol named `:x' is an ordinary
//! variable to set, bind, localize, alias and watch, against pinned GNU.
use romacs_vm::Vm;

const CASES: &str = r##"
(let ((s (make-symbol ":x"))) (set s 1) (symbol-value s))
(let ((s (make-symbol ":x"))) (set s 1) (makunbound s) (boundp s))
(let ((s (make-symbol ":x"))) (with-temp-buffer (make-local-variable s) (set s 2) (list (local-variable-p s) (symbol-value s))))
(let ((s (make-symbol ":x"))) (add-variable-watcher s #'ignore) (get-variable-watchers s))
(let ((s (make-symbol ":x")) (a (make-symbol "a"))) (set s 3) (defvaralias a s) (symbol-value a))
(let ((s (make-symbol ":x"))) (eval `(let ((,s 4)) ,s) t))
(let ((s (make-symbol ":x"))) (funcall (eval `(lambda (,s) ,s) t) 5))
(let ((s (make-symbol ":x"))) (list (keywordp s) (special-variable-p s) (boundp s)))
(set :x 1)
(make-local-variable :x)
"##;

fn wrapped(expression: &str) -> String {
    format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))")
}

fn oracle(expression: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let source = format!("(prin1 {})", wrapped(expression));
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn romacs(expression: &str) -> String {
    match Vm::new().eval_source(&wrapped(expression)).and_then(|v| v.display()) {
        Ok(text) => text,
        Err(error) => format!("VM error: {error}"),
    }
}

#[test]
fn uninterned_keywords_are_variables_as_in_gnu() {
    let failures: Vec<String> = CASES.lines().filter(|line| !line.is_empty())
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(expression), romacs(expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
