//! The text-property writers (textprop.c `add_text_properties_1`,
//! `set_text_properties`, `Fremove_text_properties`,
//! `Fremove_list_of_text_properties`) must answer as pinned GNU does: their
//! return values, the buffer's modified state and counts, its undo records,
//! its hooks and its read-only refusals, over text whose runs already have
//! the properties, have others, or have them in part.
//!
//! GNU leaves intervals unmerged, which `object-intervals` and the undo
//! records of a later change show; a case reads each character's properties
//! and records undo on text whose intervals are its runs.
use romacs_vm::Vm;

/// Each character's properties of the current buffer or of a string.
const PLISTS: &str = "(defun tpw-plists (&optional object) \
    (let ((n (if (stringp object) (length object) (buffer-size))) (first (if (stringp object) 0 1)) (out nil)) \
      (dotimes (i n) (push (text-properties-at (+ first i) object) out)) \
      (nreverse out)))";

fn agree(body: &str) {
    let source = format!("(progn {PLISTS} {body})");
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{body}: {}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    let actual = vm.eval_source(&source).and_then(romacs_vm::Local::display);
    assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{body}");
}

#[test]
fn buffer_writes_return_whether_a_run_changed() {
    agree("(with-temp-buffer (insert \"abcdefghij\") \
           (list (put-text-property 1 4 'face 'bold) \
                 (add-text-properties 1 4 '(face bold)) \
                 (add-text-properties 1 6 '(face bold)) \
                 (add-text-properties 2 3 '(face italic)) \
                 (add-text-properties 1 6 '(face bold x 1)) \
                 (add-text-properties 1 6 '(x 1 face bold)) \
                 (add-text-properties 3 5 '(x 1 y 2)) \
                 (add-text-properties 1 1 '(z 1)) \
                 (add-text-properties 4 2 '(z 1)) \
                 (remove-text-properties 7 9 '(face nil)) \
                 (remove-text-properties 3 9 '(face nil)) \
                 (remove-text-properties 1 11 '(q nil r nil)) \
                 (remove-list-of-text-properties 1 11 '(q r)) \
                 (remove-list-of-text-properties 1 11 '(q y)) \
                 (remove-list-of-text-properties 1 11 nil) \
                 (remove-text-properties 1 11 nil) \
                 (tpw-plists)))");
    agree("(with-temp-buffer (insert \"abcdefghij\") \
           (list (set-text-properties 1 4 nil) \
                 (set-text-properties 1 4 '(a 1 b 2)) \
                 (set-text-properties 1 4 '(a 1 b 2)) \
                 (set-text-properties 2 6 '(b 3)) \
                 (set-text-properties 5 5 '(c 3)) \
                 (set-text-properties 8 10 nil) \
                 (set-text-properties 1 3 nil) \
                 (tpw-plists)))");
}

#[test]
fn string_writes_return_whether_a_run_changed() {
    agree("(let ((s (copy-sequence \"abcdefghij\"))) \
           (list (put-text-property 0 3 'face 'bold s) \
                 (add-text-properties 0 3 '(face bold) s) \
                 (add-text-properties 0 5 '(face bold) s) \
                 (add-text-properties 1 2 '(face italic x 1) s) \
                 (remove-text-properties 6 8 '(face nil) s) \
                 (remove-text-properties 2 8 '(face nil) s) \
                 (remove-list-of-text-properties 0 10 '(q r) s) \
                 (remove-list-of-text-properties 0 10 '(q x) s) \
                 (set-text-properties 0 2 '(a 1) s) \
                 (set-text-properties 0 2 '(a 1) s) \
                 (set-text-properties 4 6 nil s) \
                 (tpw-plists s) \
                 (set-text-properties 0 10 nil s) \
                 (object-intervals s) \
                 (set-text-properties 0 10 nil s)))");
    // Characters of several bytes: the positions are characters.
    agree("(let ((s (copy-sequence \"aλ🙂zλb\"))) \
           (list (add-text-properties 1 3 '(p 1) s) (add-text-properties 2 5 '(p 1) s) (add-text-properties 1 5 '(p 1) s) \
                 (remove-text-properties 3 4 '(p nil) s) (tpw-plists s) \
                 (with-temp-buffer (insert s) \
                   (list (add-text-properties 2 4 '(q 2)) (remove-text-properties 1 3 '(p nil q nil)) (tpw-plists)))))");
}

