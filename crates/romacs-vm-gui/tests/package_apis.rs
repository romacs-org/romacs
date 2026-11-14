//! Functions and variables third-party packages reach (compat, ht, ivy,
//! consult), as a batch session answers them, against pinned GNU.
#[path = "support/installed.rs"]
mod installed;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A private HOME, XDG directories and TMPDIR for one test.
fn private_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("romacs-package-apis-{name}-{}", std::process::id()));
    for sub in ["config", "cache", "data", "state", "tmp"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    dir
}

/// FORM's printed value in BINARY's `-Q --batch` session.
fn batch(binary: &str, dir: &Path, form: &str) -> String {
    let output = Command::new(binary)
        .args(["-Q", "--batch", "--eval", &format!("(prin1 {form})")])
        .current_dir(dir).env("HOME", dir).env("TMPDIR", dir.join("tmp"))
        .env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state"))
        .env_remove("XDG_RUNTIME_DIR").env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY")
        .output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Each of FORMS printed alike by Romacs and pinned GNU.
fn same_as_gnu(name: &str, forms: &[&str]) {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = private_dir(name);
    for form in forms {
        assert_eq!(batch(installed::romacs(), &dir, form), batch(&gnu, &dir, form), "{form}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// casefiddle.c defines `region-extract-function` in C, so it is special
/// before simple.el sets it, and a lexical `let` of it reaches
/// `region-bounds` (compat's region tests).
#[test]
fn region_extract_function_is_special() {
    same_as_gnu("region-extract", &[
        "(list (special-variable-p 'region-extract-function) (let ((region-extract-function #'ignore)) (region-bounds)))",
        "(with-temp-buffer (insert \"abcd\") (let ((region-extract-function (lambda (_) '((1 . 3))))) (list (region-bounds) (region-noncontiguous-p))))",
    ]);
}

/// xdisp.c `get-display-property`: a list or vector of specifications, one
/// specification, one with several values, PROPERTIES given, a string.
#[test]
fn get_display_property_reads_a_specification() {
    same_as_gnu("display-property", &[
        "(with-temp-buffer (insert (propertize \"x\" 'display '(height 2.0))) (list (get-display-property 1 'height) (get-display-property 1 'raise)))",
        "(with-temp-buffer (insert (propertize \"x\" 'display '((raise 0.5) (height 2.0)))) (get-display-property 1 'height))",
        "(with-temp-buffer (insert (propertize \"x\" 'display [(raise 0.5) (space :width 3)])) (list (get-display-property 1 'raise) (get-display-property 1 'space)))",
        "(list (get-display-property 0 'height nil '((height 1.5))) (get-display-property 0 'image (propertize \"y\" 'display '(image :type png))))",
        "(with-temp-buffer (insert \"z\") (list (get-display-property 1 'height) (condition-case e (get-display-property 1 'height nil 3) (error e))))",
    ]);
}

/// startup.el `command-line` gives *scratch* `initial-major-mode` before the
/// command-line arguments run, in batch too.
#[test]
fn scratch_takes_initial_major_mode() {
    same_as_gnu("scratch", &[
        "(with-current-buffer (get-scratch-buffer-create) (list initial-major-mode major-mode))",
    ]);
}

/// `maphash` named at run time, as `apply` and a variable call it (ht.el).
#[test]
fn maphash_through_apply() {
    same_as_gnu("maphash", &[
        "(let ((h (make-hash-table)) (f #'maphash) acc) (puthash 1 2 h) (puthash 3 4 h)
           (list (apply #'maphash (list (lambda (k v) (push (cons k v) acc)) h))
                 (funcall f (lambda (k _) (push k acc)) h)
                 acc
                 (condition-case e (apply #'maphash #'ignore '(1)) (error e))))",
    ]);
}

/// editfns.c's labeled restrictions (`with-restriction` and
/// `without-restriction` with a label): narrowing and widening within the
/// label, nesting, lifting by label, an error inside, insertion at the
/// bounds, `erase-buffer`, `save-restriction`, an indirect buffer.
#[test]
fn labeled_restrictions() {
    same_as_gnu("labeled-restrictions", &[
        "(with-temp-buffer (insert \"abcdef\") (list (with-restriction 2 4 :label 'foo (list (buffer-string) (progn (narrow-to-region 1 7) (list (point-min) (point-max))) (progn (narrow-to-region 3 3) (list (point-min) (point-max))) (progn (widen) (list (point-min) (point-max))))) (list (point-min) (point-max))))",
        "(with-temp-buffer (insert \"abcdef\") (narrow-to-region 2 6) (with-restriction 3 5 :label 'foo (list (without-restriction :label 'foo (list (point-min) (point-max))) (without-restriction :label 'bar (list (point-min) (point-max))) (without-restriction (list (point-min) (point-max))) (list (point-min) (point-max)))))",
        "(with-temp-buffer (insert \"abcdefgh\") (with-restriction 2 8 :label 'outer (with-restriction 3 7 :label 'inner (list (progn (widen) (list (point-min) (point-max))) (without-restriction :label 'inner (list (point-min) (point-max) (progn (widen) (list (point-min) (point-max))))) (without-restriction :label 'outer (list (point-min) (point-max)))))))",
        "(with-temp-buffer (insert \"abcdef\") (list (condition-case e (with-restriction 2 4 :label 'foo (error \"boom\")) (error (cadr e))) (list (point-min) (point-max)) (progn (widen) (list (point-min) (point-max)))))",
        "(with-temp-buffer (insert \"abcdef\") (with-restriction 2 4 :label 'foo (list (condition-case e (narrow-to-region 1 20) (error (car e))) (progn (goto-char (point-max)) (insert \"XY\") (list (point-min) (point-max) (buffer-string))) (progn (widen) (list (point-min) (point-max))))))",
        "(with-temp-buffer (insert \"abcdef\") (with-restriction 2 4 :label 'foo (erase-buffer) (insert \"xyz\") (list (point-min) (point-max) (progn (widen) (list (point-min) (point-max))))))",
        "(with-temp-buffer (insert \"abcdef\") (save-restriction (internal--labeled-narrow-to-region 2 4 'foo) (narrow-to-region 1 7)) (list (point-min) (point-max) (progn (narrow-to-region 1 7) (list (point-min) (point-max)))))",
        "(with-temp-buffer (insert \"abcdef\") (with-restriction 2 5 :label 'foo (let ((base (current-buffer))) (with-current-buffer (make-indirect-buffer base \"ind\") (prog1 (list (point-min) (point-max) (progn (narrow-to-region 1 7) (list (point-min) (point-max)))) (kill-buffer))))))",
    ]);
}

/// eval.c `funcall-with-delayed-message` (`with-delayed-message`): the
/// function's value, its errors and the argument checks (consult, markdown-mode).
#[test]
fn funcall_with_delayed_message() {
    same_as_gnu("delayed-message", &[
        "(list (with-delayed-message (1 \"msg\") 42) (funcall-with-delayed-message 0.5 \"m\" (lambda () 'x)) (condition-case e (funcall-with-delayed-message 'a \"m\" #'ignore) (error e)) (condition-case e (funcall-with-delayed-message 1 'm #'ignore) (error e)) (condition-case e (with-delayed-message (1 \"m\") (error \"inner\")) (error e)))",
    ]);
}

/// window.c `resize-mini-window-internal`, as `window--resize-mini-window`
/// prepares and calls it (ivy).
#[test]
fn resize_mini_window_internal() {
    same_as_gnu("mini-window", &[
        "(list (fboundp 'resize-mini-window-internal) (condition-case e (resize-mini-window-internal (selected-window)) (error e)))",
        "(let ((w (minibuffer-window))) (window--resize-mini-window w 2) (prog1 (list (window-pixel-height w) (window-pixel-height (frame-root-window)) (window-total-height w)) (window--resize-mini-window w -2)))",
    ]);
}

/// minibuf.c `minibufferp`: every minibuffer made is one, shown or not, and
/// only an active one is live; evil's `evil-initialize` asks it of
/// *Minibuf-1* as it takes its mode.
#[test]
fn minibufferp_knows_every_minibuffer() {
    same_as_gnu("minibufferp", &[
        "(progn (defun my-read () (interactive) (read-from-minibuffer \"x: \")) (global-set-key (kbd \"C-c r\") #'my-read) (execute-kbd-macro (vconcat (kbd \"C-c r\") \"abc\" [return])) (let ((b (get-buffer \" *Minibuf-1*\"))) (list (minibufferp b) (minibufferp b t) (with-current-buffer b (minibufferp)) (minibufferp (get-buffer \" *Minibuf-0*\")) (minibufferp (get-buffer-create \" *Minibuf-9*\")))))",
    ]);
}

/// buffer.c `overlays-in` with positions outside the buffer, empty overlays
/// at its edges (yasnippet's `yas-active-snippets` asks from 0).
#[test]
fn overlays_in_outside_the_buffer() {
    same_as_gnu("overlays-in", &[
        "(with-temp-buffer (insert \"abcdefgh\") (let ((a (make-overlay 2 4)) (z (make-overlay 9 9)) (e (make-overlay 1 1))) (overlay-put a 'n 'a) (overlay-put z 'n 'z) (overlay-put e 'n 'e) (narrow-to-region 3 5) (mapcar (lambda (r) (sort (mapcar (lambda (o) (overlay-get o 'n)) (apply #'overlays-in r)) #'string<)) '((0 10) (-5 3) (1 100) (0 0) (100 200) (9 9) (0 1) (3 20)))))",
        "(with-temp-buffer (let ((e (make-overlay 1 1))) (list (overlays-in 0 1) (overlays-in -3 5) (overlays-in 2 9))))",
    ]);
}
