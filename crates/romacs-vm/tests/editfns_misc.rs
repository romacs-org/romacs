//! emacs-pid, invocation-name and -directory, sleep-for,
//! make-variable-buffer-local of a void variable, insert-buffer-substring
//! set-buffer-major-mode and the field functions against pinned GNU
//! (editfns.c, emacs.c, dispnew.c, data.c, buffer.c).
use romacs_vm::Vm;

/// One expression per line.
const CASES: &str = r###"
(list (natnump (emacs-pid)) (stringp invocation-name) (file-name-absolute-p invocation-directory) (eq (aref invocation-directory (1- (length invocation-directory))) ?/) (sleep-for 0) (sleep-for 0.01) (sleep-for 0 5) (sleep-for -1) (condition-case e (sleep-for "x") (error e)) (condition-case e (sleep-for 0 1.5) (error e)))
(list (progn (make-variable-buffer-local 'rt-mvbl) (list (boundp 'rt-mvbl) (default-boundp 'rt-mvbl) (local-variable-if-set-p 'rt-mvbl) (with-temp-buffer (setq rt-mvbl 3) (list rt-mvbl (default-value 'rt-mvbl))))) (progn (defvar rt-mv2 5) (make-variable-buffer-local 'rt-mv2) rt-mv2) (let ((rt-mv3 7)) (make-variable-buffer-local 'rt-mv3) rt-mv3) (with-temp-buffer (make-local-variable 'rt-mv4) (make-variable-buffer-local 'rt-mv4) (list (default-boundp 'rt-mv4) (local-variable-p 'rt-mv4))))
(let ((a (get-buffer-create " rt-src"))) (with-current-buffer a (erase-buffer) (insert "héllo world") (put-text-property 1 3 'face 'bold)) (with-temp-buffer (insert "[]") (goto-char 2) (list (insert-buffer-substring a 2 5) (buffer-string) (insert-buffer-substring " rt-src" 9 7) (buffer-string) (insert-buffer-substring a) (point) (condition-case e (insert-buffer-substring "rt-none") (error e)) (condition-case e (insert-buffer-substring a 0 3) (error e)) (insert-buffer-substring a (with-current-buffer a (copy-marker 3)) 4) (with-current-buffer a (narrow-to-region 3 6)) (progn (erase-buffer) (insert-buffer-substring a) (buffer-string)) (progn (kill-buffer a) (condition-case e (insert-buffer-substring a) (error e))))))
(let ((b (get-buffer-create " rt-mode")) (old (default-value 'major-mode))) (defalias 'rt-test-mode (lambda () (setq major-mode 'rt-test-mode) (list 'ran (buffer-name)))) (unwind-protect (list (progn (set-default 'major-mode 'rt-test-mode) (list (set-buffer-major-mode b) (with-current-buffer b major-mode) (eq (current-buffer) b))) (progn (set-default 'major-mode nil) (put major-mode 'mode-class 'special) (prog1 (list (set-buffer-major-mode b) (with-current-buffer b major-mode)) (put major-mode 'mode-class nil))) (condition-case e (set-buffer-major-mode "x") (error e)) (progn (kill-buffer b) (condition-case e (set-buffer-major-mode b) (error e)))) (set-default 'major-mode old)))
(with-temp-buffer (insert (propertize "P: " 'field 'prompt 'rear-nonsticky t) "abc" (propertize "XY" 'field 'x) "def") (list (mapcar (lambda (p) (list (field-beginning p) (field-end p) (field-string-no-properties p))) '(1 3 4 5 7 8 9 10 12)) (field-beginning 4 t) (field-end 4 t) (field-end 1 nil 2) (field-beginning 9 nil 8) (progn (delete-field 6) (buffer-string)) (condition-case e (field-end 'x) (error e)) (let ((m (copy-marker 2))) (field-end m))))
"###;

#[test]
fn editfns_misc_matches_pinned_gnu() {
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
