//! error-message-string against pinned GNU print_error_message: the
//! condition's template, the `: ' and `, ' separators, and which data
//! items print with princ rather than prin1.
use romacs_vm::Vm;

const CASES: &str = r##"
(error-message-string '(user-error "abc"))
(error-message-string (list 'user-error (string-to-unibyte "Thema changed; copy")))
(error-message-string '(user-error "a" "b" 3))
(error-message-string '(user-error))
(error-message-string '(error "x" "y" 3))
(error-message-string '(error))
(error-message-string '(error 3))
(error-message-string (list 'error (string-to-unibyte "plain")))
(let ((s "same")) (eq s (error-message-string (list 'error s))))
(error-message-string '(end-of-file "a" b))
(error-message-string '(wrong-type-argument stringp "s" 1.5))
(error-message-string '(void-variable foo))
(error-message-string '(file-missing "Opening input file" "No such file or directory" "/x"))
(error-message-string '(file-error "Opening" "/x" 3))
(error-message-string '(args-out-of-range "foo" 4 . 5))
(error-message-string '(no-such-condition-here 1 "a"))
(error-message-string nil)
(error-message-string '(quit))
(let ((print-length 1)) (error-message-string '(wrong-type-argument (1 2 3))))
(progn (define-error 'romacs-test-quoted "Can't `frob' it") (error-message-string '(romacs-test-quoted 1)))
(let ((text-quoting-style 'grave)) (define-error 'romacs-test-quoted "Can't `frob' it") (error-message-string '(romacs-test-quoted 1)))
(let ((text-quoting-style 'straight)) (define-error 'romacs-test-quoted "Can't `frob' it") (error-message-string '(romacs-test-quoted 1)))
(error-message-string '(error "x `y' z"))
"##;

fn oracle(expression: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let source = format!("(prin1 (prin1-to-string (condition-case e {expression} (error (list 'signal e)))))");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn romacs(expression: &str) -> String {
    let source = format!("(prin1-to-string (condition-case e {expression} (error (list 'signal e))))");
    match Vm::new().eval_source(&source).and_then(|v| v.display()) {
        Ok(text) => text,
        Err(error) => format!("VM error: {error}"),
    }
}

#[test]
fn error_message_string_matches_gnu() {
    let failures: Vec<String> = CASES.lines().filter(|line| !line.is_empty())
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(expression), romacs(expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
