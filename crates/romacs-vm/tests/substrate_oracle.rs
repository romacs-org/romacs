//! Runtime substrate GNU's own test suite needs, against pinned GNU.
//! Each line is one expression, evaluated after GNU's loadup chain; both
//! sides run in this process's environment, so collation compares under
//! the same locale.
use romacs_vm::Vm;
use std::path::Path;

/// The loadup chain (find_charset.rs's), for buffers and coding systems.
fn bootstrapped() -> Vm {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('='))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

fn matches_pinned_gnu(cases: &str) {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut vm = bootstrapped();
    let mut failures = Vec::new();
    for expression in cases.lines().filter(|line| !line.is_empty()) {
        // As character codes, as tests/json.rs compares.
        let source = format!("(append (prin1-to-string (condition-case e {expression} (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn obarray_clear_matches_pinned_gnu() {
    matches_pinned_gnu(r###"
(let ((ob (obarray-make))) (intern "a" ob) (intern "b" ob) (list (obarray-clear ob) (intern-soft "a" ob) (let (n) (mapatoms (lambda (s) (push s n)) ob) n) (progn (intern "c" ob) (intern-soft "c" ob)) (unintern "c" ob) (intern-soft "c" ob)))
(let ((ob (obarray-make 1000))) (dotimes (i 100) (intern (format "s%d" i) ob)) (obarray-clear ob) (dotimes (i 100) (intern (format "t%d" i) ob)) (let ((n 0)) (mapatoms (lambda (_) (setq n (1+ n))) ob) (list n (intern-soft "s5" ob) (symbol-name (intern-soft "t5" ob)))))
(condition-case e (obarray-clear [0]) (error e))
(condition-case e (obarray-clear nil) (error e))
"###);
}

#[test]
fn compare_buffer_substrings_matches_pinned_gnu() {
    matches_pinned_gnu(r###"
(with-temp-buffer (insert "abcXYZ") (let ((b (current-buffer))) (with-temp-buffer (insert "abcxyz") (list (compare-buffer-substrings b 1 7 nil 1 7) (let ((case-fold-search nil)) (compare-buffer-substrings b 1 7 nil 1 7)) (compare-buffer-substrings b 1 3 nil 1 7) (compare-buffer-substrings nil 1 7 b 1 3) (compare-buffer-substrings b 7 1 nil nil nil) (compare-buffer-substrings b (copy-marker 2) 4 (current-buffer) 2 4)))))
(with-temp-buffer (insert "héllo") (let ((b (current-buffer))) (with-temp-buffer (insert "hÉllo") (list (compare-buffer-substrings b nil nil nil nil nil) (let ((case-fold-search nil)) (compare-buffer-substrings b nil nil nil nil nil)) (progn (narrow-to-region 2 4) (compare-buffer-substrings nil nil nil b 2 4))))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "a\377b") (let ((b (current-buffer))) (with-temp-buffer (insert "a" (unibyte-string 255) "c") (list (compare-buffer-substrings b nil nil nil nil nil) (compare-buffer-substrings b 1 3 nil 1 3)))))
(with-temp-buffer (insert "abc") (list (condition-case e (compare-buffer-substrings nil 0 3 nil 1 2) (error e)) (condition-case e (compare-buffer-substrings "no such" 1 3 nil 1 2) (error e)) (condition-case e (compare-buffer-substrings 'x 1 3 nil 1 2) (error e)) (condition-case e (compare-buffer-substrings nil 1 9 nil 1 2) (error e)) (let ((b (generate-new-buffer "dead"))) (kill-buffer b) (condition-case e (compare-buffer-substrings b 1 1 nil 1 2) (error e)))))
"###);
}

#[test]
fn string_collation_matches_pinned_gnu() {
    matches_pinned_gnu(r###"
(list (string-collate-lessp "a" "B") (string-collate-lessp "a" "B" nil t) (string-collate-equalp "A" "a" nil t) (string-collate-equalp "A" "a") (string-collate-lessp 'abc "abd") (string-collate-lessp "a\0b" "a\0a") (string-collate-equalp "a\0b" "a\0a"))
(list (string-collate-lessp "é" "f" "POSIX") (string-collate-lessp "XYZZY" "xyzzy" "POSIX") (string-collate-equalp "xyzzy" "XYZZY" "C" t) (string-collate-lessp (unibyte-string 200) (unibyte-string 100)) (sort (list "11" "12" "1 1" "1 2" "1.1" "1.2") #'string-collate-lessp))
(list (condition-case e (string-collate-lessp "a" "b" "no_SUCH.locale") (error e)) (condition-case e (string-collate-lessp 1 "b") (error e)) (condition-case e (string-collate-equalp "a" "b" 'x) (error e)))
"###);
}

#[test]
fn process_table_matches_pinned_gnu() {
    // Values that differ between two processes (times, sizes, the
    // command) are compared by kind; init's are compared as values.
    matches_pinned_gnu(r###"
(let ((a (process-attributes (emacs-pid)))) (list (mapcar #'car a) (mapcar (lambda (k) (type-of (alist-get k a))) '(euid user egid group comm state ppid ttname minflt utime start etime pcpu pri nice thcount vsize rss pmem args)) (equal (alist-get 'euid a) (user-uid)) (equal (alist-get 'user a) (user-login-name)) (> (alist-get 'thcount a) 0)))
(let ((a (process-attributes 1))) (mapcar (lambda (k) (alist-get k a)) '(euid user egid group comm ppid pgrp sess ttname tpgid args)))
(list (process-attributes 999999999) (condition-case e (process-attributes "1") (error e)) (condition-case e (process-attributes 1.5) (error e)) (car (process-attributes 1.0)) (condition-case e (process-attributes (expt 2 70)) (error e)))
(let ((pids (list-system-processes))) (list (and (memq (emacs-pid) pids) t) (and (memq 1 pids) t) (seq-every-p #'natnump pids)))
"###);
}