/// A write that changes nothing leaves the buffer unmodified and its counts
/// where they were; one that changes a run advances the modification count
/// and not the character count.
#[test]
fn a_write_modifies_the_buffer_only_when_a_run_changes() {
    agree("(with-temp-buffer (insert \"abcdefghij\") (set-buffer-modified-p nil) \
           (let ((log nil) (m (buffer-modified-tick)) (c (buffer-chars-modified-tick))) \
             (dolist (f (list (lambda () (put-text-property 1 4 'face 'bold)) \
                              (lambda () (put-text-property 1 4 'face 'bold)) \
                              (lambda () (add-text-properties 2 3 '(face bold))) \
                              (lambda () (remove-text-properties 5 9 '(face nil))) \
                              (lambda () (remove-text-properties 2 9 '(face nil))) \
                              (lambda () (set-text-properties 6 8 nil)) \
                              (lambda () (set-text-properties 1 2 nil)) \
                              (lambda () (remove-list-of-text-properties 1 11 '(face))) \
                              (lambda () (remove-list-of-text-properties 1 11 '(face))))) \
               (set-buffer-modified-p nil) \
               (funcall f) \
               (push (list (buffer-modified-p) (- (buffer-modified-tick) m) (- (buffer-chars-modified-tick) c)) log) \
               (setq m (buffer-modified-tick) c (buffer-chars-modified-tick))) \
             (nreverse log)))");
}

/// Undo records name each interval a change touched and the value it had,
/// in the order GNU walks them; a change of nothing records nothing, and a
/// buffer that keeps no undo records nothing.
#[test]
fn a_write_records_each_changed_run_for_undo() {
    agree("(mapcar (lambda (change) \
             (with-temp-buffer (insert \"abcdefghij\") (put-text-property 3 6 'face 'bold) (put-text-property 5 9 'x 1) \
               (buffer-enable-undo) (set-buffer-modified-p nil) (setq buffer-undo-list nil) \
               (list (funcall change) buffer-undo-list (buffer-modified-p)))) \
           (list (lambda () (add-text-properties 2 10 '(face bold x 2))) \
                 (lambda () (add-text-properties 3 6 '(face bold))) \
                 (lambda () (put-text-property 4 7 'face 'bold)) \
                 (lambda () (put-text-property 1 11 'q nil)) \
                 (lambda () (remove-text-properties 4 8 '(face nil q nil))) \
                 (lambda () (remove-text-properties 1 3 '(face nil))) \
                 (lambda () (remove-list-of-text-properties 1 11 '(x q))) \
                 (lambda () (set-text-properties 3 9 '(q 1 x 2))) \
                 (lambda () (set-text-properties 5 6 nil)) \
                 (lambda () (set-text-properties 1 3 nil)) \
                 (lambda () (add-face-text-property 2 7 'italic)) \
                 (lambda () (let ((buffer-undo-list t)) (put-text-property 1 11 'face 'italic)))))");
}

#[test]
fn read_only_text_and_buffers_refuse_a_write() {
    agree("(with-temp-buffer (insert \"abcdefg\") (put-text-property 2 4 'read-only t) \
           (list (condition-case e (put-text-property 1 3 'face 'bold) (error e)) \
                 (condition-case e (put-text-property 4 6 'face 'bold) (error e)) \
                 (condition-case e (remove-text-properties 3 4 '(read-only nil)) (error e)) \
                 (let ((inhibit-read-only t)) (put-text-property 1 3 'face 'bold)) \
                 (let ((inhibit-read-only '(t))) (condition-case e (put-text-property 1 3 'x 1) (error e))) \
                 (let ((inhibit-read-only '(u))) (condition-case e (put-text-property 1 3 'y 1) (error e))) \
                 (let ((buffer-read-only t)) (condition-case e (put-text-property 4 6 'x 1) (error (car e)))) \
                 (let ((buffer-read-only t)) (condition-case e (put-text-property 4 6 'face 'bold) (error (car e)))) \
                 (let ((buffer-read-only t) (inhibit-read-only t)) (put-text-property 4 6 'x 1)) \
                 (let ((default-text-properties '(read-only dflt))) (condition-case e (put-text-property 5 6 'z 1) (error e))) \
                 (let ((char-property-alias-alist '((read-only ro)))) \
                   (put-text-property 6 7 'ro 'aliased) (condition-case e (put-text-property 6 7 'z 1) (error e))) \
                 (progn (put 'tpw-ro 'read-only 'by-category) (put-text-property 7 8 'category 'tpw-ro) \
                   (condition-case e (put-text-property 7 8 'z 1) (error e))) \
                 (tpw-plists)))");
}

