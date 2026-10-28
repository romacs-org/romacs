//! What GNU's minibuffer completion asks of the runtime, against pinned GNU
//! 32.0.50 in batch: dired.c's `file-name-completion` and
//! `file-name-all-completions`, minibuf.c's `internal-complete-buffer` and
//! `read-buffer`, and `read-file-name` with partial completion, the last two
//! reading their input from a keyboard macro. The same forms, evaluated in
//! order in one GNU process and in one VM on GNU's loadup chain, must print
//! the same. icomplete stays inert in a keyboard macro
//! (`icomplete-simple-completing-p`), so its display is proved through the
//! GUI's keyboard (crates/romacs-vm-gui/tests/gnu_completion.rs).
use std::path::Path;
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

/// Each case runs with DIR, the fixture directory, a private global map
/// over GNU's, and *scratch* emptied after.
const PRELUDE: &str = "(defmacro kt-case (&rest body)
  `(let ((kt-map (make-sparse-keymap)))
     (set-keymap-parent kt-map global-map)
     (use-global-map kt-map)
     (unwind-protect (progn ,@body)
       (use-global-map global-map)
       (with-current-buffer \"*scratch*\" (erase-buffer)))))";

const CASES: &[&str] = &[
    // file-name-completion: common prefix, sole exact match, no match.
    r#"(list (file-name-completion "al" DIR) (file-name-completion "alpha.txt" DIR)
        (file-name-completion "zz" DIR) (file-name-completion "b" DIR))"#,
    // completion-ignored-extensions: alpha.o and CVS/ drop out while
    // anything else matches, and "." and ".." always do.
    r#"(list (file-name-completion "alpha" DIR) (file-name-completion "alpha.o" DIR)
        (file-name-completion "C" DIR) (file-name-completion "" DIR) (file-name-completion "." DIR))"#,
    // Ignoring case, and completion-regexp-list.
    r#"(let ((completion-ignore-case t))
  (list (file-name-completion "b" DIR) (file-name-completion "NOTES" DIR) (file-name-completion "notes.txt" DIR)))"#,
    r#"(let ((completion-regexp-list '("\\.txt\\'")))
  (list (file-name-completion "" DIR) (sort (file-name-all-completions "" DIR) #'string<)))"#,
    // completion-regexp-list over a list, the obarray and a directory, each
    // regexp compiled once per pass: folding, several regexps in order, an
    // invalid one no candidate reaches, and the errors one does reach.
    r#"(let ((names '("Alpha" "alpine" "beta" "ALPS" "gamma")))
  (list (let ((completion-regexp-list '("p")) (completion-ignore-case nil)) (all-completions "" names))
        (let ((completion-regexp-list '("P")) (completion-ignore-case t)) (all-completions "" names))
        (let ((completion-regexp-list '("l" "s\\'")) (completion-ignore-case t)) (all-completions "a" names))
        (let ((completion-regexp-list '("zz" "[")) (completion-ignore-case nil)) (all-completions "" names))
        (let ((completion-regexp-list '("a" "[")) (completion-ignore-case nil)) (condition-case e (all-completions "" names) (error e)))
        (let ((completion-regexp-list '("a" 7))) (condition-case e (all-completions "" names) (error e)))
        (let ((completion-regexp-list '("l")) (completion-ignore-case t)) (try-completion "AL" names))
        (let ((completion-regexp-list '("\\`kt-rx-[ab]"))) (intern "kt-rx-alpha") (intern "kt-rx-beta") (intern "kt-rx-gamma")
          (sort (all-completions "kt-" obarray) #'string<))
        (let ((completion-regexp-list '("\\.txt\\'" "^a"))) (sort (file-name-all-completions "" DIR) #'string<))
        (let ((completion-regexp-list '("E")) (completion-ignore-case t)) (sort (file-name-all-completions "" DIR) #'string<))
        (let ((completion-regexp-list '("zz" "["))) (file-name-all-completions "" DIR))))"#,
    // PREDICATE sees relative names with default-directory bound to DIR.
    r#"(let (seen)
  (list (file-name-completion "" DIR (lambda (name) (push (list name (equal default-directory DIR)) seen)
                                         (file-directory-p name)))
        (sort seen (lambda (a b) (string< (car a) (car b))))))"#,
    r#"(list (sort (file-name-all-completions "" DIR) #'string<) (sort (file-name-all-completions "AL" DIR) #'string<)
        (let ((completion-ignore-case t)) (sort (file-name-all-completions "AL" DIR) #'string<)))"#,
    r#"(list (condition-case e (file-name-completion 'al DIR) (error e))
        (condition-case e (file-name-completion "al" (concat DIR "missing/")) (error (car e))))"#,
    // internal-complete-buffer over the buffer list.
    r#"(let ((buffers (list (get-buffer-create "alpha-buf") (get-buffer-create "alpine-buf") (get-buffer-create " *hidden-kt*"))))
  (unwind-protect
      (list (internal-complete-buffer "alp" nil nil) (sort (internal-complete-buffer "alp" nil t) #'string<)
            (internal-complete-buffer "alpha-buf" nil 'lambda) (internal-complete-buffer "alpha" nil 'lambda)
            (internal-complete-buffer "" nil 'metadata) (internal-complete-buffer "" nil '(boundaries . ""))
            (member " *hidden-kt*" (internal-complete-buffer "" nil t)) (internal-complete-buffer " *hidden-k" nil t)
            (internal-complete-buffer "alp" (lambda (b) (equal (car b) "alpine-buf")) nil))
    (mapc #'kill-buffer buffers)))"#,
    // read-buffer: its prompt with a default, TAB completion, the default,
    // ignoring case, and `read-buffer-function'.
    r#"(let ((buffers (list (get-buffer-create "alpha-buf") (get-buffer-create "alpine-buf"))) got prompts)
  (unwind-protect
      (progn
        (global-set-key "m" (lambda () (interactive) (push (read-buffer "Buffer: " (car buffers)) got)))
        (global-set-key "n" (lambda () (interactive) (push (read-buffer "Buffer: " nil t) got)))
        (let ((minibuffer-setup-hook (list (lambda () (push (minibuffer-prompt) prompts)))))
          (execute-kbd-macro (kbd "m RET m a l p i TAB RET n a l p h TAB RET"))
          (let ((read-buffer-completion-ignore-case t)) (execute-kbd-macro (kbd "n A L P I TAB RET"))))
        (list (nreverse got) (nreverse prompts)
              (let ((read-buffer-function (lambda (&rest args) (cons 'reader args))))
                (list (read-buffer "P: " "d" t) (read-buffer "P: " nil nil #'ignore)))))
    (mapc #'kill-buffer buffers)))"#,
    // read_minibuf's state: the history variable and position, a reused
    // minibuffer without its last overlays or local variables, the caller's
    // directory, file-name reading as `lambda', and the exit hook.
    r#"(let (log kt-history)
  (global-set-key "m" (lambda () (interactive) (read-string "P: " nil 'kt-history)))
  (global-set-key "f" (lambda () (interactive) (let ((minibuffer-completing-file-name t)) (read-string "F: "))))
  (let ((minibuffer-setup-hook
         (list (lambda ()
                 (push (list minibuffer-history-variable minibuffer-history-position
                             (length (overlays-in (point-min) (point-max))) major-mode
                             (local-variable-p 'kt-local) (equal default-directory DIR)
                             minibuffer-completing-file-name)
                       log)
                 (set (make-local-variable 'kt-local) t)
                 (overlay-put (make-overlay (point-max) (point-max)) 'after-string "x"))))
        (minibuffer-exit-hook (list (lambda () (push (list 'exit (minibufferp) minibuffer-history-variable) log))))
        (default-directory DIR))
    (execute-kbd-macro (kbd "m a RET m b RET f c RET")))
  (list (nreverse log) minibuffer-history-variable minibuffer-history-position minibuffer-completing-file-name
        kt-history))"#,
    // read-file-name with partial completion over the directory.
    r#"(let ((default-directory DIR) (completion-styles '(basic partial-completion)) got)
  (global-set-key "m" (lambda () (interactive) (push (read-file-name "File: ") got)))
  (execute-kbd-macro (kbd "m b - d TAB RET m a l p h a . t TAB RET"))
  (mapcar (lambda (f) (file-relative-name f DIR)) (nreverse got)))"#,
];

/// The fixture directory: names chosen for every rule the cases reach.
fn fixture(root: &Path) -> String {
    let dir = root.join(format!("minibuffer-completion-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("beta-dir")).unwrap();
    std::fs::create_dir_all(dir.join("CVS")).unwrap();
    for name in ["alpha.txt", "alpha.o", "alpine.el", "Beta", "notes.TXT", ".hidden"] {
        std::fs::write(dir.join(name), name).unwrap();
    }
    format!("{}/", dir.to_str().unwrap())
}

fn program(dir: &str) -> String {
    let cases = CASES.iter().map(|case| format!("(kt-case (princ (prin1-to-string {case}))) (princ \"\\n\\x1e\\n\")"))
        .collect::<String>();
    format!("(progn {PRELUDE} (defvar DIR {dir:?}) {cases})")
}

fn gnu_results(root: &Path, dir: &str, home: &Path) -> Vec<String> {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).env("HOME", home).current_dir(root)
        .args(["-Q", "--batch", "--eval", &program(dir)]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap().split("\n\x1e\n").take(CASES.len()).map(str::to_owned).collect()
}

#[test]
fn minibuffer_completion_matches_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let scratch = std::env::temp_dir().join(format!("minibuffer-completion-{}", std::process::id()));
    let home = scratch.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let dir = fixture(&scratch);
    let expected = gnu_results(&root, &dir, &home);
    assert_eq!(expected.len(), CASES.len());
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    vm.eval_source(&format!("(progn {PRELUDE} (defvar DIR {dir:?}))")).unwrap();
    let mut failures = vec![];
    for (case, gnu) in CASES.iter().zip(&expected) {
        let printed = match vm.eval_source(&format!("(kt-case (prin1-to-string {case}))")) {
            Ok(value) => value.display().unwrap(),
            Err(e) => format!("VM-ERROR {e}"),
        };
        let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
        if &actual != gnu { failures.push(format!("{case}\n  GNU    {gnu}\n  Romacs {actual}")); }
    }
    std::fs::remove_dir_all(&scratch).ok();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
