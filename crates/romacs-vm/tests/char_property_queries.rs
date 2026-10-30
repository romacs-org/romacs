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
