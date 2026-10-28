//! call-interactively's reading codes `s', `D', `f', `F' and `G' against
//! pinned GNU (callint.c:509-525, 681): the prompt after format-message,
//! the arguments read-string and read-file-name receive, and
//! command-history, and a prompt sliced from the spec's bytes as
//! make_string makes it, unibyte unless they form multibyte text.
use romacs_vm::Vm;

/// One expression per line. Each replaces the readers to log their calls.
const CASES: &str = r###"
(progn (fset 'read-string (lambda (&rest _) "mocked")) (fset 'cmd (lambda (s n) (interactive "sName `here': \np") (list s n))) (setq command-history nil) (list (let ((executing-kbd-macro t) (unread-command-events (append "typed\r" nil))) (call-interactively 'cmd)) command-history))
(progn (fset 'cmd (lambda (a p b) (interactive "sA: \nP\nsB: ") (list a p b))) (setq command-history nil) (let ((current-prefix-arg '(4)) (executing-kbd-macro t) (unread-command-events (append "one\r\r" nil))) (list (call-interactively 'cmd t) command-history)))
(progn (setq calls nil) (fset 'read-file-name (lambda (&rest a) (setq calls (cons (cons 'file a) calls)) "/tmp/x")) (fset 'cmd (lambda (f) (interactive "GBackup `to': ") f)) (setq command-history nil) (list (call-interactively 'cmd) (nreverse calls) command-history))
(progn (setq calls nil) (fset 'read-file-name (lambda (&rest a) (setq calls (cons a calls)) nil)) (fset 'cmd (lambda (p f) (interactive "p\nG") (list p f))) (setq command-history nil) (list (call-interactively 'cmd) (nreverse calls) command-history))
(progn (setq calls nil) (fset 'read-file-name (lambda (&rest a) (setq calls (cons a calls)) "/d/")) (fset 'cmd (lambda (d f g) (interactive "DDir: \nfFile: \nFNew: ") (list d f g))) (setq command-history nil) (let ((default-directory "/tmp/")) (list (call-interactively 'cmd) (nreverse calls) command-history)))
(progn (fset 'read-file-name (lambda (&rest _) (signal 'quit nil))) (fset 'cmd (lambda (f) (interactive "GFile: ") f)) (setq command-history nil) (list (condition-case e (call-interactively 'cmd) (quit (list 'quit e))) command-history))
(progn (setq calls nil) (fset 'read-file-name (lambda (p &rest _) (setq calls (cons (list (multibyte-string-p p) (append p nil)) calls)) "/x")) (dolist (spec (list (unibyte-string 71 255) (string-to-multibyte (unibyte-string 71 255)) (unibyte-string 71 195 169) "G\u00e9: ")) (fset 'cmd (eval (list 'lambda '(f) (list 'interactive spec) 'f) t)) (call-interactively 'cmd)) (nreverse calls))
"###;

#[test]
fn reading_codes_match_pinned_gnu() {
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
