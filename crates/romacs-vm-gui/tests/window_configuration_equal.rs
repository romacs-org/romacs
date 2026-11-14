//! window-configuration-equal-p compares layouts on the real batch host:
//! whatever point is, and with a window recreated in the same place counting as
//! the same (window.c compare_window_configurations).  The expected answers are
//! pinned GNU's.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

const FORMS: &str = "
(let ((a (current-window-configuration)))
  (princ (list (window-configuration-equal-p a a)
               (window-configuration-equal-p a (current-window-configuration))
               (compare-window-configurations a (current-window-configuration))
               (with-temp-buffer (window-configuration-equal-p a (current-window-configuration)))))
  (insert \"abc\") (goto-char 1)
  (princ (window-configuration-equal-p a (current-window-configuration)))
  (split-window)
  (princ (window-configuration-equal-p a (current-window-configuration)))
  (setq a (current-window-configuration))
  (delete-window (next-window))
  (split-window)
  (princ (window-configuration-equal-p a (current-window-configuration)))
  (princ (condition-case e (window-configuration-equal-p a 1) (error e)))
  (princ (condition-case e (window-configuration-equal-p 1 a) (error e))))";

#[test]
fn window_configurations_compare_by_layout() {
    let home = std::env::temp_dir().join(format!("romacs-window-config-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let output = Command::new(installed::romacs())
        .args(["-Q", "--batch", "--eval", &format!("(progn {FORMS})")])
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &home)
        .env("XDG_CACHE_HOME", &home)
        .env("XDG_DATA_HOME", &home)
        .env("XDG_STATE_HOME", &home)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&home).unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "(t t t nil)tnilt(wrong-type-argument window-configuration-p 1)(wrong-type-argument window-configuration-p 1)");
}
