//! GNU's C variables, as a batch session reads them, against pinned GNU:
//! the same value, or the reason Romacs's differs.
use std::process::Command;

/// Printed alike in both sessions.
const SAME: &[&str] = &[
    "buffer-access-fontify-functions", "dynamic-library-alist", "echo-area-clear-hook", "emacs-copyright",
    "face-ignored-fonts", "font-log", "font-slant-table", "font-weight-table", "font-width-table", "fontset-alias-alist",
    "inhibit-compacting-font-caches", "inhibit-load-charset-map", "internal-make-interpreted-closure-function",
    "load-force-doc-strings", "load-source-file-function", "mouse-position-function", "mwheel-coalesce-scroll-events",
    "operating-system-release", "redisplay--inhibit-bidi", "tool-bar-button-margin", "tool-bar-max-label-size",
];

fn batch(binary: &str, dir: &std::path::Path, form: &str, romacs: bool) -> String {
    let mut command = Command::new(binary);
    command.args(["-Q", "--batch", "--eval", &format!("(prin1 {form})")]).current_dir(dir).env("HOME", dir).env("TMPDIR", dir);
    if romacs { command.env("ROMACS_WORLD", "off"); }
    let output = command.output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn c_variables_read_as_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = env!("CARGO_BIN_EXE_romacs-vm-gui");
    let dir = std::env::temp_dir().join(format!("romacs-defvar-values-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = format!("(mapcar (lambda (s) (list s (special-variable-p s) (symbol-value s))) '({}))", SAME.join(" "));
    assert_eq!(batch(romacs, &dir, &form, true), batch(&gnu, &dir, &form, false));
    // The frame the batch session runs in, and the Lisp it has preloaded:
    // apropos's regexp for that must match what the session reports.
    let form = "(list (eq terminal-frame (selected-frame)) (frame-live-p terminal-frame)
                      (file-directory-p lisp-directory)
                      (and (member \"subr\" preloaded-file-list) t) (and (member \"emacs-lisp/byte-run\" preloaded-file-list) t))";
    assert_eq!(batch(romacs, &dir, form, true), batch(&gnu, &dir, form, false));
    // GNU's is its dump's sysconf, which a process's stack limit can differ from.
    assert_eq!(batch(romacs, &dir, "(and (natnump command-line-max-length) (> command-line-max-length 1000))", true), "t");
    // Only nil or t, and known to byte-opt.el.
    let form = "(mapcar (lambda (s) (and (memq s byte-boolean-vars) t)) '(inhibit-compacting-font-caches car))";
    assert_eq!(batch(romacs, &dir, form, true), batch(&gnu, &dir, form, false));
    // apropos's preloaded walk, which reached both of these unbound.
    let form = "(progn (require 'apropos) (let (n) (apropos--map-preloaded-atoms (lambda (s) (when (eq s 'last) (setq n t)))) n))";
    assert_eq!(batch(romacs, &dir, form, true), batch(&gnu, &dir, form, false));
    let _ = std::fs::remove_dir_all(&dir);
}
