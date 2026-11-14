//! The primitives whose arity the GNU DEFUN audit (gnu_arity_audit.rs)
//! found short, against pinned GNU on the same script: `overlays-at' SORTED
//! (buffer.c sort_overlays/compare_overlays: priorities, cons priorities,
//! ties, nested and equal ranges, equal starts in tree order (also after
//! `move-overlay' and in a clone), the comparator's non-transitive cases, a
//! window, markers, empty overlays, the narrowing; and the running choice
//! of `get-char-property' and `get-pos-property'), `plist-get' PREDICATE, `local-variable-if-set-p' BUFFER and
//! `make-interpreted-closure' with its optional DOCSTRING and IFORM.
#[path = "support/installed.rs"]
mod installed;
use std::process::Command;

const OVERLAYS: &str = r#";;; -*- lexical-binding: t -*-
(defun ov (beg end name &rest props)
  (let ((o (make-overlay beg end)))
    (overlay-put o 'name name)
    (while props (overlay-put o (pop props) (pop props)))
    o))
(defun names (list) (mapcar (lambda (o) (overlay-get o 'name)) list))
(defun show (label value) (prin1 (list label value)) (terpri))
(defmacro case-in (label &rest body)
  `(with-temp-buffer
     (insert "0123456789abcdefghijklmnopqrstuvwxyz")
     (show ,label (condition-case e (progn ,@body) (error (list 'error e))))))
(case-in "priorities" (ov 3 10 'a 'priority 5) (ov 1 12 'b 'priority 10) (ov 5 8 'c)
         (list (names (overlays-at 6 t)) (sort (names (overlays-at 6)) #'string<)))
(case-in "nested-tie" (ov 1 15 'outer 'priority 2) (ov 4 9 'inner 'priority 2) (ov 4 12 'mid 'priority 2)
         (names (overlays-at 5 t)))
(case-in "equal-ranges" (ov 2 6 'first) (ov 2 6 'second) (ov 2 6 'third)
         (names (overlays-at 3 t)))
(case-in "secondary" (ov 2 6 'p13 'priority '(1 . 3)) (ov 2 6 'p15 'priority '(1 . 5))
         (ov 2 6 'p20 'priority '(2 . 0)) (ov 2 6 'p1 'priority 1) (ov 2 6 'bad 'priority '(x . y))
         (names (overlays-at 3 t)))
(case-in "overlap-secondary" (ov 1 6 's1 'priority '(0 . 9)) (ov 3 9 's2 'priority '(0 . 1))
         (ov 2 7 's3 'priority '(0 . 5)) (ov 4 5 's4 'priority '(0 . 7))
         (names (overlays-at 4 t)))
(case-in "overlap-secondary-reversed" (ov 1 6 's1 'priority '(0 . 1)) (ov 3 9 's2 'priority '(0 . 9))
         (names (overlays-at 4 t)))
(case-in "category" (put 'arity-cat 'priority 50) (ov 1 9 'cat 'category 'arity-cat) (ov 1 9 'ten 'priority 10)
         (names (overlays-at 4 t)))
(case-in "marker" (ov 3 10 'a 'priority 1) (ov 5 7 'b) (names (overlays-at (copy-marker 6) t)))
(case-in "empty-excluded" (ov 6 6 'empty) (ov 5 7 'real) (names (overlays-at 6 t)))
(case-in "none" (names (overlays-at 6 t)))
(case-in "no-overlay-here" (ov 1 3 'a) (names (overlays-at 6 t)))
(case-in "outside-narrowing" (ov 5 9 'far 'priority 3) (ov 6 8 'near) (narrow-to-region 1 3)
         (names (overlays-at 6 t)))
(case-in "end-excluded" (ov 3 6 'a) (list (names (overlays-at 5 t)) (names (overlays-at 6 t))))
(case-in "bad-pos" (overlays-at "x" t))
(case-in "sorted-symbol" (ov 1 9 'a 'priority 1) (ov 1 9 'b 'priority 2) (names (overlays-at 4 'yes)))
(let* ((w1 (selected-window)) (w2 (split-window)))
  (case-in "window"
           (set-window-buffer w1 (current-buffer))
           (ov 1 9 'any 'priority 1) (ov 1 9 'w1 'window w1 'priority 2) (ov 1 9 'w2 'window w2 'priority 3)
           (ov 1 9 'sym 'window 'foo 'priority 4)
           (list (names (overlays-at 4 w1)) (names (overlays-at 4 w2)) (names (overlays-at 4 t))))
  (delete-window w2))
(case-in "out-of-range" (ov 1 5 'a) (list (overlays-at 1000 t) (overlays-at 0) (overlays-at -5 t)))
(case-in "float-pos" (overlays-at 1.5))
(case-in "non-transitive" (ov 1 6 'A 'priority '(0 . 1)) (ov 2 6 'B 'priority '(0 . 0)) (ov 3 7 'C 'priority '(0 . 0))
         (list (names (overlays-at 4 t)) (names (overlays-at 5 t))))
(case-in "non-transitive-created-late" (ov 3 7 'C 'priority '(0 . 0)) (ov 2 6 'B 'priority '(0 . 0)) (ov 1 6 'A 'priority '(0 . 1))
         (names (overlays-at 4 t)))
(case-in "property-among-having" (ov 2 6 'B 'priority '(0 . 0)) (ov 3 7 'C 'priority '(0 . 0) 'p 'c) (ov 1 6 'A 'priority '(0 . 1) 'p 'a)
         (list (get-char-property 4 'p) (get-pos-property 4 'p) (overlay-get (cdr (get-char-property-and-overlay 4 'p)) 'name)))
(case-in "equal-starts" (ov 1 6 'A 'priority '(0 . 1)) (ov 1 5 'B 'priority '(0 . 0)) (ov 2 7 'C 'priority '(0 . 0))
         (names (overlays-at 3 t)))
(case-in "equal-starts-moved" (let ((a (ov 1 6 'A 'priority '(0 . 1)))) (ov 1 5 'B 'priority '(0 . 0)) (ov 2 7 'C 'priority '(0 . 0))
           (move-overlay a 2 6) (move-overlay a 1 6) (list (names (overlays-at 3 t)) (progn (move-overlay a 1 7) (names (overlays-at 3 t))))))
(case-in "equal-starts-clone" (ov 1 6 'A 'priority '(0 . 1)) (ov 1 5 'B 'priority '(0 . 0)) (ov 2 7 'C 'priority '(0 . 0))
         (let ((clone (make-indirect-buffer (current-buffer) "arity-clone" t)))
           (prog1 (with-current-buffer clone (names (overlays-at 3 t))) (kill-buffer clone))))
"#;

const OTHERS: &str = r#";;; -*- lexical-binding: t -*-
(defun show (label thunk)
  (prin1 (list label (condition-case e (funcall thunk) (error (list 'error e))))) (terpri))
(show "plist-get eq" (lambda () (list (plist-get '(a 1 b 2) 'b) (plist-get '(a 1 b 2) 'b nil) (plist-get '(a 1 b) 'b))))
(show "plist-get predicate" (lambda () (list (plist-get '("a" 1 "b" 2) "b" #'string=) (plist-get '("a" 1 "b" 2) "c" #'equal)
                                             (plist-get '(1 x 2.0 y) 2 #'=) (plist-get '(a 1 . b) 'z #'eq))))
(show "plist-get predicate args" (lambda () (let (seen) (plist-get '(a 1 b 2) 'q (lambda (k p) (push (list k p) seen) nil)) (nreverse seen))))
(show "plist-get circular" (lambda () (let ((l (list 'a 1 'b 2))) (setcdr (last l) l) (list (plist-get l 'b) (plist-get l 'z) (plist-get l 'z #'equal)))))
(show "plist-get dotted" (lambda () (list (plist-get '(a 1 . 2) 'z) (plist-get 5 'a) (plist-get 5 'a #'eq) (plist-get '(a) 'a #'eq))))
(show "plist-get signal" (lambda () (plist-get '(a 1) 'a (lambda (_k _p) (error "boom")))))
(show "plist-get non-function" (lambda () (plist-get '(a 1) 'a 'no-such-function)))
(defvar arity-auto nil) (make-variable-buffer-local 'arity-auto)
(defvar arity-plain nil)
(defvar arity-explicit nil)
(show "local-variable-if-set-p" (lambda ()
  (let ((other (get-buffer-create "other")))
    (with-current-buffer other (setq-local arity-explicit 1))
    (list (local-variable-if-set-p 'arity-auto) (local-variable-if-set-p 'arity-auto other)
          (local-variable-if-set-p 'arity-plain) (local-variable-if-set-p 'arity-plain other)
          (local-variable-if-set-p 'arity-explicit) (local-variable-if-set-p 'arity-explicit other)
          (local-variable-if-set-p 'fill-column other) (local-variable-if-set-p 'load-path other)
          (local-variable-if-set-p 'arity-plain 'not-a-buffer) (local-variable-if-set-p nil other)))))
(show "local-variable-if-set-p bad buffer" (lambda () (local-variable-if-set-p 'arity-explicit 'not-a-buffer)))
(show "local-variable-if-set-p bad symbol" (lambda () (local-variable-if-set-p "x")))
(show "make-interpreted-closure" (lambda ()
  (let ((three (make-interpreted-closure '(x) '((* x 2)) '(t)))
        (four (make-interpreted-closure '(x) '((+ x y)) '((y . 10) t) "Add Y."))
        (five (make-interpreted-closure '(x) '((list x)) '(t) nil '(interactive "p"))))
    (list (funcall three 4) (funcall four 1) (documentation four) (funcall five 'z)
          (interactive-form five) (func-arity three)))))
;; Only the condition: legacy primitives' arity data is a known divergence.
(show "make-interpreted-closure 2 args" (lambda () (condition-case e (make-interpreted-closure nil '(1)) (error (car e)))))
"#;

fn run(binary: &str, dir: &std::path::Path) -> (Option<i32>, String, String) {
    let output = Command::new(binary).args(["-Q", "--batch", "-l", "probe.el"]).current_dir(dir)
        .env("HOME", dir).env("XDG_CONFIG_HOME", dir.join("config")).env("XDG_CACHE_HOME", dir.join("cache"))
        .env("XDG_DATA_HOME", dir.join("data")).env("XDG_STATE_HOME", dir.join("state"))
        .output().unwrap();
    let text = |bytes: Vec<u8>| String::from_utf8_lossy(&bytes).into_owned();
    (output.status.code(), text(output.stdout), text(output.stderr))
}

fn matches_gnu(name: &str, script: &str) {
    let gnu = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let dir = std::env::temp_dir().join(format!("romacs-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("probe.el"), script).unwrap();
    let expected = run(&gnu, &dir);
    let actual = run(installed::romacs(), &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(expected.0, Some(0), "GNU: {}", expected.2);
    assert_eq!((actual.0, &actual.1), (expected.0, &expected.1), "Romacs stderr: {}", actual.2);
}

#[test]
fn overlays_at_sorted_matches_pinned_gnu() {
    matches_gnu("overlays-at", OVERLAYS);
}

#[test]
fn optional_arguments_found_by_the_audit_match_pinned_gnu() {
    matches_gnu("arity-fixes", OTHERS);
}
