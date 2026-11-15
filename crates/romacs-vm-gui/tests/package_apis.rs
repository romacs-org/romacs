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
/// bounds, `erase-buffer`, `save-restriction` (across `buffer-swap-text`
/// too), an indirect buffer. Point after a labeled widening is
/// VM2-LABELED-WIDEN-POINT's.
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
        "(let ((a (generate-new-buffer \"a\")) (b (generate-new-buffer \"b\"))) (with-current-buffer b (insert \"123456789\")) (with-current-buffer a (insert \"abcdef\") (with-restriction 2 4 :label 'foo (save-restriction (buffer-swap-text b)))) (prog1 (list (with-current-buffer a (widen) (list (point-min) (point-max))) (with-current-buffer b (widen) (list (point-min) (point-max)))) (kill-buffer a) (kill-buffer b)))",
        "(let ((a (generate-new-buffer \"a\")) (b (generate-new-buffer \"b\"))) (with-current-buffer b (insert \"123456789\")) (with-current-buffer a (insert \"abcdef\") (narrow-to-region 2 6) (save-restriction (internal--labeled-narrow-to-region 3 4 'foo) (buffer-swap-text b))) (prog1 (list (with-current-buffer a (widen) (list (point-min) (point-max))) (with-current-buffer b (widen) (list (point-min) (point-max)))) (kill-buffer a) (kill-buffer b)))",
        "(with-temp-buffer (insert \"abcdef\") (with-restriction 2 5 :label 'outermost-restriction (list (progn (widen) (list (point-min) (point-max))) (progn (widen) (list (point-min) (point-max))))))",
    ]);
}
