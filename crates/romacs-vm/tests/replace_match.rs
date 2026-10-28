use romacs_vm::Vm;

#[test]
fn replace_match_replays_pinned_gnu_values_offline() {
    let evidence: serde_json::Value = serde_json::from_str(include_str!("fixtures/replace_match_gnu.json")).unwrap();
    let cases = evidence["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 19, "pinned GNU evidence must not disappear");
    assert_eq!(cases.iter().filter(|case| case["supported"] == true).count(), 17);
    assert_eq!(evidence["regime"], "-Q --batch; text-quoting-style grave");
    for case in cases {
        if case["supported"] == true {
            check(&grave(case["source"].as_str().unwrap()), case["gnu"].as_str().unwrap());
        }
    }
}

/// SOURCE with error messages quoted as `text-quoting-style' grave quotes.
fn grave(source: &str) -> String {
    format!("(let ((text-quoting-style 'grave)) {source})")
}

fn check(source: &str, expected: &str) {
    let mut vm = Vm::new();
    let actual = vm.eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected), "REPLACE_MATCH: {source}");
}

#[test]
fn replace_match_character_ranges_and_subexpressions() {
    check("(list (fboundp 'replace-match) (functionp #'replace-match))", "(t t)");
    check(r#"(progn (string-match "-mode" "Auto-mode") (replace-match "" nil nil "Auto-mode"))"#, r#""Auto""#);
    check(r#"(progn (string-match "é\\(λ\\)" "aéλz") (list (replace-match "X" t t "aéλz") (replace-match "Y" t t "aéλz" 1) (match-data t)))"#, r#"("aXz" "aéYz" (1 3 2 3))"#);
    check(r#"(progn (string-match "b" "abc") (funcall #'replace-match "" t t "abc"))"#, r#""ac""#);
}

#[test]
fn replace_match_backreferences_and_escapes() {
    check(r#"(progn (string-match "\\(a\\)b" "zab!") (replace-match "<\\&-\\1-\\9-\\\\-\\?>" t nil "zab!"))"#, r#""z<ab-a--\\-\\?>!""#);
    check(r#"(progn (string-match "\\(a\\)b" "zab!") (replace-match "\\&" t nil "zab!" 1))"#, r#""zab!""#);
    check(&grave(r#"(progn (string-match "a" "a") (condition-case e (replace-match "\\x" t nil "a") (error e)))"#), r#"(error "Invalid use of `\\' in replacement text")"#);
    check(r#"(progn (string-match "a" "a") (let ((text-quoting-style 'curve)) (condition-case e (replace-match "\\x" t nil "a") (error e))))"#, r#"(error "Invalid use of ‘\\’ in replacement text")"#);
}

#[test]
fn replace_match_preserves_source_and_replacement_properties() {
    check(r#"(let ((s (copy-sequence "abc")) (r (copy-sequence "XY"))) (put-text-property 0 3 'face 'bold s) (put-text-property 0 2 'face 'italic r) (string-match "b" s) (let ((out (replace-match r t t s))) (list (substring-no-properties out) (get-text-property 0 'face out) (get-text-property 1 'face out) (get-text-property 3 'face out) (substring-no-properties s))))"#, r#"("aXYc" bold italic bold "abc")"#);
}

#[test]
fn replace_match_case_inference_and_initials() {
    for (text, replacement, expected) in [
        ("XX", "mIxEd woRD", "\"MIXED WORD\""),
        ("Xx", "mIxEd woRD", "\"MIxEd WoRD\""),
        ("X", "mIxEd woRD", "\"MIXED WORD\""),
        ("xX", "mIxEd woRD", "\"mIxEd woRD\""),
        ("XX", "ßéﬃ", "\"SSÉFFI\""),
        ("Xx", "ßéﬃ", "\"Sséﬃ\""),
        ("ÉΛ", "élan", "\"ÉLAN\""),
        ("Élan", "mIxEd", "\"MIxEd\""),
    ] {
        check(&format!("(progn (string-match {text:?} {text:?}) (replace-match {replacement:?} nil t {text:?}))"), expected);
    }
    check(r#"(progn (string-match "\\(a\\) XX" "a XX") (replace-match "\\1 mIxEd" nil nil "a XX"))"#, r#""a mIxEd""#);
    check(r#"(progn (string-match "a \\(XX\\)" "a XX") (replace-match "\\& mIxEd" nil nil "a XX" 1))"#, r#""a XX MIXED""#);
    check(r#"(list (upcase-initials "mIxEd woRD") (upcase-initials "ßé ﬃélan") (upcase-initials ?a))"#, r#"("MIxEd WoRD" "Ssé Ffiélan" 65)"#);
}

#[test]
fn replace_match_inference_obeys_case_tables() {
    check(r#"(progn (set-case-table (make-char-table 'case-table)) (string-match "XX" "XX") (replace-match "mixed" nil t "XX"))"#, r#""mixed""#);
    check(r#"(let ((table (copy-sequence (standard-case-table)))) (aset table ?X nil) (set-case-table table) (string-match "XX" "XX") (replace-match "mixed" nil t "XX"))"#, r#""mixed""#);
    check(r#"(let ((table (copy-sequence (standard-case-table)))) (aset table ?1 ?a) (set-case-table table) (string-match "11" "11") (replace-match "mixed" nil t "11"))"#, r#""MIXED""#);
    check(r#"(let* ((d (copy-sequence (standard-case-table))) (u (copy-sequence (char-table-extra-slot d 0)))) (aset u ?1 256) (set-char-table-extra-slot d 0 u) (set-case-table d) (string-match "Ab" "Ab") (list (aref (replace-match (unibyte-string ?1) nil t "Ab") 0) (aref (upcase (unibyte-string ?1)) 0)))"#, "(49 49)");
}

#[test]
fn replace_match_checks_types_ranges_and_arity() {
    check(r#"(progn (string-match "a" "a") (condition-case e (replace-match "x" t t "a" 30) (error (list (car e) (car (cdr e)) (car (cdr (cdr e)))))))"#, "(args-out-of-range 30 0)");
    check(r#"(progn (set-match-data '(0 1 nil nil)) (condition-case e (replace-match "x" t t "a" 1) (error e)))"#, r#"(error "replace-match subexpression does not exist" 1)"#);
    check(r#"(progn (string-match "a" "a") (list (multibyte-string-p (replace-match (string-to-multibyte "\\&") t nil (string-make-unibyte "a"))) (multibyte-string-p (replace-match (string-to-multibyte "b") t t "a"))))"#, "(nil t)");
    for (source, expected) in [
        (r#"(condition-case e (replace-match 3) (error e))"#, "(wrong-type-argument stringp 3)"),
        (r#"(condition-case e (replace-match "x" t t 3) (error e))"#, "(wrong-type-argument stringp 3)"),
        (r#"(progn (string-match "a" "a") (set-match-data nil) (condition-case e (replace-match "x" t t "a") (error e)))"#, r#"(error "replace-match subexpression does not exist" nil)"#),
        (r#"(progn (set-match-data '(0 1)) (condition-case e (replace-match "x" t t "a" 'bad) (error e)))"#, "(wrong-type-argument integerp bad)"),
        (r#"(progn (set-match-data '(0 4)) (condition-case e (replace-match "x" t t "a") (error e)))"#, "(args-out-of-range 0 4)"),
        (r#"(condition-case e (replace-match) (error (list (car e) (car (cdr (cdr e))))))"#, "(wrong-number-of-arguments 0)"),
        (r#"(condition-case e (replace-match "a" t t "b" nil 1) (error (list (car e) (car (cdr (cdr e))))))"#, "(wrong-number-of-arguments 6)"),
    ] { check(source, expected); }
}

#[test]
fn replace_match_infers_case_after_set_match_data() {
    check(r#"(progn (set-match-data '(0 1)) (replace-match "x" nil t "a"))"#, r#""x""#);
    check(r#"(progn (set-match-data '(0 1)) (replace-match "x" nil t "A"))"#, r#""X""#);
}

/// Buffer replacement (search.c Freplace_match through replace_range), and
/// the region casing it calls (casefiddle.c casify_region). One expression
/// per line; `R' lists each group's registers without the buffer.
const BUFFER_CASES: &str = r###"
(with-temp-buffer (insert "hello world") (goto-char 1) (re-search-forward "world") (list (replace-match "earth") (buffer-string) (point) (match-beginning 0) (match-end 0)))
(with-temp-buffer (insert "hello World") (goto-char 1) (re-search-forward "world" nil t) (list (let ((case-fold-search t)) (replace-match "earth")) (buffer-string) (point)))
(with-temp-buffer (insert "x HELLO y") (goto-char 1) (let ((case-fold-search t)) (re-search-forward "hello")) (list (replace-match "bye now") (buffer-string) (point) (match-beginning 0) (match-end 0)))
(with-temp-buffer (insert "x Hello World y") (goto-char 1) (re-search-forward "Hello World") (list (replace-match "bye now") (buffer-string) (point)))
(with-temp-buffer (insert "x Hello world y") (goto-char 1) (re-search-forward "Hello world") (list (replace-match "bye now") (buffer-string)))
(with-temp-buffer (insert "x X y") (goto-char 1) (re-search-forward "X") (list (replace-match "yz") (buffer-string)))
(with-temp-buffer (insert "x XX y") (goto-char 1) (re-search-forward "XX") (list (replace-match "mIxEd woRD" t) (buffer-string)))
(with-temp-buffer (insert "x XX Xx X xX ÉΛ y") (goto-char 1) (list (progn (re-search-forward "XX") (replace-match "mIxEd woRD")) (progn (re-search-forward "Xx") (replace-match "mIxEd woRD")) (progn (re-search-forward "X ") (replace-match "ab ")) (progn (re-search-forward "xX") (replace-match "mIxEd")) (progn (re-search-forward "ÉΛ") (replace-match "élan")) (buffer-string)))
(with-temp-buffer (insert "foo-bar") (goto-char 1) (re-search-forward "\\(foo\\)-\\(bar\\)") (list (replace-match "\\2+\\1[\\&]\\\\") (buffer-string) (point) (mapcar (lambda (i) (list (match-beginning i) (match-end i))) '(0 1 2))))
(with-temp-buffer (insert "abc") (goto-char 1) (looking-at "\\(a\\)\\(b\\)\\(z\\)?") (list (let ((text-quoting-style 'curve)) (condition-case e (replace-match "\\?") (error e))) (let ((text-quoting-style 'grave)) (list (condition-case e (replace-match "\\x") (error e)) (condition-case e (replace-match "x\\") (error e)))) (buffer-string) (replace-match "<\\3>") (buffer-string) (point)))
(with-temp-buffer (insert "abc") (set-match-data (list 1 2)) (list (let ((text-quoting-style 'grave)) (condition-case e (replace-match "\\9" t) (error e))) (buffer-string) (progn (set-match-data (list 1 2 nil nil)) (replace-match "<\\1>" t)) (buffer-string)))
(with-temp-buffer (insert "abc") (goto-char 1) (re-search-forward "a") (list (let ((text-quoting-style 'grave)) (condition-case e (replace-match "x\\9" t) (error e))) (buffer-string)))
(with-temp-buffer (insert "abc") (string-match "\\(a\\)\\(b\\)" "ab") (list (replace-match "<\\1>" t nil nil 2) (buffer-string) (point)))
(with-temp-buffer (insert "abcdef") (narrow-to-region 4 7) (set-match-data (list 5 6 1 2)) (list (replace-match "<\\1>" t) (progn (widen) (buffer-string))))
(with-temp-buffer (insert "zb") (goto-char 1) (re-search-forward "\\(a\\)?b") (list (replace-match "[\\1]") (buffer-string) (progn (goto-char 1) (re-search-forward "\\(q\\)?\\]") (condition-case e (replace-match "z" t t nil 1) (error e))) (buffer-string)))
(with-temp-buffer (insert "abc def") (goto-char 1) (re-search-forward "\\(b\\)c \\(d\\)") (list (replace-match "XYZ" t t nil 1) (buffer-string) (point) (mapcar (lambda (i) (list (match-beginning i) (match-end i))) '(0 1 2))))
(with-temp-buffer (insert "abcd") (goto-char 1) (re-search-forward "\\(a\\)\\(b\\)\\(c\\)") (list (replace-match "WXYZ" t t nil 2) (buffer-string) (point) (mapcar (lambda (i) (list (match-beginning i) (match-end i))) '(0 1 2 3)) (replace-match "" t t nil 2) (buffer-string) (mapcar (lambda (i) (list (match-beginning i) (match-end i))) '(0 1 2 3))))
(with-temp-buffer (insert "abcdef") (goto-char 1) (looking-at "bc\\|abc") (goto-char 2) (list (replace-match "XY") (buffer-string) (point) (progn (set-match-data (list 4 6)) (goto-char 1) (replace-match "QRS")) (point) (progn (set-match-data (list 1 2)) (goto-char (point-max)) (replace-match "")) (point) (buffer-string)))
(with-temp-buffer (insert "abcdef") (let ((m1 (copy-marker 3)) (m2 (copy-marker 5)) (m3 (copy-marker 2)) (m4 (copy-marker 5 t)) (m5 (copy-marker 2 t))) (set-match-data (list 2 5)) (replace-match "WXYZ" t t) (list (buffer-string) (point) (mapcar #'marker-position (list m1 m2 m3 m4 m5)))))
(with-temp-buffer (insert (propertize "abc" 'face 'bold) "def") (goto-char 1) (re-search-forward "bcd") (list (replace-match (propertize "XY" 'p 1) t t) (buffer-string) (progn (goto-char 1) (re-search-forward "XY") (replace-match (propertize "\\&!" 'p 2) t)) (buffer-string)))
(with-temp-buffer (insert "abc def") (goto-char 1) (re-search-forward "def") (replace-match (propertize "Ghi" 'face 'bold)) (re-search-backward "abc") (replace-match (propertize "XyZ" 'face 'italic) nil t) (buffer-string))
(with-temp-buffer (buffer-enable-undo) (insert "abc def") (undo-boundary) (goto-char 1) (re-search-forward "def") (setq buffer-undo-list nil) (list (replace-match "XYZW" t t) buffer-undo-list (buffer-string)))
(with-temp-buffer (buffer-enable-undo) (insert "abc Def") (undo-boundary) (goto-char 1) (re-search-forward "Def") (setq buffer-undo-list nil) (list (replace-match "xyz") buffer-undo-list (buffer-string)))
(with-temp-buffer (buffer-enable-undo) (insert "a DEF b") (undo-boundary) (goto-char 1) (re-search-forward "DEF") (setq buffer-undo-list nil) (list (replace-match "xyz") buffer-undo-list (buffer-string)))
(with-temp-buffer (insert "abcdef") (narrow-to-region 2 4) (list (progn (set-match-data (list 1 3)) (condition-case e (replace-match "x" t t) (error e))) (progn (set-match-data (list 3 5)) (condition-case e (replace-match "x" t t) (error e))) (progn (set-match-data (list 2 4)) (replace-match "QQQ" t t)) (point-min) (point-max) (progn (widen) (buffer-string))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abc def") (goto-char 1) (re-search-forward "\\(b\\)c") (list (replace-match "<\\1\\&>") (buffer-string) (multibyte-string-p (buffer-string)) (point) (progn (goto-char 1) (re-search-forward "def") (replace-match "ghi")) (buffer-string)))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abc Def") (goto-char 1) (re-search-forward "Def") (list (replace-match "xyz") (buffer-string) (progn (goto-char 1) (re-search-forward "abc") (replace-match (string-to-multibyte "q\\&") t)) (append (buffer-string) nil)))
(with-temp-buffer (insert "aéλz") (goto-char 1) (re-search-forward "é\\(λ\\)") (list (replace-match "<\\1Ω>" t) (buffer-string) (point) (match-beginning 0) (match-end 0)))
(with-temp-buffer (insert "xyz") (string-match "b" "abc") (list (replace-match "Q" t t) (buffer-string) (point)))
(with-temp-buffer (insert "xyz") (string-match "y" "y") (set-match-data nil) (list (condition-case e (replace-match "Q") (error e)) (buffer-string)))
(with-temp-buffer (insert "abc") (goto-char 1) (looking-at "b*") (list (replace-match "Q") (buffer-string) (point)))
(with-temp-buffer (insert "abc") (goto-char 1) (looking-at "ab") (setq buffer-read-only t) (list (condition-case e (replace-match "Q") (error e)) (let ((inhibit-read-only t)) (replace-match "Q")) (buffer-string)))
(with-temp-buffer (insert "A Abc") (goto-char 1) (re-search-forward "A Abc") (list (replace-match "") (buffer-string) (point)))
(with-temp-buffer (insert "foo") (goto-char 1) (re-search-forward "foo") (list (condition-case e (replace-match 'x) (error e)) (condition-case e (replace-match "x" t t nil 'bad) (error e)) (buffer-string)))
(with-temp-buffer (insert "foo bar baz") (goto-char 1) (list (re-search-forward "\\(foo\\) \\(bar\\)") (replace-match "\\2 \\1") (buffer-string) (point) (mapcar (lambda (i) (list (match-beginning i) (match-end i))) '(0 1 2))))
(with-temp-buffer (insert "foo bar baz") (goto-char 1) (list (re-search-forward "Bar" nil t) (let ((case-fold-search t)) (re-search-forward "BAR" nil t)) (replace-match "QuX") (buffer-string)))
(with-temp-buffer (insert "aßbc") (let ((m (copy-marker 2))) (goto-char 3) (list (upcase-region 1 3) (buffer-string) (point) (marker-position m))))
(with-temp-buffer (insert "foo bar-baz qUUx") (goto-char 5) (list (upcase-initials-region 1 (point-max)) (buffer-string) (point) (upcase-region 9 5) (buffer-string) (point)))
(with-temp-buffer (insert (propertize "abc" 'face 'bold) "déf") (let ((m (copy-marker 3)) (n (copy-marker 4 t))) (goto-char 4) (list (upcase-region 1 7) (buffer-string) (point) (marker-position m) (marker-position n))))
(with-temp-buffer (insert "ɐxⱥy") (put-text-property 1 2 'face 'bold) (put-text-property 3 4 'face 'italic) (let ((m (copy-marker 2)) (n (copy-marker 3)) (o (copy-marker 4))) (goto-char 4) (list (upcase-region 1 5) (buffer-string) (point) (mapcar #'marker-position (list m n o)) (buffer-size) (position-bytes 5))))
(with-temp-buffer (insert "ɐbc ⱥbc") (goto-char 3) (list (upcase-initials-region 1 8) (buffer-string) (point) (position-bytes (point))))
(with-temp-buffer (buffer-enable-undo) (insert "foo bar") (undo-boundary) (setq buffer-undo-list nil) (list (upcase-initials-region 1 8) buffer-undo-list (buffer-string) (progn (setq buffer-undo-list nil) (upcase-initials-region 1 8)) buffer-undo-list (progn (setq buffer-undo-list nil) (upcase-region 1 1)) buffer-undo-list (progn (upcase-region 2 5) buffer-undo-list) (buffer-string)))
(with-temp-buffer (insert "abcdef") (narrow-to-region 2 4) (list (condition-case e (upcase-region 1 3) (error (list (car e) (cdr (cdr e))))) (condition-case e (upcase-region 2 'x) (error e)) (upcase-region 4 2) (progn (widen) (buffer-string))))
(with-temp-buffer (insert "'abc 'def") (list (upcase-initials-region 1 (point-max)) (buffer-string) (progn (erase-buffer) (insert "'abc 'def") (let ((table (copy-syntax-table))) (modify-syntax-entry ?' "w" table) (set-syntax-table table) (upcase-initials-region 1 (point-max)) (buffer-string))) (progn (erase-buffer) (insert "'abc 'def") (modify-syntax-entry ?' "w p") (upcase-initials-region 1 (point-max)) (buffer-string))))
(with-temp-buffer (set-buffer-multibyte nil) (insert "abc d\377f") (list (upcase-region 1 (point-max)) (append (buffer-string) nil) (progn (erase-buffer) (insert "abc def") (upcase-initials-region 1 (point-max))) (buffer-string)))
(with-temp-buffer (insert "abc") (setq buffer-read-only t) (list (condition-case e (upcase-region 1 4) (error e)) (condition-case e (upcase-region 2 2) (error e)) (buffer-string)))
(list (commandp 'upcase-region) (commandp 'upcase-initials-region) (interactive-form 'upcase-region) (interactive-form 'upcase-initials-region))
"###;

/// Buffer cases whose change hooks run Lisp, which needs the Lisp prelude.
const BUFFER_HOOK_CASES: &str = r###"
(with-temp-buffer (insert "abc Def ghi") (goto-char 1) (re-search-forward "Def") (let ((log nil)) (set (make-local-variable 'before-change-functions) (list (lambda (b e) (setq log (cons (list 'before b e (buffer-string)) log))))) (set (make-local-variable 'after-change-functions) (list (lambda (b e l) (setq log (cons (list 'after b e l (buffer-string)) log))))) (list (replace-match "xyz") (reverse log) (progn (setq log nil) (goto-char 1) (re-search-forward "abc") (replace-match "QQ" t t)) (reverse log) (point))))
(with-temp-buffer (insert "foo bar") (let ((log nil)) (set (make-local-variable 'before-change-functions) (list (lambda (b e) (setq log (cons (list 'before b e) log))))) (set (make-local-variable 'after-change-functions) (list (lambda (b e l) (setq log (cons (list 'after b e l) log))))) (list (upcase-initials-region 1 8) (reverse log) (progn (setq log nil) (upcase-initials-region 1 8)) (reverse log) (progn (setq log nil) (upcase-region 3 6)) (reverse log) (buffer-string))))
(with-temp-buffer (insert "abc def") (goto-char 1) (re-search-forward "\\(b\\)c \\(de\\)") (let ((log nil)) (set (make-local-variable 'after-change-functions) (list (lambda (b e l) (setq log (cons (list b e l (match-beginning 0) (match-end 0) (match-beginning 2)) log))))) (list (replace-match "Q" t t nil 1) (reverse log) (match-beginning 2) (match-end 2) (point))))
(with-temp-buffer (insert "one TWO three") (goto-char 1) (re-search-forward "TWO") (set (make-local-variable 'after-change-functions) (list (lambda (_b _e _l) (save-excursion (goto-char 1) (looking-at "o"))))) (list (replace-match "deux") (buffer-string) (point) (match-beginning 0) (match-end 0)))
(with-temp-buffer (insert "abc") (defvar region-extract-function nil) (let ((region-extract-function (lambda (_) '((1 . 2) (3 . 4))))) (list (upcase-region 1 4 t) (buffer-string) (upcase-initials-region 1 1 nil) (condition-case e (let ((region-extract-function (lambda (_) '(1)))) (upcase-region 1 4 t)) (error e)) (buffer-string))))
(with-temp-buffer (insert "x foo bar y") (let ((str "x foo bar y") (answer "foo bar")) (goto-char 1) (search-forward answer nil t) (replace-match (mapconcat (lambda (c) (if (memq c '(?\s)) (char-to-string c) (propertize (char-to-string c) 'face 'bold))) answer "") nil t) (list (buffer-string) (point) str)))
(with-temp-buffer (insert "x Foo y") (let ((answer "foo")) (goto-char 1) (let ((case-fold-search t)) (search-forward answer nil t)) (replace-match (mapconcat (lambda (c) (propertize (char-to-string c) 'face 'bold)) answer "") nil t) (list (buffer-string) (point))))
"###;

#[test]
fn replace_match_in_buffers_matches_pinned_gnu() {
    matches_gnu(BUFFER_CASES);
}

#[test]
fn replace_match_buffer_hooks_match_pinned_gnu() {
    matches_gnu(BUFFER_HOOK_CASES);
}

/// Replace each string with properties in a value by its text and the
/// properties at each character.
const FLATTEN: &str = r#"(defun romacs-test-flatten (value)
  (cond ((and (stringp value) (or (next-property-change 0 value) (text-properties-at 0 value)))
         (let ((i 0) (props nil))
           (while (< i (length value))
             (setq props (cons (text-properties-at i value) props) i (1+ i)))
           (list 'props (substring-no-properties value) (nreverse props))))
        ((consp value) (cons (romacs-test-flatten (car value)) (romacs-test-flatten (cdr value))))
        (t value)))"#;

/// Each expression's printed value, as character codes (as tests/json.rs
/// compares), in a VM with the Lisp prelude. A string's text properties
/// are compared character by character: GNU prints its interval tree,
/// whose splits Romacs's property runs do not keep.
fn matches_gnu(cases: &str) {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in cases.lines().filter(|line| !line.is_empty()) {
        let source = format!("(append (prin1-to-string (condition-case e (romacs-test-flatten {expression}) (error (list 'signal e)))) nil)");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", FLATTEN, "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let mut vm = Vm::new();
        vm.install_lisp_prelude().unwrap();
        vm.eval_source(FLATTEN).unwrap();
        let actual = vm.eval_source(&source).and_then(|v| v.display());
        if actual.as_deref() != Ok(expected.as_str()) {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {:?}", text(&expected), actual.as_deref().map(text)));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A character whose case is several characters (casefiddle.c's special
/// casing) replaces it as one character for several: its text properties
/// cover them all and markers after it follow the text. GNU's
/// replace_range_2 leaves markers at their character positions and gives
/// the extra characters the properties around them (DIVERGENCES.md
/// VM2-CASE3).
#[test]
fn region_case_expansion_keeps_properties_and_follows_markers() {
    check(
        r#"(with-temp-buffer (insert "aßbc") (put-text-property 2 3 'face 'bold) (let ((m (copy-marker 3)) (n (copy-marker 5))) (goto-char 4) (list (upcase-region 1 5) (prin1-to-string (buffer-string)) (marker-position m) (marker-position n) (point))))"#,
        r##"(nil "#(\"ASSBC\" 1 3 (face bold))" 4 6 5)"##,
    );
    check(
        r#"(with-temp-buffer (insert "ﬁx ﬁ") (list (upcase-initials-region 1 5) (buffer-string)))"#,
        r#"(nil "Fix Fi")"#,
    );
}
