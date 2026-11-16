//! `equal' and `sxhash-equal' on char-tables against pinned GNU in batch:
//! two tables are equal by what they hold (purpose, default, parent, extra
//! slots, each character's value), so a full keymap is equal to its copy
//! and an `equal' hash table finds a table by an equal one.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

fn batch(binary: &str, dir: &std::path::Path, form: &str) -> String {
    let mut command = Command::new(binary);
    command.args(["-Q", "--batch", "--eval", &format!("(princ (prin1-to-string {form}))")]).current_dir(dir).env("HOME", dir).env("TMPDIR", dir);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn char_tables_are_equal_by_what_they_hold() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-char-table-equal-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = r#"(list
  (equal (make-char-table 'x) (make-char-table 'x))
  (let ((a (make-char-table 'x))) (equal a (copy-sequence a)))
  (let ((a (make-keymap))) (define-key a "q" 'ignore) (equal a (copy-keymap a)))
  (let ((a (make-keymap)) (b (make-keymap))) (define-key a "q" 'ignore) (equal a b))
  (let ((a (make-char-table 'x)) (b (make-char-table 'x))) (aset a ?q "v") (aset b ?q (copy-sequence "v")) (equal a b))
  (let ((a (make-char-table 'x)) (b (make-char-table 'x))) (aset a ?q 1) (aset b ?q 2) (equal a b))
  (let ((a (make-char-table 'x)) (b (make-char-table 'x))) (set-char-table-range a '(?a . ?z) 1) (dotimes (i 26) (aset b (+ ?a i) 1)) (equal a b))
  (let ((a (make-char-table 'x)) (b (make-char-table 'x))) (aset a #x10000 1) (aset b #x10000 1) (equal a b))
  (let ((a (make-char-table 'x)) (b (make-char-table 'x))) (aset a #x3fff00 1) (equal a b))
  (equal (make-char-table 'x) (make-char-table 'y))
  (equal (make-char-table 'x 0) (make-char-table 'x 1))
  (let ((a (make-char-table 'x)) (b (make-char-table 'x))) (set-char-table-parent a (make-char-table 'p)) (equal a b))
  (let ((a (make-char-table 'x)) (p (make-char-table 'p))) (set-char-table-parent a p) (equal a (copy-sequence a)))
  (progn (put 'ct-extra 'char-table-extra-slots 2)
         (let ((a (make-char-table 'ct-extra)) (b (make-char-table 'ct-extra)))
           (list (equal a b) (progn (set-char-table-extra-slot a 1 'z) (equal a b)))))
  (let* ((a (make-keymap)) (c (make-composed-keymap (list a)))) (keymap-set a "q" #'ignore) (equal c (copy-keymap c)))
  (= (sxhash-equal (make-char-table 'x)) (sxhash-equal (make-char-table 'x)))
  (let ((h (make-hash-table :test 'equal)) (k (make-char-table 'x)))
    (aset k ?a 1) (puthash k 'found h)
    (let ((other (make-char-table 'x))) (aset other ?a 1) (gethash other h)))
  ;; A compressed Unicode property block equals the same values expanded.
  (let* ((a (unicode-property-table-internal 'general-category)) (b (copy-sequence a)))
    (list (equal a b)
          (progn (put-unicode-property-internal b 128 (get-unicode-property-internal b 128)) (equal a b)))))"#;
    let expected = batch(&gnu, &dir, form);
    assert_eq!(expected, "(t t t nil t nil t t nil nil nil nil t (t nil) t t found (t t))");
    assert_eq!(batch(romacs, &dir, form), expected);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Two tables that hold themselves are equal, and comparing them is one
/// walk: GNU's comparison of them does not finish, so Romacs alone is run.
#[test]
fn tables_that_hold_themselves_compare_once() {
    let dir = std::env::temp_dir().join(format!("romacs-char-table-self-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = "(let ((a (make-char-table 'x)) (b (make-char-table 'x))) (set-char-table-range a t a) (set-char-table-range b t b) (aset a ?q a) (aset b ?q b) (equal a b))";
    assert_eq!(batch(installed::romacs(), &dir, form), "t");
    std::fs::remove_dir_all(&dir).unwrap();
}
