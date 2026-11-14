//! `get-char-property-and-overlay' (textprop.c:695) and
//! `barf-if-buffer-read-only' (buffer.c:2465), which org and gnosis call on
//! every insertion and edit, must answer as pinned GNU does.
use romacs_vm::Vm;

fn agree(source: &str) {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(&oracle).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
    assert!(output.status.success(), "{source}: {}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let actual = Vm::new().eval_source(source).and_then(|v| v.display());
    assert_eq!(actual.as_deref(), Ok(expected.as_str()), "{source}");
}

#[test]
fn get_char_property_and_overlay_names_the_supplying_overlay() {
    agree("(with-temp-buffer (insert \"abcdef\") (put-text-property 1 3 'face 'bold) \
           (let ((low (make-overlay 2 5)) (high (make-overlay 3 4))) \
             (overlay-put low 'face 'italic) (overlay-put high 'face 'underline) (overlay-put high 'priority 5) \
             (list (get-char-property-and-overlay 1 'face) \
                   (eq (cdr (get-char-property-and-overlay 2 'face)) low) \
                   (eq (cdr (get-char-property-and-overlay 3 'face)) high) \
                   (car (get-char-property-and-overlay 3 'face)) \
                   (get-char-property-and-overlay 6 'face) (get-char-property-and-overlay 7 'face) \
                   (get-char-property-and-overlay 1 'face \"xyz\") \
                   (get-char-property-and-overlay 0 'face (propertize \"xy\" 'face 'bold)))))");
}

/// A position outside the accessible region signals as GNU's does: the
/// position as both ends for `get-text-property` and `text-properties-at`
/// (`validate_interval_range`), alone for `get-char-property`, a marker kept as
/// the marker.
#[test]
fn positions_outside_the_restriction_signal_as_gnu() {
    agree("(with-temp-buffer (insert (make-string 200 ?x)) (narrow-to-region 100 150) \
           (let ((m (copy-marker 50 t)) (buffer (current-buffer))) \
             (list (condition-case e (get-text-property 50 'foo) (args-out-of-range (cdr e))) \
                   (condition-case e (get-text-property m 'foo) (args-out-of-range (cdr e))) \
                   (condition-case e (get-text-property 160 'foo) (args-out-of-range (cdr e))) \
                   (condition-case e (text-properties-at 50) (args-out-of-range (cdr e))) \
                   (condition-case e (text-properties-at m) (args-out-of-range (cdr e))) \
                   (condition-case e (text-properties-at 50 buffer) (args-out-of-range (cdr e))) \
                   (condition-case e (get-char-property 50 'foo) (args-out-of-range (cdr e))) \
                   (condition-case e (get-char-property m 'foo) (args-out-of-range (cdr e))) \
                   (condition-case e (get-char-property 300 'foo) (args-out-of-range (cdr e))) \
                   (condition-case e (get-pos-property 50 'foo) (args-out-of-range (cdr e))) \
                   (condition-case e (next-single-property-change 50 'foo) (args-out-of-range (cdr e))))))");
}

#[test]
fn barf_if_buffer_read_only_honours_variables_and_text_properties() {
    agree("(with-temp-buffer (insert \"abcdef\") (put-text-property 2 3 'inhibit-read-only t) \
           (let ((buffer-read-only t)) \
             (list (barf-if-buffer-read-only 2) \
                   (condition-case e (barf-if-buffer-read-only 4) (buffer-read-only (car e))) \
                   (let ((inhibit-read-only t)) (barf-if-buffer-read-only 4)) \
                   (progn (goto-char 2) (barf-if-buffer-read-only)) \
                   (condition-case e (barf-if-buffer-read-only 99) (error e)) \
                   (condition-case e (barf-if-buffer-read-only 'x) (error e)) \
                   (let ((buffer-read-only nil)) (barf-if-buffer-read-only 99)))))");
}

#[test]
fn get_char_property_and_overlay_through_a_window_sees_its_overlays() {
    agree("(let ((buffer (get-buffer-create \"cpo-window\")) (other-window (minibuffer-window))) \
             (set-window-buffer (selected-window) buffer) \
             (with-current-buffer buffer (erase-buffer) (insert \"abcdef\") \
               (let ((mine (make-overlay 2 5)) (other (make-overlay 2 5)) (loose (make-overlay 4 6))) \
                 (overlay-put mine 'face 'bold) (overlay-put mine 'window (selected-window)) \
                 (overlay-put other 'face 'italic) (overlay-put other 'window other-window) (overlay-put other 'priority 9) \
                 (overlay-put loose 'face 'underline) (overlay-put loose 'window t) (overlay-put loose 'priority 20) \
                 (list (car (get-char-property-and-overlay 3 'face (selected-window))) \
                       (eq (cdr (get-char-property-and-overlay 3 'face (selected-window))) mine) \
                       (car (get-char-property-and-overlay 3 'face)) \
                       (car (get-char-property-and-overlay 5 'face (selected-window)))))))");
}

/// Foverlay_get is lookup_char_property: a `category' symbol supplies a
/// missing property, and `char-property-alias-alist' an alternative.
#[test]
fn overlay_get_follows_category_and_aliases() {
    agree("(with-temp-buffer (insert \"abcdef\") \
           (put 'cpo-category 'face 'bold) \
           (let ((o (make-overlay 2 4)) (char-property-alias-alist '((mouse-face help-echo)))) \
             (overlay-put o 'category 'cpo-category) (overlay-put o 'help-echo \"tip\") \
             (list (overlay-get o 'face) (overlay-get o 'mouse-face) (overlay-get o 'absent) \
                   (get-char-property 2 'face) (let ((hit (get-char-property-and-overlay 2 'mouse-face))) (list (car hit) (eq (cdr hit) o))) \
                   (progn (setplist 'cpo-category '(face . bold)) (overlay-get o 'face)))))");
}

#[test]
fn barf_if_buffer_read_only_in_a_unibyte_buffer() {
    agree("(with-temp-buffer (set-buffer-multibyte nil) (insert \"abcdef\") (put-text-property 2 3 'inhibit-read-only t) \
           (let ((buffer-read-only t)) \
             (list (barf-if-buffer-read-only 2) \
                   (condition-case e (barf-if-buffer-read-only 4) (buffer-read-only (car e))) \
                   (condition-case e (barf-if-buffer-read-only 99) (error e)) \
                   (condition-case e (barf-if-buffer-read-only 0) (error e)))))");
}

// `font-lock-mode' makes `char-property-alias-alist' buffer-local, so an
// Eshell prompt's `font-lock-face' reads as its `face' (font-core.el:165).
#[test]
fn buffer_local_property_aliases_and_defaults_apply() {
    agree("(with-temp-buffer (insert \"ab\") (put-text-property 1 2 'font-lock-face 'bold) \
           (let ((o (make-overlay 2 3))) (overlay-put o 'font-lock-face 'italic) \
             (set (make-local-variable 'char-property-alias-alist) '((face font-lock-face))) \
             (set (make-local-variable 'default-text-properties) '(p 7)) \
             (list (get-text-property 1 'face) (get-char-property 2 'face) (overlay-get o 'face) \
                   (text-property-any 1 3 'face 'bold) (next-single-property-change 1 'face) (get-text-property 2 'p) \
                   (with-temp-buffer (insert (propertize \"x\" 'font-lock-face 'bold)) \
                     (list (get-text-property 1 'face) (get-text-property 1 'p))))))");
}

/// `text-property-any`, `text-property-not-all` and
/// `next-single-property-change` read a property as `get-text-property`
/// does, through `category`, `char-property-alias-alist` and
/// `default-text-properties`, in buffers, narrowed buffers and strings.
#[test]
fn single_property_scans_answer_as_gnu() {
    agree("(with-temp-buffer (insert \"αβγδεζηθικλμνξοπρστυφχψω0123456789\") \
           (put 'cat-a 'mark 'from-category) (put 'cat-b 'other 1) \
           (put-text-property 3 6 'mark 'direct) (put-text-property 8 11 'category 'cat-a) \
           (put-text-property 11 13 'category 'cat-b) (put-text-property 15 18 'alias 'through-alias) \
           (put-text-property 18 20 'mark nil) (put-text-property 5 16 'face 'bold) \
           (let ((char-property-alias-alist '((mark alias other-alias))) (results nil)) \
             (dolist (default-text-properties '(nil (mark by-default))) \
               (dolist (value '(nil direct from-category through-alias by-default absent)) \
                 (push (list (text-property-any 1 35 'mark value) (text-property-not-all 1 35 'mark value) \
                             (text-property-any 4 9 'mark value) (text-property-not-all 9 4 'mark value) \
                             (text-property-any 20 20 'mark value)) results)) \
               (dolist (from '(1 3 5 6 8 10 11 13 15 17 18 20 34 35)) \
                 (push (list (next-single-property-change from 'mark) (next-single-property-change from 'mark nil 12) \
                             (next-single-property-change from 'mark nil 2) (next-single-property-change from 'mark nil 99) \
                             (next-single-property-change from 'face) (next-single-property-change from 'absent nil 30)) results))) \
             (narrow-to-region 9 19) \
             (push (list (text-property-any 9 19 'mark 'through-alias) (text-property-not-all 9 19 'mark 'from-category) \
                         (next-single-property-change 9 'mark) (next-single-property-change 17 'mark) \
                         (next-single-property-change 19 'mark) (next-single-property-change 17 'mark nil 30) \
                         (condition-case e (text-property-any 1 19 'mark t) (args-out-of-range (cdr e)))) results) \
             (nreverse results)))");
    agree("(let ((s (concat \"αβγ\" (propertize \"δεζ\" 'mark 'x 'face 'bold) \"ηθι\" (propertize \"κλ\" 'category 'cat-s) \"μν\"))) \
           (put 'cat-s 'mark 'x) \
           (list (text-property-any 0 13 'mark 'x s) (text-property-not-all 3 13 'mark 'x s) (text-property-any 6 13 'mark 'x s) \
                 (text-property-any 0 13 'mark nil s) (text-property-not-all 0 3 'mark nil s) \
                 (next-single-property-change 0 'mark s) (next-single-property-change 3 'mark s) \
                 (next-single-property-change 6 'mark s) (next-single-property-change 9 'mark s 10) \
                 (next-single-property-change 11 'mark s) (next-single-property-change 11 'mark s 12) \
                 (next-single-property-change 13 'mark s) (text-property-any 0 3 'mark nil \"abc\") \
                 (next-single-property-change 0 'mark \"abc\" 2)))");
}
