//! `string-width` of emoji sequences, as a batch session reads them,
//! against pinned GNU: a run that a `compose-gstring-and-emoji' rule of
//! `composition-function-table' matches (VS-16, ZWJ, modifier and flag
//! sequences) is two columns, compose-gstring-and-emoji-for-terminal's
//! width; other compositions keep their characters' widths.
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
fn emoji_sequences_are_as_wide_as_gnu_counts_them() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-string-width-emoji-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = r#"(list
      (mapcar #'string-width
              (list "❤️" "❤" "☺️" "\U0001F44D\U0001F3FD" "\U0001F1FA\U0001F1F8"
                    "\U0001F468‍\U0001F469‍\U0001F467" "é" "ก่" "1️⃣"
                    "가" "\U0001F600" "a‍b" "\U0001F3F3️‍\U0001F308" "x️" "é"
                    "️" "a️️" "\U0001F468‍\U0001F469‍\U0001F467x" "plain ascii"))
      (string-width "ab❤️cd" 2 4) (string-width "ab❤️cd" 3 4)
      (string-width "\U0001F468‍\U0001F469‍\U0001F467 Zoë Müller ❤️"))"#;
    let expected = batch(&gnu, &dir, form);
    assert!(expected.starts_with("((2 1 2 2 2 2 1 1 2 2 2 2 2 2 1"), "{expected}");
    assert_eq!(batch(romacs, &dir, form), expected);
    std::fs::remove_dir_all(&dir).unwrap();
}
