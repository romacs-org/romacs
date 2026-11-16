//! `\cX' and `\CX' in regexps test the current buffer's category table,
//! as GNU's CHAR_HAS_CATEGORY does: the standard categories characters.el
//! sets, and one defined at run time in a buffer's own table. Against
//! pinned GNU, in batch sessions.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

fn batch(binary: &str, dir: &std::path::Path, form: &str) -> String {
    let mut command = Command::new(binary);
    command.args(["-Q", "--batch", "--eval", &format!("(prin1 {form})")]).current_dir(dir).env("HOME", dir).env("TMPDIR", dir);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn regexp_categories_read_the_category_table_as_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-regexp-categories-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = r#"(list
      (mapcar (lambda (case) (string-match (car case) (cdr case)))
              '(("\\c^" . "️") ("\\`\\c.\\c^+" . "x️") ("\\cg" . "abα") ("\\cj" . "xあ")
                ("\\cl" . "é") ("\\C." . "x́") ("\\cc" . "中") ("\\cg" . "abc")))
      (with-temp-buffer
        (let ((table (copy-category-table)))
          (define-category ?Z "probe" table)
          (modify-category-entry ?q ?Z table)
          (set-category-table table)
          (list (string-match "\\cZ" "abq") (string-match "\\CZ" "q") (string-match "\\cZ" "abc")))))"#;
    let expected = batch(&gnu, &dir, form);
    assert!(expected.starts_with("((0 0 2"), "{expected}");
    assert_eq!(batch(romacs, &dir, form), expected);
    std::fs::remove_dir_all(&dir).unwrap();
}
