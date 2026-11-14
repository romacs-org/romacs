//! GNU's C variables, as a batch session reads them, against pinned GNU:
//! the same value, or the reason Romacs's differs.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

/// Printed alike in both sessions.
const SAME: &[&str] = &[
    "buffer-access-fontify-functions", "dynamic-library-alist", "echo-area-clear-hook", "emacs-copyright",
    "face-ignored-fonts", "font-log", "multiple-frames", "font-slant-table", "font-weight-table", "font-width-table", "fontset-alias-alist",
    "inhibit-compacting-font-caches", "inhibit-load-charset-map", "internal-make-interpreted-closure-function",
    "load-force-doc-strings", "load-source-file-function", "mouse-position-function", "mwheel-coalesce-scroll-events",
    "operating-system-release", "post-gc-hook", "redisplay--inhibit-bidi", "tool-bar-button-margin", "tool-bar-max-label-size",
];

fn batch(binary: &str, dir: &std::path::Path, form: &str) -> String {
    let mut command = Command::new(binary);
    command.args(["-Q", "--batch", "--eval", &format!("(prin1 {form})")]).current_dir(dir).env("HOME", dir).env("TMPDIR", dir);
    let output = command.output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn c_variables_read_as_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-defvar-values-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = format!("(mapcar (lambda (s) (list s (special-variable-p s) (symbol-value s))) '({}))", SAME.join(" "));
    assert_eq!(batch(romacs, &dir, &form), batch(&gnu, &dir, &form));
    // The frame the batch session runs in, and the Lisp it has preloaded:
    // apropos's regexp for that must match what the session reports.
    let form = "(list (eq terminal-frame (selected-frame)) (frame-live-p terminal-frame)
                      (file-directory-p lisp-directory)
                      (and (member \"subr\" preloaded-file-list) t) (and (member \"emacs-lisp/byte-run\" preloaded-file-list) t))";
    assert_eq!(batch(romacs, &dir, form), batch(&gnu, &dir, form));
    // GNU's shape in Romacs's name (VM2-TITLE): GNU's with the name put back.
    let form = "(mapcar (lambda (format) (prin1-to-string format)) (list frame-title-format icon-title-format))";
    assert_eq!(batch(romacs, &dir, form).replace("Romacs at", "GNU Emacs at"), batch(&gnu, &dir, form));
    // GNU's is its dump's sysconf, which a process's stack limit can differ from.
    assert_eq!(batch(romacs, &dir, "(and (natnump command-line-max-length) (> command-line-max-length 1000))"), "t");
    // Only nil or t, and known to byte-opt.el.
    let form = "(mapcar (lambda (s) (and (memq s byte-boolean-vars) t)) '(inhibit-compacting-font-caches car))";
    assert_eq!(batch(romacs, &dir, form), batch(&gnu, &dir, form));
    // apropos's preloaded walk, which reached both of these unbound.
    let form = "(progn (require 'apropos) (let (n) (apropos--map-preloaded-atoms (lambda (s) (when (eq s 'last) (setq n t)))) n))";
    assert_eq!(batch(romacs, &dir, form), batch(&gnu, &dir, form));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `find-word-boundary-function-table': subword-mode's stops, each way, at
/// the buffer's edges, and `forward-word-strictly' past them; a field's edge.
#[test]
fn subword_mode_moves_as_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-subword-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = "(with-temp-buffer
      (insert \"GtkWidget fooBarBaz EmacsFrame_x\") (subword-mode 1) (goto-char 1)
      (let (r)
        (dotimes (_ 9) (push (list (forward-word) (point)) r))
        (dotimes (_ 9) (push (list (forward-word -1) (point)) r))
        (goto-char 1) (push (list (forward-word 3) (point)) r)
        (goto-char (point-max)) (push (list (forward-word -2) (point)) r)
        (goto-char 1) (forward-word-strictly 1) (push (point) r)
        (goto-char 1) (subword-mode -1) (push (list (forward-word) (point)) r)
        (subword-mode 1) (goto-char 1) (put-text-property 5 9 'field 'f)
        (push (list (forward-word 2) (point)) r)
        (nreverse r)))";
    assert_eq!(batch(romacs, &dir, form), batch(&gnu, &dir, form));
    // The case commands scan the same way (casefiddle.c casify_word).
    let form = "(with-temp-buffer
      (insert \"fooBarBaz quxQuux\") (subword-mode 1)
      (let (r)
        (goto-char 1) (upcase-word 1) (push (list (buffer-string) (point)) r)
        (capitalize-word 1) (push (list (buffer-string) (point)) r)
        (goto-char (point-max)) (downcase-word -1) (push (list (buffer-string) (point)) r)
        (goto-char 1) (upcase-word 9) (push (list (buffer-string) (point)) r)
        (goto-char (point-max)) (downcase-word -9) (push (list (buffer-string) (point)) r)
        (nreverse r)))";
    assert_eq!(batch(romacs, &dir, form), batch(&gnu, &dir, form));
    let _ = std::fs::remove_dir_all(&dir);
}

/// `post-gc-hook' after a collection, against pinned GNU: once per
/// collection, after the finalizers, and a function that signals leaves the
/// hook. (A collection inside it differs on purpose: VM2-POST-GC,
/// romacs-vm's `garbage_collect' tests.)
#[test]
fn post_gc_hook_runs_as_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-post-gc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = "(progn
      (setq post-gc-log nil)
      (add-hook 'post-gc-hook (lambda () (push 'hook post-gc-log)))
      (eval '(progn (make-finalizer (lambda () (push 'finalizer post-gc-log))) nil) t)
      (garbage-collect)
      (garbage-collect)
      (add-hook 'post-gc-hook (lambda () (error \"boom\")))
      (garbage-collect)
      (list (reverse post-gc-log) (length post-gc-hook)))";
    assert_eq!(batch(romacs, &dir, form), batch(&gnu, &dir, form));
    let _ = std::fs::remove_dir_all(&dir);
}

/// With no pointer, as batch has none: frame.c's answers and checks,
/// through `mouse-position-function'.
#[test]
fn mouse_position_with_no_pointer_is_pinned_gnu() {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let romacs = installed::romacs();
    let dir = std::env::temp_dir().join(format!("romacs-mouse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let form = "(let ((seen (lambda (p) (list 'seen (eq (car p) (selected-frame)) (cdr p)))))
      (list (eq (car (mouse-position)) (selected-frame)) (cdr (mouse-position)) (cdr (mouse-pixel-position))
            (mouse-position-in-root-frame)
            (condition-case e (set-mouse-position (selected-frame) 1 2) (error e))
            (condition-case e (set-mouse-pixel-position (selected-frame) 1 2) (error e))
            (condition-case e (set-mouse-position nil 1 2) (error e))
            (condition-case e (set-mouse-pixel-position (selected-frame) 'a 2) (error e))
            (condition-case e (set-mouse-position (selected-frame) 2147483648 2) (error e))
            (let ((mouse-position-function seen)) (list (mouse-position) (mouse-pixel-position)))))";
    assert_eq!(batch(romacs, &dir, form), batch(&gnu, &dir, form));
    let _ = std::fs::remove_dir_all(&dir);
}