/// The change hooks run around a write that changes a run, with its range;
/// `inhibit-modification-hooks` stops them and leaves `deactivate-mark`,
/// as `with-silent-modifications` relies on.
#[test]
fn change_hooks_bracket_a_write_that_changes_a_run() {
    agree("(with-temp-buffer (insert \"abcdef\") (set-buffer-modified-p nil) \
           (let* ((log nil) \
                  (before-change-functions (list (lambda (b e) (push (list 'b b e) log)))) \
                  (after-change-functions (list (lambda (b e l) (push (list 'a b e l) log))))) \
             (put-text-property 2 5 'face 'bold) (put-text-property 2 5 'face 'bold) \
             (add-text-properties 1 3 '(face bold)) \
             (remove-text-properties 1 7 '(nope nil)) \
             (set-text-properties 5 7 nil) \
             (remove-list-of-text-properties 1 3 '(face)) \
             (let ((inhibit-modification-hooks t)) (put-text-property 1 3 'x 1)) \
             (setq deactivate-mark nil) \
             (let ((m (buffer-modified-p)) (inhibit-modification-hooks t) (buffer-undo-list t)) \
               (put-text-property 1 6 'y 2) (restore-buffer-modified-p m)) \
             (list (nreverse log) (buffer-modified-p) deactivate-mark (tpw-plists))))");
    agree("(with-temp-buffer (insert \"abcdef\") \
           (let ((log nil)) \
             (put-text-property 2 4 'modification-hooks (list (lambda (b e) (push (list b e) log)))) \
             (put-text-property 1 6 'face 'bold) (put-text-property 5 6 'x 1) \
             (let ((inhibit-modification-hooks t)) (put-text-property 1 6 'face 'italic)) \
             (list (nreverse log) (get-text-property 3 'face))))");
}

#[test]
fn written_properties_stick_to_inherited_insertions() {
    agree("(with-temp-buffer (insert \"abcdef\") (put-text-property 2 4 'face 'bold) (put-text-property 3 5 'k 1) \
           (put-text-property 2 4 'rear-nonsticky '(face)) \
           (goto-char 4) (insert-and-inherit \"X\") (goto-char 2) (insert-and-inherit \"Y\") \
           (tpw-plists))");
}

#[test]
fn unibyte_buffers_take_the_same_writes() {
    agree("(with-temp-buffer (set-buffer-multibyte nil) (insert \"abcdef\") \
           (list (put-text-property 1 3 'face 'bold) (add-text-properties 1 3 '(face bold)) (add-text-properties 2 5 '(face bold)) \
                 (remove-text-properties 4 6 '(face nil)) (get-text-property 2 'face) (get-text-property 4 'face) \
                 (next-single-property-change 1 'face) (set-text-properties 1 2 nil) (get-text-property 1 'face) \
                 (buffer-modified-p) (tpw-plists)))");
}

/// A unibyte buffer's character reads its properties in GNU's plist
/// order, newest first, whatever order their symbols were made in: after
/// writes, an inheriting insertion, a copy to a string and a change to
/// multibyte.
#[test]
fn unibyte_buffers_keep_each_plist_in_order() {
    agree("(with-temp-buffer (set-buffer-multibyte nil) (insert \"abcd\") \
           (put-text-property 1 3 'zz 1) (put-text-property 1 3 'aa 2) (put-text-property 1 2 'zz 5) \
           (set-text-properties 3 4 '(m 1 b 2 z 3)) \
           (goto-char 3) (insert-and-inherit (propertize \"x\" 'n 1 'zz 7)) \
           (list (tpw-plists) (tpw-plists (buffer-substring 1 5)) \
                 (progn (set-buffer-multibyte t) (tpw-plists))))");
}

