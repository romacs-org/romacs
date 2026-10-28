//! insert-file-contents runs GNU's change bracket for a read that is not
//! a visit (fileio.c:4889 prepare_to_modify_buffer, 5132
//! signal_after_change): the before-change hooks at point, the
//! after-change hooks over the inserted text with point before it, the
//! text's insertion hooks and first-change-hook. Compared against pinned GNU.
use romacs_vm::Vm;
use std::path::{Path, PathBuf};
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;

const SETUP: &str = r##"(set-buffer (get-buffer-create "*ifc-hooks*")) (kill-all-local-variables) (erase-buffer) (set-buffer-modified-p nil) (setq log nil) (defun ifc-log (&rest args) (setq log (cons (append args (list (point) (buffer-string))) log)))"##;

const CASES: &str = r##"
(let ((before-change-functions (list (lambda (b e) (ifc-log 'b b e)))) (after-change-functions (list (lambda (b e l) (ifc-log 'a b e l))))) (list (insert-file-contents FILE) (point) (buffer-string) (reverse log)))
(progn (insert "ab") (goto-char 2) (let ((before-change-functions (list (lambda (b e) (ifc-log 'b b e)))) (after-change-functions (list (lambda (b e l) (ifc-log 'a b e l))))) (list (insert-file-contents FILE) (point) (buffer-string) (reverse log))))
(let ((first-change-hook (list (lambda () (ifc-log 'first))))) (list (insert-file-contents FILE) (buffer-string) (reverse log)))
(let ((after-change-functions (list (lambda (b e l) (ifc-log 'a b e l))))) (list (insert-file-contents FILE nil 1 2) (buffer-string) (reverse log)))
(let ((after-change-functions (list (lambda (b e l) (ifc-log 'a b e l))))) (list (insert-file-contents EMPTY) (buffer-string) (reverse log)))
(let ((inhibit-modification-hooks t) (after-change-functions (list (lambda (b e l) (ifc-log 'a b e l))))) (list (insert-file-contents FILE) (buffer-string) (reverse log)))
(let ((after-change-functions (list (lambda (b e l) (ifc-log 'a b e l))))) (list (car (last (insert-file-contents FILE t))) (buffer-string) (buffer-modified-p) (reverse log)))
(let ((before-change-functions (list (lambda (b e) (goto-char (point-max)) (insert "z")))) (after-change-functions (list (lambda (b e l) (ifc-log 'a b e l))))) (insert "pq") (goto-char 1) (setq log nil) (list (insert-file-contents FILE) (buffer-string) (reverse log)))
(progn (setq after-change-functions (list (lambda (&rest _) (error "boom")))) (list (condition-case e (insert-file-contents FILE) (error e)) (buffer-string) after-change-functions))
(progn (insert "ab") (defun ifc-front (b e) (ifc-log 'front b e)) (defun ifc-behind (b e) (ifc-log 'behind b e)) (put-text-property 2 3 'insert-in-front-hooks '(ifc-front)) (put-text-property 1 2 'insert-behind-hooks '(ifc-behind)) (goto-char 2) (list (insert-file-contents FILE) (buffer-string) (reverse log)))
(progn (insert "ab") (defun ifc-mod (b e) (ifc-log 'mod b e)) (put-text-property 1 3 'modification-hooks '(ifc-mod)) (goto-char 2) (list (insert-file-contents FILE) (buffer-string) (reverse log)))
"##;

fn oracle(expression: &str) -> String {
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let source = format!("(progn {SETUP} (prin1 (prin1-to-string {expression})))");
    let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &source])
        .env("LANG", "C.UTF-8").output().unwrap();
    assert!(output.status.success(), "{expression}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn romacs(vm: &mut Vm, expression: &str) -> String {
    let source = format!("(progn (defvar log nil) {SETUP} (prin1-to-string {expression}))");
    match vm.eval_source(&source).and_then(|v| v.display()) {
        Ok(text) => text,
        Err(error) => format!("VM error: {error}"),
    }
}

fn lisp(path: &Path) -> String {
    serde_json::to_string(path.to_str().unwrap()).unwrap()
}

#[test]
fn insert_file_contents_runs_change_hooks_as_gnu() {
    let dir: PathBuf = std::env::temp_dir().join(format!("romacs-ifc-hooks-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (file, empty) = (dir.join("file"), dir.join("empty"));
    std::fs::write(&file, b"XY").unwrap();
    std::fs::write(&empty, b"").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let failures: Vec<String> = CASES.lines().filter(|line| !line.is_empty())
        .map(|line| line.replace("FILE", &lisp(&file)).replace("EMPTY", &lisp(&empty)))
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(&expression), romacs(&mut vm, &expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// REPLACE (fileio.c:4684-4880, restore_window_points): only the text
// between the common head and tail changes, as a deletion and an insertion
// with their hooks; markers in the unchanged parts stay, point inside the
// replaced text is spread over the new text, and a file that is the
// buffer's head truncates it. Narrowing limits the replaced region.
const REPLACE_CASES: &str = r##"
(progn (insert TEXT) (goto-char PT) (let ((mb (copy-marker 2)) (md (copy-marker 7))) (setq-local after-change-functions (list (lambda (b e l) (ifc-log 'a b e l)))) (setq-local before-change-functions (list (lambda (b e) (ifc-log 'b b e)))) NARROW (let ((ret (insert-file-contents FILE nil nil nil t))) (widen) (list (buffer-string) (point) (marker-position mb) (marker-position md) (cadr ret) (reverse log)))))
"##;

#[test]
fn insert_file_contents_replace_matches_gnu() {
    let dir: PathBuf = std::env::temp_dir().join(format!("romacs-ifc-replace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let files = [("f1", &b"abc12def"[..]), ("f2", b"abc"), ("f3", "\u{3bb}b".as_bytes()), ("f4", b"xbcy"), ("f5", b"")];
    for (name, bytes) in files { std::fs::write(dir.join(name), bytes).unwrap(); }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let cases = [("abcXYZdef", "f1", "(point-max)", "nil"), ("abcdef", "f2", "(point-max)", "nil"),
                 ("\u{3bb}a", "f3", "(point-max)", "nil"), ("abcXYZdef", "f1", "5", "nil"),
                 ("abcd", "f4", "(point-max)", "(narrow-to-region 2 4)"), ("abc", "f5", "2", "nil"),
                 ("abc", "f2", "2", "nil")];
    let failures: Vec<String> = cases.iter().map(|(text, file, point, narrow)| {
            REPLACE_CASES.trim().replace("TEXT", &serde_json::to_string(text).unwrap()).replace("FILE", &lisp(&dir.join(file)))
                .replace("PT", point).replace("NARROW", narrow)
        })
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(&expression), romacs(&mut vm, &expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// Undo records of a read (insert_from_gap's record_insert): REPLACE records
// the deleted old middle and the inserted new one, so one undo restores the
// buffer; a plain read records its insertion; a visit records it when the
// undo list was not empty (and empties a list that was).
const UNDO_CASES: &str = r##"
(progn (set-buffer (generate-new-buffer "ifc-undo")) (buffer-enable-undo) (setq buffer-undo-list nil) (insert "abcXYZdef") (undo-boundary) (insert-file-contents F1 nil nil nil t) (list (buffer-string) buffer-undo-list (progn (primitive-undo 1 buffer-undo-list) (buffer-string))))
(progn (set-buffer (generate-new-buffer "ifc-undo")) (buffer-enable-undo) (setq buffer-undo-list nil) (insert "ab") (undo-boundary) (goto-char 2) (insert-file-contents F2) (list (buffer-string) buffer-undo-list (progn (primitive-undo 1 buffer-undo-list) (buffer-string))))
(progn (set-buffer (generate-new-buffer "ifc-undo")) (buffer-enable-undo) (setq buffer-undo-list (list 'foo)) (insert-file-contents F2 t) (setq buffer-file-name nil) buffer-undo-list)
(progn (set-buffer (generate-new-buffer "ifc-undo")) (buffer-enable-undo) (setq buffer-undo-list nil) (insert-file-contents F2 t) (setq buffer-file-name nil) buffer-undo-list)
"##;

#[test]
fn insert_file_contents_records_undo_as_gnu() {
    let dir: PathBuf = std::env::temp_dir().join(format!("romacs-ifc-undo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f1"), b"abc12def").unwrap();
    std::fs::write(dir.join("f2"), b"XY").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let failures: Vec<String> = UNDO_CASES.lines().filter(|line| !line.is_empty())
        .map(|line| line.replace("F1", &lisp(&dir.join("f1"))).replace("F2", &lisp(&dir.join("f2"))))
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(&expression), romacs(&mut vm, &expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
