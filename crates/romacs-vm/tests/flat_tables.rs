//! Case tables and syntax tables are read as flat rows
//! (`char_table::flat`): what the primitives that read them answer must
//! stay pinned GNU's, through every way a table's contents change. Each
//! case answers in integers and symbols, so no output encoding takes part.
use romacs_vm::Vm;

/// `syn' makes a syntax table under PARENT (the standard one for nil) of
/// SPECS; `under' calls F with TABLE the buffer's; `fold' compares two
/// strings ignoring case. `gnu-cases' gives the standard case tables the
/// entries GNU's characters.el gives five characters, which a VM that has
/// not loaded it reads from Unicode's simple mappings instead.
const PRELUDE: &str = r"(progn
 (defalias 'syn (lambda (parent &rest specs) (let ((st (make-char-table 'syntax-table))) (set-char-table-parent st (or parent (standard-syntax-table))) (while specs (modify-syntax-entry (car specs) (car (cdr specs)) st) (setq specs (cdr (cdr specs)))) st)))
 (defalias 'under (lambda (table f) (let ((old (syntax-table))) (set-syntax-table table) (prog1 (funcall f) (set-syntax-table old)))))
 (defalias 'fold (lambda (a b) (compare-strings a nil nil b nil nil t)))
 (defalias 'gnu-cases (lambda () (let ((down (standard-case-table)) (up (char-table-extra-slot (standard-case-table) 0))) (aset up #x17f #x17f) (aset down #x212a #x212a) (aset up #xdf #x1e9e) (aset down #x130 #x130) (aset up #x131 #x131))))
 EXPR)";

fn agree(source: &str) {
    let source = PRELUDE.replace("EXPR", source);
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")])
        .env("LC_ALL", "C.UTF-8").stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let actual = Vm::new().eval_source(&source).and_then(romacs_vm::Local::display);
    assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{source}");
}

/// `Fcompare_strings` upcases only the characters that differ, and answers
/// with the index of the pair that decides.
#[test]
fn compare_strings_folds_as_gnu() {
    agree("(let ((f (lambda (a b &rest r) (compare-strings a (nth 0 r) (nth 1 r) b (nth 2 r) (nth 3 r) t)))) \
             (list (funcall f \"hello world\" \"HELLO WORLD\") (funcall f \"hello\" \"HELLP\") (funcall f \"Hellp\" \"hELLO\") \
                   (funcall f \"abc\" \"ABCD\") (funcall f \"ABCD\" \"abc\") (funcall f \"\" \"\") (funcall f \"\" \"a\") \
                   (funcall f \"xxabcxx\" \"ABCD\" 2 5 0 3) (funcall f \"xxabcxx\" \"ABCD\" 2 -2 nil nil) \
                   (funcall f \"xxabcxx\" \"ABDD\" 2 5 0 3) (funcall f \"abc\" \"ABC\" 0 99 0 99) \
                   (compare-strings \"abc\" nil nil \"ABC\" nil nil) (compare-strings \"ABC\" nil nil \"abc\" nil nil) \
                   (condition-case e (funcall f \"abc\" \"ABC\" 4 nil) (error (car e))) \
                   (condition-case e (funcall f 'abc \"ABC\") (error e)) \
                   (condition-case e (funcall f \"abc\" 7) (error e))))");
    // Greek, Cyrillic, CJK, final sigma, and the characters whose Unicode
    // uppercase is ASCII: U+017F (S) and U+212A (itself), first as
    // characters.el leaves them, then with the table's entries changed.
    agree("(progn (gnu-cases) \
             (let ((up (char-table-extra-slot (standard-case-table) 0)) (out nil)) \
               (push (list (fold \"αβγδ\" \"ΑΒΓΔ\") (fold \"αβγδ\" \"ΑΒΓΕ\") (fold \"привет\" \"ПРИВЕТ\") \
                           (fold \"привет\" \"ПРИВЕД\") (fold \"日本語\" \"日本語\") (fold \"日本語\" \"日本誤\") \
                           (fold \"aλ日x\" \"AΛ日X\") (fold \"ς\" \"σ\") (fold \"ς\" \"Σ\") \
                           (fold \"\u{17f}\" \"s\") (fold \"\u{17f}\" \"S\") (fold \"\u{212a}\" \"k\") \
                           (fold \"\u{212a}\" \"K\") (fold \"ß\" \"SS\") (fold \"ǆ\" \"Ǆ\") (fold \"ǅ\" \"Ǆ\") \
                           (fold \"é\" \"É\") (fold \"ÿ\" \"Ÿ\") (fold \"i\" \"İ\") (fold \"ı\" \"I\")) out) \
               (aset up #x17f ?S) (aset up #x212a ?K) \
               (push (list (fold \"\u{17f}\" \"s\") (fold \"\u{17f}\" \"S\") (fold \"\u{212a}\" \"k\") (fold \"\u{212a}\" \"K\") \
                           (fold \"a\u{17f}b\u{212a}\" \"ASBK\") (fold \"a\u{17f}b\u{212a}\" \"ASBL\") (upcase ?\u{17f}) (upcase ?\u{212a})) out) \
               (aset up #x17f #x17f) (aset up #x212a #x212a) (aset up ?k nil) \
               (push (list (fold \"\u{17f}\" \"S\") (fold \"\u{212a}\" \"K\") (fold \"k\" \"K\") (upcase ?\u{17f}) (upcase ?k)) out) \
               (nreverse out)))");
}

/// A unibyte string's bytes above ASCII are raw bytes, which no case
/// table folds; a unibyte buffer cases a character below 256 as a byte.
#[test]
fn compare_strings_folds_bytes_as_gnu() {
    agree("(list (fold \"\\351\" \"\\311\") (fold \"a\\351\" \"A\\351\") (fold \"a\\351\" \"A\\311\") \
                 (fold \"\\351\" \"é\") (fold \"é\" \"\\351\") (fold (string-to-multibyte \"a\\351\") \"A\\351\") \
                 (with-temp-buffer (set-buffer-multibyte nil) \
                   (list (fold \"é\" \"É\") (fold \"\\351\" \"\\311\") (fold \"a\\351\" \"A\\351\") \
                         (fold \"λ\" \"Λ\") (fold \"abc\" \"ABD\") (upcase ?\\351) (upcase ?a) (downcase ?\\311) \
                         (append (upcase \"a\\351z\") nil) (append (upcase \"aéz\") nil))))");
}

#[test]
fn assoc_string_folds_as_gnu() {
    agree("(let ((keys '((\"Alpha\" . 1) (beta . 2) \"Gamma\" (\"ΔΕΛΤΑ\" . 4) (\"épsilon\" . 5) 7 (nil . 8) (t . 9)))) \
             (list (assoc-string \"alpha\" keys t) (assoc-string \"alpha\" keys) (assoc-string \"BETA\" keys t) \
                   (assoc-string 'beta keys) (assoc-string \"gamma\" keys t) (assoc-string \"GAMMA\" (cdr (cdr keys)) t) \
                   (cdr (assoc-string \"δελτα\" keys t)) (cdr (assoc-string \"ÉPSILON\" keys t)) (assoc-string \"NIL\" keys t) \
                   (assoc-string 'T keys t) (assoc-string \"zeta\" keys t) (assoc-string \"alph\" keys t) \
                   (assoc-string \"alphaa\" keys t) (condition-case e (assoc-string 7 keys t) (error e)) \
                   (assoc-string 7 '(1 2) t) (assoc-string \"épsilon\" keys) (cdr (assoc-string 'ÉPSILON keys t))))");
}

/// The buffer's own case table folds, as `set-case-table` set it and as
/// later writes to it and to a parent leave it.
#[test]
fn compare_strings_reads_the_buffers_case_table_as_gnu() {
    agree("(let ((out nil)) \
             (with-temp-buffer \
               (let ((d (make-char-table 'case-table)) (up (make-char-table 'case-table)) (parent (make-char-table 'case-table))) \
                 (aset d ?A ?a) (aset d ?Q ?z) \
                 (set-case-table d) \
                 (push (list (fold \"a\" \"A\") (fold \"q\" \"Q\") (fold \"z\" \"Q\") (fold \"b\" \"B\") \
                             (upcase ?z) (downcase ?Q) (upcase ?b) (append (upcase \"azb\") nil)) out) \
                 (aset (char-table-extra-slot d 0) ?b ?B) \
                 (push (list (fold \"b\" \"B\") (upcase ?b)) out) \
                 (set-char-table-parent up parent) \
                 (aset parent ?m ?M) (aset up ?n ?N) \
                 (set-char-table-extra-slot d 0 up) \
                 (set-case-table d) \
                 (push (list (fold \"m\" \"M\") (fold \"n\" \"N\") (fold \"b\" \"B\") (upcase ?m)) out) \
                 (aset parent ?m nil) (aset parent ?o ?O) (aset parent ?λ ?Λ) \
                 (push (list (fold \"m\" \"M\") (fold \"o\" \"O\") (upcase ?o) (append (upcase \"mno\") nil) (fold \"λ\" \"Λ\")) out) \
                 (aset parent ?λ nil) \
                 (push (fold \"λ\" \"Λ\") out) \
                 (with-temp-buffer (push (list (fold \"m\" \"M\") (fold \"q\" \"Q\") (upcase ?m)) out)) \
                 (push (fold \"q\" \"Q\") out))) \
             (push (fold \"q\" \"Q\") out) \
             (let ((s (standard-case-table))) \
               (aset (char-table-extra-slot s 0) ?q ?!) \
               (push (list (fold \"q\" \"Q\") (fold \"q\" \"!\") (upcase ?q)) out) \
               (aset (char-table-extra-slot s 0) ?q ?Q) \
               (push (list (fold \"q\" \"Q\") (upcase ?q)) out)) \
             (nreverse out))");
}

#[test]
fn case_conversion_reads_tables_as_gnu() {
    agree("(progn (gnu-cases) \
             (list (append (upcase \"hello, αβγ ς привет 日本 \u{17f}\u{212a}ßǆ\") nil) \
                   (append (downcase \"HELLO, ΑΒΓ Σ ПРИВЕТ 日本 \u{212a}İǅ\") nil) \
                   (upcase ?α) (downcase ?Ω) (upcase ?я) (downcase ?Я) (upcase ?日) (upcase ?\u{17f}) (downcase ?\u{212a}) \
                   (upcase ?\u{212a}) (upcase ?ß) (upcase ?ǆ) (downcase ?ǅ) (upcase ?ÿ) (upcase ?µ) (downcase ?İ) \
                   (upcase (+ ?a (ash 1 27))) (upcase 4194303) (upcase 4194175) \
                   (with-temp-buffer (insert \"abc αβγ ǆ\") (upcase-region 1 (point-max)) (append (buffer-string) nil)) \
                   (with-temp-buffer (insert \"ABC ΑΒΓ Σ\") (downcase-region 1 (point-max)) (append (buffer-string) nil))))");
}

/// The syntax table's own entries, a parent's where it has none, and each
/// later write to either; a table set for one call and dropped.
#[test]
fn syntax_reads_follow_table_and_parent_writes_as_gnu() {
    agree("(with-temp-buffer \
             (insert \"foo.bar-baz αβγ_日本 x\") \
             (let* ((parent (syn nil)) (s (syn parent)) (out nil) \
                    (walk (lambda () (goto-char 1) (list (skip-syntax-forward \"w\") (skip-syntax-forward \"^w\") \
                                                     (skip-syntax-forward \"w_\") (progn (goto-char (point-max)) (skip-syntax-backward \"w\")) \
                                                     (skip-syntax-backward \"^w\") (skip-syntax-backward \"w_.\") \
                                                     (char-syntax ?.) (char-syntax ?-) (char-syntax ?α) (char-syntax ?日) (char-syntax ?_))))) \
               (set-syntax-table s) \
               (push (funcall walk) out) \
               (modify-syntax-entry ?. \"w\" s) \
               (push (funcall walk) out) \
               (modify-syntax-entry ?- \"_\" parent) \
               (push (funcall walk) out) \
               (modify-syntax-entry ?α \".\" parent) (modify-syntax-entry '(?β . ?γ) \"_\" s) \
               (push (funcall walk) out) \
               (modify-syntax-entry ?. \"@\" s) \
               (push (funcall walk) out) \
               (set-char-table-parent s (standard-syntax-table)) \
               (push (funcall walk) out) \
               (push (under (syn nil) walk) out) \
               (push (under (syn nil ?a \".\") walk) out) \
               (push (under (syn nil ?b \".\") walk) out) \
               (push (under (syn (syn nil ?f \".\") ?o \"_\") walk) out) \
               (push (funcall walk) out) \
               (push (list (progn (goto-char 1) (forward-word 2) (point)) (scan-sexps 1 1) (scan-sexps 1 2) \
                           (string-match \"\\\\w+\" \"foo.bar-baz\") (match-end 0) \
                           (string-match \"\\\\s_+\" \"foo.bar-baz_\") (match-end 0)) out) \
               (nreverse out)))");
}

/// A descriptor is a cons the table shares: GNU reads its car at every
/// character, so a change made in place is read at once.
#[test]
fn a_syntax_descriptor_altered_in_place_reads_as_gnu() {
    agree("(with-temp-buffer \
             (insert \"a.b;c\") \
             (let ((s (syn nil)) (entry (list 1)) (out nil) \
                   (walk (lambda () (goto-char 1) (list (skip-syntax-forward \"w\") (char-syntax ?.) \
                                                    (string-match \"\\\\w+\" \"a.b\") (match-end 0) (scan-sexps 1 1) \
                                                    (progn (goto-char 1) (forward-word 1) (point)))))) \
               (aset s ?. entry) \
               (set-syntax-table s) \
               (push (funcall walk) out) \
               (setcar entry 2) \
               (push (funcall walk) out) \
               (setcar entry 3) \
               (push (funcall walk) out) \
               (setcar entry 1) \
               (push (funcall walk) out) \
               (nreverse out)))");
}

/// A unibyte buffer's bytes above ASCII read the table at their raw-byte
/// characters.
#[test]
fn syntax_reads_in_a_unibyte_buffer_as_gnu() {
    agree("(with-temp-buffer \
             (set-buffer-multibyte nil) \
             (insert \"ab\\351\\352 cd\\377\") \
             (let ((s (syn nil)) (out nil) \
                   (walk (lambda () (goto-char 1) (list (skip-syntax-forward \"w\") (skip-syntax-forward \"^w\") \
                                                    (progn (goto-char (point-max)) (skip-syntax-backward \"w\")) \
                                                    (char-syntax ?\\351) (char-syntax ?a))))) \
               (set-syntax-table s) \
               (push (funcall walk) out) \
               (modify-syntax-entry (unibyte-char-to-multibyte ?\\351) \".\" s) \
               (push (funcall walk) out) \
               (modify-syntax-entry ?a \".\" s) \
               (push (funcall walk) out) \
               (nreverse out)))");
}