/// A `syntax-table` property written, removed or replaced by any of the
/// writers reaches the scanner's next reading.
#[test]
fn syntax_table_writes_reach_the_scanner() {
    // `q' is a word character unless its `syntax-table' property makes it
    // a string quote: the sexp at 1 is then the string \"q bc q\".  The
    // buffer has no `syntax-propertize-function', so nothing is left to
    // propertize, as `syntax-propertize' itself would record.
    agree("(with-temp-buffer (insert \"q bc q x\") (setq syntax-propertize--done most-positive-fixnum) \
           (let ((parse-sexp-lookup-properties t) (delimiter (string-to-syntax \"\\\"\"))) \
             (list (scan-sexps 1 1) \
                   (progn (put-text-property 1 2 'syntax-table delimiter) (put-text-property 6 7 'syntax-table delimiter) (scan-sexps 1 1)) \
                   (progn (remove-text-properties 6 7 '(syntax-table nil)) (condition-case e (scan-sexps 1 1) (error (car e)))) \
                   (progn (set-text-properties 6 7 (list 'syntax-table delimiter)) (scan-sexps 1 1)) \
                   (progn (set-text-properties 1 9 nil) (scan-sexps 1 1)))))");
}

#[test]
fn add_face_text_property_combines_each_run() {
    agree("(with-temp-buffer (insert \"abcdef\") (put-text-property 2 4 'face 'bold) \
           (list (add-face-text-property 1 5 'italic) (add-face-text-property 3 6 'underline t) \
                 (add-face-text-property 1 2 'italic) (tpw-plists)))");
}

/// `get-text-property` reads the property, else its category's, an alias's
/// or the default, and signals outside the text.
#[test]
fn get_text_property_reads_category_alias_and_default() {
    agree("(progn (put 'tpw-cat 'face 'cat-face) \
           (with-temp-buffer (insert \"abcdef\") (put-text-property 1 3 'category 'tpw-cat) \
             (put-text-property 2 5 'alt 'alt-face) (put-text-property 5 6 'face nil) \
             (let ((char-property-alias-alist '((face alt other))) (default-text-properties '(face dflt q 7))) \
               (list (mapcar (lambda (p) (get-text-property p 'face)) '(1 2 3 4 5 6 7)) \
                     (mapcar (lambda (p) (get-text-property p 'q)) '(1 6)) \
                     (get-text-property 1 'nope) \
                     (condition-case e (get-text-property 8 'face) (error e)) \
                     (condition-case e (get-text-property 0 'face) (error e))))))");
}

/// A key is its symbol, compared as `eq` compares: an uninterned symbol
/// is another key than the interned symbol of its name.
#[test]
fn an_uninterned_symbol_is_its_own_key() {
    agree("(let* ((u (make-symbol \"face\")) (s (propertize \"abc\" u 1 'face 2))) \
           (list (get-text-property 0 u s) (get-text-property 0 'face s) (length (text-properties-at 0 s)) \
                 (eq (car (memq u (text-properties-at 0 s))) u) \
                 (remove-text-properties 0 3 (list u nil) s) (get-text-property 0 u s) (get-text-property 0 'face s) \
                 (with-temp-buffer (insert \"abc\") \
                   (list (add-text-properties 1 3 (list u 5 'face 6)) (add-text-properties 1 3 (list u 5)) \
                         (get-text-property 1 u) (get-text-property 1 'face) (next-single-property-change 1 u) \
                         (remove-list-of-text-properties 1 4 (list u)) (get-text-property 1 u) (get-text-property 1 'face)))))");
}

/// No text has a property keyed by an object that is no symbol, so reading
/// one answers nil, or the default GNU's `eq` lookup finds for it.
#[test]
fn a_property_that_is_no_symbol_reads_as_nil() {
    agree("(with-temp-buffer (insert \"abc\") (put-text-property 1 3 'face 'bold) \
           (list (get-text-property 1 7) (get-text-property 1 \"s\") (get-text-property 0 '(a) (propertize \"x\" 'face 'bold)) \
                 (let ((default-text-properties '(7 seven))) (get-text-property 1 7))))");
}
