use romacs_vm::Vm;

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    vm.set_collection_floor(1);
    let expression = format!(
        "(let ((completion-test-result {source})) (if (equal completion-test-result '{expected}) t (list 'got completion-test-result)))"
    );
    let value = vm
        .eval_source(&expression)
        .unwrap_or_else(|e| panic!("{source}: {e}"));
    assert_eq!(
        value.display().unwrap(),
        "t",
        "{source}: expected {expected}"
    );
}

#[test]
fn prefix_exact_and_duplicate_completion() {
    for (source, expected) in [
        (
            r#"(list (try-completion "al" '("alpha" "alpine" "beta")) (all-completions "al" '("alpha" "alpine" "beta")) (test-completion "alpha" '("alpha" "alpine")))"#,
            r#"("alp" ("alpha" "alpine") t)"#,
        ),
        (
            r#"(list (try-completion "abc" '("abc" "abc")) (try-completion "abc" '("abc" "abcd")) (try-completion "z" '("abc")) (all-completions "" '("a" "a")) (test-completion "ab" '("abc")))"#,
            r#"(t "abc" nil ("a" "a") nil)"#,
        ),
        (
            r#"(list (try-completion "" nil) (all-completions "" nil) (test-completion "" nil) (try-completion "" '("")))"#,
            r#"(nil nil nil t)"#,
        ),
        (
            r#"(list (try-completion "a" '(9 (17 . bad) abc (abd . x) "z")) (all-completions "a" '(9 (17 . bad) abc (abd . x) "z")))"#,
            r#"("ab" ("abc" "abd"))"#,
        ),
        (
            r#"(list (try-completion "a" '("abc" . ignored)) (test-completion "abc" '("abc" . ignored)))"#,
            r#"("abc" t)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn predicates_use_original_entries_and_unwind() {
    for (source, expected) in [
        (
            r#"(let ((calls nil)) (list (all-completions "a" '((abc . 1) (abd . 2) z) (lambda (x) (setq calls (cons x calls)) (eq (cdr x) 2))) calls))"#,
            r#"(("abd") ((abd . 2) (abc . 1)))"#,
        ),
        (
            r#"(let ((calls nil)) (list (try-completion "" '("a" "b" "c") (lambda (x) (setq calls (cons x calls)) t)) calls))"#,
            r#"("" ("b" "a"))"#,
        ),
        (
            r#"(let ((calls nil)) (list (test-completion "a" '(("a" . 1) ("a" . 2)) (lambda (x) (setq calls (cons x calls)) nil)) calls))"#,
            r#"(nil (("a" . 1)))"#,
        ),
        (
            r#"(test-completion "a" '("a") (lambda (_) 'accepted))"#,
            r#"accepted"#,
        ),
        (
            r#"(let ((xs (list "ab" "ac"))) (all-completions "a" xs (lambda (_) (setcdr xs (list "ad")) (garbage-collect) t)))"#,
            r#"("ab" "ac")"#,
        ),
        (
            r#"(let ((xs (list "ab" "ac"))) (all-completions "a" xs (lambda (_) (setcar (cdr xs) "az") t)))"#,
            r#"("ab" "az")"#,
        ),
        (
            r#"(list (catch 'done (all-completions "a" '("a") (lambda (_) (throw 'done 7)))) (try-completion "a" '("a")))"#,
            r#"(7 t)"#,
        ),
        (
            r#"(list (condition-case e (all-completions "a" '("a") (lambda (_) (signal 'quit nil))) (quit (car e))) (try-completion "a" '("a")))"#,
            r#"(quit t)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn hash_obarray_and_programmed_tables() {
    for (source, expected) in [
        (
            r#"(let ((h (make-hash-table :test 'equal)) (calls nil)) (puthash "abc" 1 h) (puthash 'abd 2 h) (puthash 17 3 h) (list (all-completions "a" h (lambda (k v) (setq calls (cons (cons k v) calls)) (= v 2))) calls (try-completion "a" h)))"#,
            r#"(("abd") ((abd . 2) ("abc" . 1)) "ab")"#,
        ),
        (
            r#"(let ((o (obarray-make 3))) (intern "abc" o) (intern "abd" o) (list (try-completion "a" o) (sort (all-completions "a" o)) (test-completion "abc" o)))"#,
            r#"("ab" ("abc" "abd") t)"#,
        ),
        (
            r#"(let ((o (vector 0))) (intern "abc" o) (list (try-completion "a" o) (all-completions "a" o) (test-completion "abc" o)))"#,
            r#"("abc" ("abc") t)"#,
        ),
        (
            r#"(let ((f (lambda (s p flag) (list s p flag)))) (list (try-completion "x" f 'pred) (all-completions "x" f 'pred) (test-completion "x" f 'pred)))"#,
            r#"(("x" pred nil) ("x" pred t) ("x" pred lambda))"#,
        ),
        (
            r#"(let ((h (make-hash-table :test 'eq))) (puthash 'abc 1 h) (puthash "abc" 2 h) (test-completion "abc" h (lambda (k v) (list k v))))"#,
            r#"(abc 1)"#,
        ),
        (
            r#"(let* ((h (make-hash-table :test 'equal)) (s (copy-sequence "abc"))) (puthash 'abc 1 h) (puthash s 2 h) (test-completion "abc" h (lambda (k v) (list (eq k s) v))))"#,
            r#"(t 2)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn completion_case_regex_and_properties() {
    for (source, expected) in [
        (
            r#"(let ((completion-ignore-case t)) (list (try-completion "FO" '("foo" "FOOD")) (try-completion "foo" '("FOO" "foo")) (all-completions "A" '("abc" "Abd" "z")) (test-completion "ABC" '("abc"))))"#,
            r#"("foo" "foo" ("abc" "Abd") t)"#,
        ),
        (
            r#"(progn (string-match "b" "abc") (let ((completion-regexp-list '("b" "c$"))) (list (all-completions "a" '("abc" "abd" "aac")) (try-completion "a" '("abc" "abd")) (test-completion "abc" '("abc")) (match-beginning 0))))"#,
            r#"(("abc") "abc" t 1)"#,
        ),
        (
            r#"(let ((completion-ignore-case t) (completion-regexp-list '("B"))) (all-completions "A" '("abc" "Acc")))"#,
            r#"("abc")"#,
        ),
        (
            r#"(let ((s (propertize "abcd" 'tag t))) (list (eq s (car (all-completions "a" (list s)))) (get-text-property 0 'tag (try-completion "a" (list s)))))"#,
            r#"(t t)"#,
        ),
        (
            r#"(let ((completion-regexp-list '(17))) (list (all-completions "z" '("abc")) (condition-case e (all-completions "a" '("abc")) (error e))))"#,
            r#"(nil (wrong-type-argument stringp 17))"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn string_comparison_ranges_and_byte_domains() {
    for (source, expected) in [
        (
            r#"(list (compare-strings "abc" nil nil "abc" nil nil) (compare-strings "abc" nil nil "abd" nil nil) (compare-strings "abcd" nil nil "ab" nil nil) (compare-strings "xabc" 1 99 "abc" 0 nil))"#,
            r#"(t -3 3 t)"#,
        ),
        (
            r#"(list (compare-strings "αβγ" -2 nil "βγ" nil nil) (compare-strings "abc" 1 1 "z" 0 0) (compare-strings "aB" nil nil "Ab" nil nil t))"#,
            r#"(t t t)"#,
        ),
        (
            r#"(list (compare-strings (unibyte-string 255) nil nil "ÿ" nil nil) (compare-strings (unibyte-string 255) nil nil (string #x3fffff) nil nil))"#,
            r#"(1 t)"#,
        ),
        (
            r#"(condition-case e (compare-strings "a" 2 nil "b" nil nil) (error e))"#,
            r#"(args-out-of-range "a" 2 nil)"#,
        ),
        (
            r#"(condition-case e (compare-strings "a" nil nil 'wrong nil nil) (error e))"#,
            r#"(wrong-type-argument stringp wrong)"#,
        ),
        (
            r#"(condition-case e (compare-strings "a" 'wrong nil "b" nil nil) (error e))"#,
            r#"(wrong-type-argument integerp wrong)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn completion_preserves_native_predicates_names_and_early_results() {
    for (source, expected) in [
        (
            r#"(let ((xs (list "a"))) (setcdr xs xs) (test-completion "a" xs))"#,
            r#"t"#,
        ),
        (
            r#"(let ((xs (list "a" "b"))) (setcdr (cdr xs) xs) (try-completion "" xs))"#,
            r#""""#,
        ),
        (
            r#"(let ((s (intern "completion-probe-name"))) (eq (symbol-name s) (car (all-completions "completion-probe" (list s)))))"#,
            r#"t"#,
        ),
        (
            r#"(let ((s (intern "completion-probe-mutable"))) (aset (symbol-name s) 0 88) (list (symbol-name s) (all-completions "X" (list s))))"#,
            r#"("Xompletion-probe-mutable" ("Xompletion-probe-mutable"))"#,
        ),
        (
            r#"(eq (symbol-name 'car) (car (all-completions "car" (list 'car))))"#,
            r#"t"#,
        ),
        (
            r#"(progn (fset (quote commandp) (lambda (_) t)) (all-completions "" (quote (not-a-command)) (quote commandp)))"#,
            r#"nil"#,
        ),
        (
            r#"(let ((down (make-char-table 'case-table nil)) (up (make-char-table 'case-table nil)) (canon (make-char-table 'case-table nil)) (eqv (make-char-table 'case-table nil))) (set-char-table-extra-slot down 0 up) (set-char-table-extra-slot down 1 canon) (set-char-table-extra-slot down 2 eqv) (set-case-table down) (let ((completion-ignore-case t) (completion-regexp-list (list "A"))) (all-completions "" (list "a"))))"#,
            r#"nil"#,
        ),
        (
            r#"(let ((down (make-char-table 'case-table nil)) (up (make-char-table 'case-table nil)) (canon (make-char-table 'case-table nil)) (eqv (make-char-table 'case-table nil))) (set-char-table-extra-slot down 0 up) (set-char-table-extra-slot down 1 canon) (set-char-table-extra-slot down 2 eqv) (set-case-table down) (progn (aset canon 65 97) (let ((completion-ignore-case t) (completion-regexp-list (list "A"))) (all-completions "" (list "a")))))"#,
            r#"("a")"#,
        ),
        (
            r#"(let ((down (make-char-table 'case-table nil)) (up (make-char-table 'case-table nil)) (canon (make-char-table 'case-table nil)) (eqv (make-char-table 'case-table nil))) (set-char-table-extra-slot down 0 up) (set-char-table-extra-slot down 1 canon) (set-char-table-extra-slot down 2 eqv) (set-case-table down) (compare-strings "A" nil nil "a" nil nil t))"#,
            r#"-1"#,
        ),
        (
            r#"(list (commandp (lambda () (interactive))) (commandp (lambda () nil)) (commandp 'forward-char) (commandp 'car) (commandp [1]) (commandp [1] t) (commandp '(autoload "missing" nil t)))"#,
            r#"(t nil t nil t nil t)"#,
        ),
        (
            r#"(let ((s (intern "completion-gc-name"))) (symbol-name s) (garbage-collect) (eq (symbol-name s) (car (all-completions "completion-gc" (list s)))))"#,
            r#"t"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn unmodified_gnu_regexp_opt_uses_native_completion() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = Vm::new();
    let paths = ["gnu-lisp", "gnu-lisp/emacs-lisp", "gnu-lisp/progmodes"]
        .map(|path| format!("{:?}", root.join(path).to_str().unwrap()))
        .join(" ");
    vm.eval_source(&format!("(setq load-path '({paths}))"))
        .unwrap();
    for file in [
        "rlisp/vm/prelude.el",
        "gnu-lisp/emacs-lisp/debug-early.el",
        "gnu-lisp/emacs-lisp/byte-run.el",
        "gnu-lisp/emacs-lisp/backquote.el",
        "gnu-lisp/subr.el",
        "gnu-lisp/keymap.el",
        "gnu-lisp/emacs-lisp/pcase.el",
        "gnu-lisp/emacs-lisp/gv.el",
        "gnu-lisp/emacs-lisp/inline.el",
        "gnu-lisp/bindings.el",
        "gnu-lisp/emacs-lisp/easy-mmode.el",
        "gnu-lisp/emacs-lisp/cl-lib.el",
        "gnu-lisp/button.el",
        "gnu-lisp/emacs-lisp/derived.el",
        "gnu-lisp/obarray.el",
        "gnu-lisp/emacs-lisp/rx.el",
        "gnu-lisp/abbrev.el",
        "gnu-lisp/emacs-lisp/regexp-opt.el",
    ] {
        let source = format!("(load {:?} nil t)", root.join(file).to_str().unwrap());
        vm.eval_source(&source)
            .unwrap_or_else(|e| panic!("{file}: {e}"));
    }
    let source = r#"(let ((pattern (concat "\\`" (regexp-opt '("alpha" "alpine" "beta" "αβx" "αγy")) "\\'")))
      (mapcar (lambda (text) (if (string-match pattern text) t nil))
              '("alpha" "alpine" "beta" "αβx" "αγy" "alp" "alphas" "bet" "αδ")))"#;
    let value = vm.eval_source(source).unwrap();
    assert_eq!(value.display().unwrap(), "(t t t t t nil nil nil nil)");
    for source in [
        r#"(equal (regexp-opt-charset '(97 98 99 100 122)) "[a-dz]")"#,
        r#"(equal (regexp-opt-charset '(93 94 45)) "[]^-]")"#,
        r#"(let ((pattern (concat "\\`" (regexp-opt '("foo" "food" "fool" "bar" "baz" "αβ" "αγ")) "\\'")))
             (equal (mapcar (lambda (s) (if (string-match pattern s) t nil))
                            '("foo" "food" "fool" "bar" "baz" "αβ" "αγ" "fo" "foods" "ba" "αδ"))
                    '(t t t t t t t nil nil nil nil)))"#,
    ] {
        assert_eq!(
            vm.eval_source(source).unwrap().display().unwrap(),
            "t",
            "{source}"
        );
    }
}

#[test]
fn special_symbols_and_unibyte_commands_match_gnu() {
    for (source, expected) in [
        (
            r#"(list (symbol-name nil) (symbol-name t) (all-completions "" (list nil t)) (try-completion "n" (list nil)) (test-completion "nil" (list nil)) (test-completion "t" (list t)) (test-completion "nil" '((nil . value))) (test-completion "nil" obarray))"#,
            r#"("nil" "t" ("nil" "t") "nil" nil t t t)"#,
        ),
        (
            r#"(let ((n (symbol-name nil)) (yes (symbol-name t))) (garbage-collect) (list (eq n (symbol-name nil)) (eq yes (symbol-name t))))"#,
            r#"(t t)"#,
        ),
        (
            r#"(list (commandp (symbol-name 'abc)) (commandp (string-make-unibyte "abc")) (commandp (string-make-unibyte "abc") t) (all-completions "a" (list (symbol-name 'abc)) 'commandp))"#,
            r#"(t t nil ("abc"))"#,
        ),
        (
            r#"(progn (fset 'interactive-form (lambda (_) '(interactive))) (list (commandp (make-interpreted-closure nil '(nil) '(t) (symbol-name 'doc) nil)) (commandp (make-interpreted-closure nil '(nil) '(t) (cons (symbol-name 'file) 1) nil))))"#,
            r#"(nil nil)"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn explicit_ascii_nil_case_mappings_are_respected() {
    for (source, expected) in [
        (
            r#"(progn (aset (char-table-extra-slot (standard-case-table) 0) 97 nil) (compare-strings "A" nil nil "a" nil nil t))"#,
            r#"-1"#,
        ),
        (
            r#"(progn (aset (char-table-extra-slot (standard-case-table) 1) 65 nil) (let ((completion-ignore-case t) (completion-regexp-list '("A"))) (all-completions "" '("a"))))"#,
            r#"nil"#,
        ),
    ] {
        check(source, expected);
    }
}

#[test]
fn native_nil_ranges_keep_unicode_fallback_outside_ascii() {
    check(
        r#"(progn (set-char-table-range (char-table-extra-slot (standard-case-table) 1) '(65 . 255) nil)
      (let ((completion-ignore-case t))
        (list (let ((completion-regexp-list '("A"))) (all-completions "" '("a")))
              (let ((completion-regexp-list '("À"))) (all-completions "" '("à"))))))"#,
        r#"(nil ("à"))"#,
    );
}
