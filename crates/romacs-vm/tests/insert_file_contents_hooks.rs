//! insert-file-contents runs GNU's change bracket for a read that is not
//! a visit (fileio.c:4889 prepare_to_modify_buffer, 5132
//! signal_after_change): the before-change hooks at point, the
//! after-change hooks over the inserted text with point before it, the
//! text's insertion hooks and first-change-hook. Compared against pinned GNU.
use romacs_vm::Vm;
use std::path::{Path, PathBuf};
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;
#[path = "support/template.rs"]
mod template;

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
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    let failures: Vec<String> = CASES.lines().filter(|line| !line.is_empty())
        .map(|line| template::fill(line, &[("FILE", &lisp(&file)), ("EMPTY", &lisp(&empty))]))
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
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    let cases = [("abcXYZdef", "f1", "(point-max)", "nil"), ("abcdef", "f2", "(point-max)", "nil"),
                 ("\u{3bb}a", "f3", "(point-max)", "nil"), ("abcXYZdef", "f1", "5", "nil"),
                 ("abcd", "f4", "(point-max)", "(narrow-to-region 2 4)"), ("abc", "f5", "2", "nil"),
                 ("abc", "f2", "2", "nil")];
    let failures: Vec<String> = cases.iter().map(|(text, file, point, narrow)| {
            template::fill(REPLACE_CASES.trim(), &[("TEXT", &serde_json::to_string(text).unwrap()), ("FILE", &lisp(&dir.join(file))),
                                                  ("PT", point), ("NARROW", narrow)])
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
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    let failures: Vec<String> = UNDO_CASES.lines().filter(|line| !line.is_empty())
        .map(|line| template::fill(line, &[("F1", &lisp(&dir.join("f1"))), ("F2", &lisp(&dir.join("f2")))]))
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(&expression), romacs(&mut vm, &expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// A REPLACE whose new middle holds a raw-byte character puts it in the
// buffer as GNU does (VM2-RAWTEXT1). Pinned GNU: (("…/f" 2) "abcλ\377def" 9 nil).
#[test]
fn a_raw_byte_replace_matches_gnu() {
    let dir: PathBuf = std::env::temp_dir().join(format!("romacs-ifc-raw-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f"), b"abc\xce\xbb\xffdef").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    let expression = format!("(progn (set-buffer (generate-new-buffer \"ifc-raw\")) (buffer-enable-undo) (insert \"abcXYZdef\") (undo-boundary)
      (let ((before (copy-tree buffer-undo-list)) (coding-system-for-read 'utf-8))
        (list (condition-case e (insert-file-contents {} nil nil nil t) (error e)) (buffer-string) (point) (equal buffer-undo-list before))))",
        lisp(&dir.join("f")));
    let got = romacs(&mut vm, &expression);
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(got, format!("\"(({} 2) \\\"abcλ\\\\377def\\\" 9 nil)\"", lisp(&dir.join("f")).replace('"', "\\\"")));
}

// REPLACE's read-only checks are those of the ranges it changes: the
// deleted middle (delete-region) and the insertion point after it, not
// the original point. Point at 2 under read-only text changes nothing.
const READ_ONLY_CASES: &str = r##"
(progn (set-buffer (generate-new-buffer "ifc-ro")) (insert "abcXYZdef") (let ((inhibit-read-only t)) (put-text-property RO 'read-only t)) (goto-char 2) (list (condition-case e (cadr (insert-file-contents FILE nil nil nil t)) (error e)) (buffer-substring-no-properties 1 (point-max)) (point)))
"##;

#[test]
fn insert_file_contents_replace_checks_the_ranges_it_changes() {
    let dir: PathBuf = std::env::temp_dir().join(format!("romacs-ifc-ro-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f1"), b"abc12def").unwrap();
    std::fs::write(dir.join("same"), b"abcXYZdef").unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    let failures: Vec<String> = [("1 3", "f1"), ("1 3", "same"), ("4 7", "f1"), ("3 4", "f1")].iter()
        .map(|(ro, file)| template::fill(READ_ONLY_CASES.trim(), &[("RO", ro), ("FILE", &lisp(&dir.join(file)))]))
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(&expression), romacs(&mut vm, &expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// A visiting REPLACE of the whole buffer (fileio.c:4647, 4828) binds
// `buffer-file-name' to nil around its deletion and insertion, so a file
// changed on disk is read in (auto-revert, revert-buffer) without locking it
// or asking about a supersession threat, and its change hooks see no file
// name; narrowed, the change asks as any first change does. The truename is set unabbreviated so the lock's
// visited-buffer lookup sees the buffer wherever the file lives.
const VISIT_CASE: &str = r##"(let ((threats 0)) (fset 'userlock--ask-user-about-supersession-threat (lambda (_f) (setq threats (1+ threats)))) (write-region "one" nil FILE nil 'silent) (set-buffer (find-file-noselect FILE)) (setq buffer-file-truename (file-truename FILE)) (write-region "zzz" nil FILE nil 'silent) (set-file-times FILE (time-add nil 5)) (setq log nil) (setq-local before-change-functions (list (lambda (b e) (ifc-log 'b b e (and buffer-file-name t))))) (setq-local after-change-functions (list (lambda (b e l) (ifc-log 'a b e l (and buffer-file-name t))))) NARROW (insert-file-contents FILE t nil nil t) (prog1 (list threats (buffer-string) (buffer-modified-p) (reverse log)) (set-buffer-modified-p nil) (kill-buffer)))"##;

#[test]
fn visiting_replace_does_not_ask_about_supersession_as_gnu() {
    let dir: PathBuf = std::env::temp_dir().join(format!("romacs-ifc-visit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("visited.txt");
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = gnu_ert_vm::bootstrapped_with(&root, "lisp/romacs/romacs-loadup-bootstrap.el");
    let failures: Vec<String> = ["", "(narrow-to-region 1 2)"].iter()
        .map(|narrow| template::fill(VISIT_CASE, &[("FILE", &lisp(&file)), ("NARROW", narrow)]))
        .filter_map(|expression| {
            let (gnu, ours) = (oracle(&expression), romacs(&mut vm, &expression));
            (gnu != ours).then(|| format!("{expression}\n  gnu:    {gnu}\n  romacs: {ours}"))
        })
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
