use romacs_vm::Vm;
use std::path::Path;
#[path = "support/gnu_ert_vm.rs"]
mod gnu_ert_vm;
use gnu_ert_vm::bootstrapped_with;

fn bootstrapped(root: &Path) -> Vm {
    bootstrapped_with(root, "rlisp/romacs-gnu-ert-bootstrap.el")
}

#[test]
fn complete_gnu_ert_dependency_closure() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped(&root);
    assert!(vm.load_report()["steps"].as_array().unwrap().iter().all(|step| step["status"] == "ok"), "strict ERT bootstrap retained a failed load form");
    assert_eq!(vm.eval_source("(list (featurep 'ert) (fboundp 'ert-run-tests-batch))").unwrap().display().unwrap(), "(t t)");
    assert_eq!(vm.eval_source("(list (featurep 'mule) (fboundp 'sgml-html-meta-auto-coding-function))").unwrap().display().unwrap(), "(t t)");
    assert_eq!(vm.eval_source("(list (function-documentation (lambda () \"Private doc\" nil)) (condition-case e (length (lambda () \"Opaque\" nil)) (wrong-type-argument (car (cdr e)))))").unwrap().display().unwrap(), "(\"Private doc\" sequencep)");
    assert_eq!(vm.eval_source("(let ((function (lambda () \"Private source\" nil))) (mapcar (lambda (circle) (let ((print-circle circle)) (list (equal (prin1-to-string function) (with-output-to-string (cl-prin1 function))) (condition-case nil (progn (length function) nil) (wrong-type-argument t))))) '(nil t)))").unwrap().display().unwrap(), "((t t) (t t))", "OPAQUE-GNU-CL-PRINT");
    vm.eval_source("(require 'hfy-cmap)").unwrap();
    assert_eq!(vm.eval_source("(list emacs-major-version emacs-minor-version (with-temp-buffer (insert \"255 250 250\\t\\tsnow\\n248 248 255\\t\\tghost white\\n248 248 255\\t\\tGhostWhite\\n\") (hfy-cmap--parse-buffer (current-buffer))))").unwrap().display().unwrap(), "(32 0 ((\"GhostWhite\" 248 248 255) (\"ghost white\" 248 248 255) (\"snow\" 255 250 250)))");
}

// GNU's Unicode tables, with their byte-code functions in source
// (DIVERGENCES.md VM2-UNIDATA1), give pinned GNU's property values.
#[test]
fn unicode_character_properties_match_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped(&root);
    vm.eval_source("(load \"international/charprop.el\" nil t t)").unwrap();
    let source = "(prin1-to-string (list (mapcar (lambda (c) (mapcar (lambda (p) (get-char-code-property c p)) '(name general-category decomposition bidi-class old-name canonical-combining-class mirroring uppercase))) '(?a #xE9 #xAC01 #xD7A3 #x4E00 #x1F600 #x5D0 #x558 #xFB01 #x2160 #xE0100 #x300)) (mapcar (lambda (p) (char-code-property-description p (get-char-code-property #x5D0 p))) '(general-category bidi-class canonical-combining-class))))";
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, expected);
}

// coding.c's charset coding type, through mule-conf.el's definitions.
#[test]
fn charset_coding_systems_match_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped(&root);
    vm.eval_source("(progn (load \"language/european.el\" nil t t) (load \"language/chinese.el\" nil t t))").unwrap();
    let source = "(prin1-to-string (list (decode-coding-string \"\\341\" 'iso-8859-15) (decode-coding-string \"a\\244\\341\\r\\nb\" 'iso-8859-15-dos) (encode-coding-string \"a\\u00e1\\u20ac\\u0100\" 'iso-8859-15) (encode-coding-string \"\\u00e1\" 'iso-8859-2) (decode-coding-string \"\\x81 a\\xe9\" 'windows-1252) last-coding-system-used (encode-coding-string (string #x3fff81 ?a) 'latin-1) (decode-coding-string \"\\244\\242\" 'chinese-gbk) (encode-coding-string \"\\u4e00\" 'chinese-gbk) (decode-coding-string \"\\201\" 'chinese-gbk) (decode-coding-string \"a\" 'iso-8859-15) (decode-coding-string \"\\xe9\\xe9\" 'cp437)))";
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, expected);
}

// fileio.c choose_write_coding_system and the encoding after it, on the
// loadup chain's coding systems: the value and the bytes written must be
// pinned GNU's. Each case writes the file FILE, named by its extension.
#[test]
fn write_region_coding_matches_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("rt-write-coding-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cases: [(&str, &str); 24] = [
        ("txt", r#"(with-temp-buffer (insert "héllo\n") (list (write-region nil nil FILE) last-coding-system-used))"#),
        // file-coding-system-alist's prefer-utf-8, then select-safe-coding-system.
        ("el", r#"(with-temp-buffer (insert "λ x\n") (write-region nil nil FILE) last-coding-system-used)"#),
        ("bin", r#"(with-temp-buffer (set-buffer-multibyte nil) (insert "h\351\n") (write-region nil nil FILE) last-coding-system-used)"#),
        ("txt", r#"(let ((coding-system-for-write 'latin-1)) (write-region "café\n" nil FILE) last-coding-system-used)"#),
        ("txt", r#"(with-temp-buffer (setq buffer-file-coding-system 'utf-8-dos) (insert "a\nb") (write-region nil nil FILE) last-coding-system-used)"#),
        ("txt", r#"(with-temp-buffer (setq buffer-file-coding-system 'latin-1) (insert "é") (write-region nil nil FILE) (list last-coding-system-used (local-variable-p 'buffer-file-coding-system)))"#),
        // A nil START widens: the whole buffer, the restriction kept.
        ("txt", r#"(with-temp-buffer (insert "xéyz") (narrow-to-region 2 3) (write-region nil nil FILE) (list last-coding-system-used (point-min) (point-max)))"#),
        ("txt", r#"(with-temp-buffer (insert "été") (let ((coding-system-for-write 'utf-8)) (write-region 2 2 FILE)) last-coding-system-used)"#),
        ("txt", r#"(progn (write-region "é" nil FILE) last-coding-system-used)"#),
        // A function entry of the alist receives write-region's seven arguments.
        ("rtfn", r#"(let* ((seen nil) (file-coding-system-alist (list (cons "\\.rtfn\\'" 'rt-rtfn-coding)))) (defalias 'rt-rtfn-coding (lambda (args) (setq seen (list (car args) (nth 1 args) (nth 2 args) (length args))) 'latin-1)) (with-temp-buffer (insert "é!") (write-region nil nil FILE) (list seen last-coding-system-used)))"#),
        ("txt", r#"(let ((select-safe-coding-system-function 'rt-select-utf-16le)) (defalias 'rt-select-utf-16le (lambda (&rest _) 'utf-16le)) (with-temp-buffer (insert "é\n") (write-region nil nil FILE) last-coding-system-used))"#),
        // The function sees the widened buffer of a nil START.
        ("txt", r#"(let* ((seen nil) (select-safe-coding-system-function 'rt-select-seen)) (defalias 'rt-select-seen (lambda (from to &rest _) (setq seen (list from to (point-min) (point-max))) nil)) (with-temp-buffer (insert "abcé") (narrow-to-region 2 3) (write-region nil nil FILE) (list seen last-coding-system-used (point-min) (point-max))))"#),
        ("txt", r#"(let* ((seen nil) (coding-system-require-warning t) (coding-system-for-write 'latin-1) (select-safe-coding-system-function 'rt-select-warned)) (defalias 'rt-select-warned (lambda (_from _to codings &rest _) (setq seen codings) 'utf-8)) (write-region "é" nil FILE) (list seen last-coding-system-used))"#),
        ("txt", r#"(with-temp-buffer (setq buffer-file-coding-system 'raw-text-dos) (insert "é\n") (write-region nil nil FILE) last-coding-system-used)"#),
        ("txt", r#"(let ((coding-system-for-write 'undecided-dos)) (write-region "é\nx" nil FILE) last-coding-system-used)"#),
        ("txt", r#"(list (let ((coding-system-for-write 'rt-nosuch)) (condition-case e (write-region "x" nil FILE) (error e))) (file-exists-p FILE))"#),
        // CHECK_CODING_SYSTEM evaluates a lazy coding system's define form.
        ("txt", r#"(progn (put 'rt-lazy 'coding-system-define-form '(define-coding-system-alias 'rt-lazy 'latin-1)) (let ((coding-system-for-write 'rt-lazy)) (write-region "é" nil FILE)) (list last-coding-system-used (get 'rt-lazy 'coding-system-define-form)))"#),
        ("txt", r#"(list (let ((select-safe-coding-system-function (lambda (&rest _) 'utf-8))) (condition-case e (write-region "x" nil FILE) (error (list (car e) (cadr e))))) (let ((select-safe-coding-system-function 'rt-select-string)) (defalias 'rt-select-string (lambda (&rest _) "utf-8")) (condition-case e (write-region "x" nil FILE) (error e))))"#),
        // One save-restriction extent covers the choice and the write: a lazy
        // define form sees the widened buffer, and a narrowing select
        // function is undone on return, error and throw.
        ("txt", r#"(progn (defvar rt-lazy-seen nil) (put 'rt-lazy-wide 'coding-system-define-form '(progn (setq rt-lazy-seen (list (point-min) (point-max))) (define-coding-system-alias 'rt-lazy-wide 'latin-1))) (with-temp-buffer (insert "abcd") (narrow-to-region 2 3) (let ((coding-system-for-write 'rt-lazy-wide)) (write-region nil nil FILE)) (list rt-lazy-seen last-coding-system-used (point-min) (point-max))))"#),
        ("txt", r#"(let ((select-safe-coding-system-function 'rt-select-narrow)) (defalias 'rt-select-narrow (lambda (&rest _) (narrow-to-region 2 3) nil)) (with-temp-buffer (insert "abcd") (write-region nil nil FILE) (list last-coding-system-used (point-min) (point-max))))"#),
        ("txt", r#"(progn (defvar rt-escape nil) (defalias 'rt-select-escape (lambda (&rest _) (narrow-to-region 2 3) (if rt-escape (throw 'rt-escape 'thrown) (error "rt-escape")))) (let ((select-safe-coding-system-function 'rt-select-escape)) (with-temp-buffer (insert "abcd") (list (condition-case e (write-region nil nil FILE) (error (cadr e))) (point-min) (point-max) (progn (setq rt-escape t) (catch 'rt-escape (write-region 1 4 FILE))) (point-min) (point-max)))))"#),
        // A narrowed unibyte buffer keeps its restriction across the extent.
        ("txt", r#"(with-temp-buffer (set-buffer-multibyte nil) (insert "abcd") (narrow-to-region 2 4) (list (let ((coding-system-for-write 'no-conversion)) (write-region 2 4 FILE)) last-coding-system-used (point-min) (point-max)))"#),
        ("txt", r#"(with-temp-buffer (set-buffer-multibyte nil) (insert "abcd") (narrow-to-region 2 4) (list (let ((coding-system-for-write 'no-conversion)) (write-region nil nil FILE)) (point-min) (point-max)))"#),
        // A coding function sees the live START string, not a copy.
        ("rtid", r#"(let* ((s (copy-sequence "λbc")) (same nil) (file-coding-system-alist (list (cons "\\.rtid\\'" (lambda (args) (setq same (and (memq s args) t)) (aset s 1 ?z) 'utf-8))))) (write-region s nil FILE) (list same s last-coding-system-used))"#),
    ];
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for (index, (extension, expression)) in cases.iter().enumerate() {
        let file = dir.join(format!("case-{index}.{extension}"));
        let expression = expression.replace("FILE", &format!("{:?}", file.to_str().unwrap()));
        let source = format!("(string-to-list (prin1-to-string (condition-case err {expression} (error (list 'signal err)))))");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = (String::from_utf8(output.stdout).unwrap(), std::fs::read(&file).ok());
        let _ = std::fs::remove_file(&file);
        let actual = (vm.eval_source(&source).unwrap().display().unwrap(), std::fs::read(&file).ok());
        if actual != expected {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {} {:?}\n  Romacs {} {:?}", text(&expected.0), expected.1, text(&actual.0), actual.1));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// GNU's write path runs a pre-write conversion and encode translation table
// even on ASCII text; Romacs refuses both before opening the destination.
// fileio.c Finsert_file_contents from the coding decision on, on the
// loadup chain: set-auto-coding on the file's bytes as unibyte text (or a
// work buffer when the buffer has text), file-coding-system-alist,
// detection in priority order, decoding, the visit bookkeeping and the Lisp
// tail. Each case reads FILE, holding the case's bytes; the printed result
// must be pinned GNU's.
#[test]
fn insert_file_contents_coding_matches_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("rt-insert-coding-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // Only a local buffer-file-coding-system: GNU's startup sets the default
    // (set-locale-environment), which the loadup chain does not run. Undo
    // lists are compared where GNU records no insertion: VM2 records no
    // insertion undo (DIVERGENCES.md VM2-EDIT-HOOKS1).
    let state = "last-coding-system-used (buffer-string) (and (local-variable-p 'buffer-file-coding-system) buffer-file-coding-system) (point) (buffer-modified-p) enable-multibyte-characters";
    let read = format!("(with-temp-buffer (list (cdr (insert-file-contents FILE)) {state}))");
    let big = [vec![b'x'; 6000], b"\n;; Local Variables:\n;; coding: latin-1\n;; End:\n\xe9\n".to_vec()].concat();
    // The charset detector, with iso-latin-1 ahead of the ISO-2022
    // categories VM2 cannot detect.
    let encode = "(default-file-name-coding-system 'utf-8-unix) (file-name-coding-system nil) (set-auto-coding-function (lambda (f n) (setq seen (list last-coding-system-used (multibyte-string-p f) (equal f (encode-coding-string FILE 'utf-8-unix t)) n)) 'latin-1)))";
    let latin = "(let ((saved (coding-system-priority-list))) (unwind-protect (progn (set-coding-system-priority 'iso-latin-1) (with-temp-buffer (list (cdr (insert-file-contents FILE)) last-coding-system-used (buffer-string)))) (apply #'set-coding-system-priority saved)))";
    let cases: Vec<(&str, Vec<u8>, String)> = vec![
        ("txt", "* Heading\nλ body\n".into(), read.clone()),
        ("txt", b"hi\n".to_vec(), read.clone()),
        ("txt", b"a\r\nb\r\n".to_vec(), read.clone()),
        ("txt", b"a\rb\r".to_vec(), read.clone()),
        ("txt", b"a\0b".to_vec(), read.clone()),
        // auto-coding-regexp-alist: the signature, then a coding cookie.
        ("txt", b"\xef\xbb\xbfx\n".to_vec(), read.clone()),
        ("el", b";; -*- coding: latin-1 -*-\ncaf\xe9\n".to_vec(), read.clone()),
        // C1 bytes mule-conf.el's latin-extra-code-table admits, and CRLF.
        ("txt", b"caf\xe9 \x93q\x94\n".to_vec(), latin.into()),
        ("txt", b"caf\xe9\r\n".to_vec(), latin.into()),
        // The cookie in the 3 KiB tail, into an empty buffer and into text.
        ("el", big.clone(), "(with-temp-buffer (list (cdr (insert-file-contents FILE)) last-coding-system-used (substring (buffer-string) -3)))".into()),
        ("el", big, "(with-temp-buffer (insert \"AB\") (goto-char 2) (list (cdr (insert-file-contents FILE)) last-coding-system-used (substring (buffer-string) -4) (point)))".into()),
        ("txt", "λ\r\n".into(), "(with-temp-buffer (insert \"AB\") (goto-char 2) (list (cdr (insert-file-contents FILE)) last-coding-system-used (buffer-string) (point) deactivate-mark))".into()),
        ("txt", "* Heading\nλ body\n".into(), "(with-temp-buffer (set-buffer-multibyte nil) (list (cdr (insert-file-contents FILE)) last-coding-system-used (buffer-string)))".into()),
        ("txt", b"a\r\nb\r\n".to_vec(), "(with-temp-buffer (set-buffer-multibyte nil) (list (cdr (insert-file-contents FILE)) last-coding-system-used (buffer-string)))".into()),
        ("txt", b"a\r\nb\r\n".to_vec(), "(with-temp-buffer (list (cdr (insert-file-contents-literally FILE)) last-coding-system-used (buffer-string) enable-multibyte-characters))".into()),
        ("txt", b"\xce\xbb\0\xff".to_vec(), "(with-temp-buffer (set-buffer-multibyte nil) (list (cdr (insert-file-contents-literally FILE)) last-coding-system-used (append (buffer-string) nil)))".into()),
        ("txt", b"caf\xe9\n".to_vec(), "(with-temp-buffer (list (cdr (let ((coding-system-for-read 'latin-1)) (insert-file-contents FILE))) last-coding-system-used (buffer-string)))".into()),
        ("txt", b"hi\n".to_vec(), "(with-temp-buffer (list (cdr (let ((coding-system-for-read 'utf-8)) (insert-file-contents FILE))) last-coding-system-used))".into()),
        ("txt", b"hi\n".to_vec(), "(with-temp-buffer (list (cdr (let ((coding-system-for-read 'raw-text)) (insert-file-contents FILE t))) last-coding-system-used enable-multibyte-characters))".into()),
        ("txt", "* Heading\nλ body\n".into(), "(with-temp-buffer (list (cdr (insert-file-contents FILE nil 2 9)) (buffer-string) last-coding-system-used))".into()),
        // Visiting: the undo list starts empty, the buffer unmodified.
        ("txt", "* Heading\nλ body\n".into(), format!("(with-temp-buffer (buffer-enable-undo) (list (cdr (insert-file-contents FILE t)) buffer-undo-list {state}))")),
        ("txt", b"".to_vec(), format!("(with-temp-buffer (list (cdr (insert-file-contents FILE t)) {state}))")),
        // set-auto-coding-function sees the bytes as the unibyte text of the
        // buffer, undo off; a non-local exit leaves the buffer as it was.
        ("txt", "é\n".into(), "(let* ((seen nil) (set-auto-coding-function (lambda (f n) (setq seen (list (equal f FILE) n enable-multibyte-characters (append (buffer-string) nil) buffer-undo-list (point))) 'latin-1))) (with-temp-buffer (buffer-enable-undo) (list (cdr (insert-file-contents FILE)) seen last-coding-system-used (buffer-string))))".into()),
        ("txt", "é\n".into(), "(let ((set-auto-coding-function (lambda (&rest _) (error \"rt-boom\")))) (with-temp-buffer (buffer-enable-undo) (list (condition-case e (insert-file-contents FILE) (error (cadr e))) (buffer-string) enable-multibyte-characters buffer-undo-list)))".into()),
        // A non-ASCII name reaches the function as ENCODE_FILE makes it:
        // encoded by `default-file-name-coding-system', unrecorded, into an
        // empty buffer and into the work buffer of a buffer with text.
        ("λ", "é\n".into(), format!("(let* ((seen nil) {encode} (with-temp-buffer (setq last-coding-system-used 'no-conversion) (list (cdr (insert-file-contents FILE)) seen (buffer-string))))")),
        ("λ", "é\n".into(), format!("(let* ((seen nil) {encode} (with-temp-buffer (insert \"x\") (setq last-coding-system-used 'no-conversion) (list (cdr (insert-file-contents FILE)) seen (buffer-string))))")),
        // The work buffer of a buffer with text.
        ("txt", "é\n".into(), "(let* ((seen nil) (set-auto-coding-function (lambda (f n) (setq seen (list (buffer-name) n enable-multibyte-characters (append (buffer-string) nil) buffer-undo-list buffer-read-only)) nil))) (with-temp-buffer (insert \"x\") (list (cdr (insert-file-contents FILE)) seen last-coding-system-used)))".into()),
        // file-coding-system-alist: a function entry gets the arguments.
        ("rtfn", "é\n".into(), "(let* ((seen nil) (file-coding-system-alist (list (cons \"\\\\.rtfn\\\\'\" (lambda (args) (setq seen (list (car args) (length args) (nthcdr 2 args))) '(latin-1 . latin-1)))))) (with-temp-buffer (list (cdr (insert-file-contents FILE)) seen last-coding-system-used (buffer-string))))".into()),
        // CHECK_CODING_SYSTEM evaluates a lazy coding system's define form.
        ("txt", "é\n".into(), "(progn (put 'rt-ifc-lazy 'coding-system-define-form '(define-coding-system-alias 'rt-ifc-lazy 'latin-1)) (with-temp-buffer (list (cdr (let ((coding-system-for-read 'rt-ifc-lazy)) (insert-file-contents FILE))) last-coding-system-used (buffer-string))))".into()),
        // The Lisp tail: format-decode, then after-insert-file-functions
        // with both inhibit variables bound and undo off.
        ("txt", b"Content-Type: text/enriched\nText-Width: 70\n\n<bold>hi</bold>\n".to_vec(), "(with-temp-buffer (list (cdr (insert-file-contents FILE)) (buffer-string) buffer-file-format))".into()),
        ("txt", b"hi\n".to_vec(), "(let* ((seen nil) (after-insert-file-functions (list (lambda (n) (setq seen (list n inhibit-modification-hooks inhibit-point-motion-hooks buffer-undo-list)) (insert \"Z\") (1+ n))))) (with-temp-buffer (buffer-enable-undo) (list (cdr (insert-file-contents FILE)) seen (buffer-string))))".into()),
        ("txt", b"hi\n".to_vec(), "(let ((after-insert-file-functions (list (lambda (_) 99)))) (with-temp-buffer (condition-case e (insert-file-contents FILE) (error e))))".into()),
        // A visited missing file decides and records before it signals.
        ("txt", b"".to_vec(), "(with-temp-buffer (delete-file FILE) (list (condition-case e (let ((coding-system-for-read 'latin-1)) (insert-file-contents FILE t)) (error (car e))) last-coding-system-used (and (local-variable-p 'buffer-file-coding-system) buffer-file-coding-system) (buffer-modified-p)))".into()),
        ("txt", b"".to_vec(), "(with-temp-buffer (delete-file FILE) (list (condition-case e (let ((coding-system-for-read 'raw-text)) (insert-file-contents FILE t)) (error (car e))) last-coding-system-used enable-multibyte-characters))".into()),
        // Markers and overlays of the empty buffer survive its unibyte
        // decision phase, and save-excursion restores point.
        ("txt", "é\n".into(), "(with-temp-buffer (let ((m (point-marker)) (m2 (copy-marker 1 t)) (o (make-overlay 1 1))) (list (save-excursion (cdr (insert-file-contents FILE))) (point) (marker-position m) (marker-position m2) (overlay-start o) (overlay-end o) (buffer-string) last-coding-system-used)))".into()),
        // A file of exactly 4 KiB: with END omitted the tail read finds EOF
        // and set-auto-coding-function sees 1 KiB; END = 4096 keeps 4 KiB.
        ("txt", [vec![b'y'; 4095], b"\n".to_vec()].concat(), "(let* ((seen nil) (set-auto-coding-function (lambda (_f n) (push n seen) nil))) (with-temp-buffer (insert \"x\") (insert-file-contents FILE) (insert-file-contents FILE nil 0 4096) (insert-file-contents FILE nil 0 5000) (insert-file-contents FILE nil 1) (list (nreverse seen) (buffer-size))))".into()),
    ];
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for (index, (extension, bytes, expression)) in cases.iter().enumerate() {
        let file = dir.join(format!("case-{index}.{extension}"));
        let expression = expression.replace("FILE", &format!("{:?}", file.to_str().unwrap()));
        let source = format!("(let ((print-escape-newlines t) (print-escape-control-characters t)) (string-to-list (prin1-to-string (condition-case err {expression} (error (list 'signal err))))))");
        std::fs::write(&file, bytes).unwrap();
        let output = std::process::Command::new(&binary).current_dir(&dir)
            .args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        std::fs::write(&file, bytes).unwrap();
        let actual = vm.eval_source(&source).unwrap().display().unwrap();
        if actual != expected {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {}", text(&expected), text(&actual)));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// What insert-file-contents cannot decode yet it refuses before the buffer
// changes, with the decision's buffer state restored.
#[test]
fn insert_file_contents_refuses_unsupported_decoding_intact() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("rt-insert-refuse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A cookie-less Latin-1 file reads as in pinned GNU: GNU's walk rejects
    // the ISO-2022 categories and finds iso-latin-1 (VM2-FILEIO1).
    let latin = dir.join("latin.txt");
    std::fs::write(&latin, b"caf\xe9\n").unwrap();
    let probe = format!("(with-temp-buffer (insert \"keep\") (goto-char 3) (list (cadr (insert-file-contents {:?})) (buffer-substring-no-properties 1 (point-max)) (point) last-coding-system-used (get-text-property 3 'charset)))", latin.to_str().unwrap());
    assert_eq!(vm.eval_source(&probe).unwrap().display().unwrap(), "(5 \"kecafé\\nep\" 3 iso-latin-1-unix iso-8859-1)");
    let cases: [(&[u8], &str); 3] = [
        // GNU's walk finds iso-2022-7bit, which VM2 cannot decode yet.
        (b"a\x1b$Bb\n", "insert-file-contents decoding with iso-2022-7bit"),
        // GNU decodes these as no-conversion and utf-8, keeping raw bytes as
        // eight-bit characters a VM2 buffer cannot hold.
        (b"\xe9\0\xff\0", "insert-file-contents of raw bytes into a multibyte buffer"),
        (b"\xc3", "insert-file-contents of raw bytes into a multibyte buffer"),
    ];
    for (index, (bytes, refusal)) in cases.into_iter().enumerate() {
        let file = dir.join(format!("case-{index}.txt"));
        std::fs::write(&file, bytes).unwrap();
        let probe = format!("(with-temp-buffer (insert \"keep\") (goto-char 3) (buffer-enable-undo) (let ((before buffer-undo-list) (lcsu last-coding-system-used)) (list (condition-case e (insert-file-contents {:?}) (error (cadr e))) (buffer-string) (point) (eq buffer-undo-list before) (eq last-coding-system-used lcsu) enable-multibyte-characters)))", file.to_str().unwrap());
        let actual = vm.eval_source(&probe).unwrap().display().unwrap();
        assert_eq!(actual, format!("(\"VM2 {refusal} is unsupported\" \"keep\" 3 t t t)"), "{bytes:?}");
        let empty = format!("(with-temp-buffer (list (condition-case e (insert-file-contents {:?}) (error (cadr e))) (buffer-size) enable-multibyte-characters buffer-undo-list))", file.to_str().unwrap());
        let actual = vm.eval_source(&empty).unwrap().display().unwrap();
        assert_eq!(actual, format!("(\"VM2 {refusal} is unsupported\" 0 t t)"), "empty {bytes:?}");
    }
    // VM2-IFC-HELD1: the decision phase holds the empty buffer's markers, so
    // set-auto-coding-function may not make it multibyte; the markers survive.
    let file = dir.join("held.txt");
    std::fs::write(&file, b"hi\n").unwrap();
    let probe = format!("(with-temp-buffer (let ((m (point-marker)) (set-auto-coding-function (lambda (&rest _) (set-buffer-multibyte t)))) (list (condition-case e (save-excursion (insert-file-contents {:?})) (error (cadr e))) (point) (marker-position m) (buffer-size) enable-multibyte-characters)))", file.to_str().unwrap());
    assert_eq!(vm.eval_source(&probe).unwrap().display().unwrap(), "(\"cannot decode buffer while insert-file-contents decides its coding\" 1 1 0 t)");
    let _ = std::fs::remove_dir_all(&dir);
}

// Where GNU would build a composition glyph string, a static or an
// automatic one in a displayed buffer, find-composition-internal refuses
// (VM2-COMPOSE1) instead of answering nil.
#[test]
fn find_composition_refuses_compositions_it_cannot_build() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    for (form, kind) in [
        ("(find-composition-internal 0 1 \"e\\u0301x\" nil)", "with automatic composition"),
        ("(progn (set-window-buffer nil (get-buffer-create \"rt-c\")) (set-buffer \"rt-c\") (insert \"e\\u0301x\") (find-composition-internal 1 nil nil nil))", "with automatic composition"),
        ("(let ((s (copy-sequence \"abc\"))) (put-text-property 0 2 'composition '(2 . ?X) s) (with-temp-buffer (find-composition-internal 0 nil s nil)))", "on a static composition"),
        ("(let ((s (copy-sequence \"abc\"))) (put-text-property 1 2 'composition '(1 . ?X) s) (with-temp-buffer (insert \"xxxx\") (find-composition-internal 0 3 s nil)))", "on a static composition"),
        // The property is found below a narrowing, searching back to LIMIT.
        ("(with-temp-buffer (insert \"abc\") (put-text-property 1 2 'composition '(1 . ?X)) (narrow-to-region 2 4) (find-composition-internal 2 1 nil nil))", "on a static composition"),
        ("(with-temp-buffer (insert \"abc\") (put-text-property 1 2 'composition '(1 . ?X)) (narrow-to-region 2 4) (find-composition-internal 3 1 nil nil))", "on a static composition"),
    ] {
        let actual = vm.eval_source(&format!("(condition-case e {form} (error (cadr e)))")).unwrap().display().unwrap();
        assert_eq!(actual, format!("\"VM2 find-composition-internal {kind} is unsupported\""), "{form}");
    }
}

#[test]
fn write_region_refuses_unsupported_conversion_before_opening() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("rt-write-refuse-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let setup = r#"(defvar rt-pre-called nil) (defalias 'rt-pre-write (lambda (_from _to) (setq rt-pre-called t) (goto-char (point-min)) (insert "PRE"))) (define-coding-system 'rt-pre-utf-8 "Pre." :coding-type 'utf-8 :mnemonic ?U :charset-list '(unicode) :pre-write-conversion 'rt-pre-write) (define-coding-system 'rt-tbl-utf-8 "Tbl." :coding-type 'utf-8 :mnemonic ?U :charset-list '(unicode) :encode-translation-table (make-translation-table '((?a . ?b))))"#;
    // An own table, and a standard one that is a list of char-tables.
    let cases = [
        ("(coding-system-for-write 'rt-pre-utf-8)", "PREabc", "pre-write-conversion"),
        ("(coding-system-for-write 'rt-tbl-utf-8)", "bbc", "translation table"),
        ("(coding-system-for-write 'utf-8) (standard-translation-table-for-encode (list (make-translation-table '((?a . ?b)))))", "bbc", "translation table"),
    ];
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    for (index, (coding, gnu_bytes, refusal)) in cases.into_iter().enumerate() {
        let file = dir.join(format!("case-{index}.txt"));
        let write = format!(r#"(let ({coding}) (write-region "abc" nil {:?}))"#, file.to_str().unwrap());
        std::fs::write(&file, b"old\n").unwrap();
        let output = std::process::Command::new(&binary)
            .args(["-Q", "--batch", "--eval", &format!("(progn {setup} {write})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(std::fs::read(&file).unwrap(), gnu_bytes.as_bytes(), "pinned GNU converts {coding}");
        std::fs::write(&file, b"old\n").unwrap();
        let probe = format!("(progn {setup} (list (condition-case e {write} (error (cadr e))) rt-pre-called))");
        let actual = vm.eval_source(&probe).unwrap().display().unwrap();
        assert!(actual.contains(refusal) && actual.ends_with(" nil)"), "{coding}: {actual}");
        assert_eq!(std::fs::read(&file).unwrap(), b"old\n", "{coding} left the destination intact");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// save-restriction in a narrowed unibyte buffer: the saved bounds move as
// GNU's BEGV and ZV markers do across insertions and deletions, and are
// restored on return, error and throw unless the buffer was killed.
#[test]
fn save_restriction_unibyte_matches_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let setup = r#"(defmacro rt-ub (&rest body)
  `(with-temp-buffer (set-buffer-multibyte nil) (insert "abcdef") (narrow-to-region 3 5) (goto-char 4)
     (list (condition-case e (progn ,@body) (error (list 'E e))) (point-min) (point-max) (point) (save-restriction (widen) (buffer-string)))))"#;
    let source = r#"(prin1-to-string
  (list
   (rt-ub (save-restriction (widen) (goto-char 1) (insert "B") (goto-char 4) (insert "S") (goto-char 7) (insert "E") (goto-char 9) (insert "A") (list (point-min) (point-max))))
   (rt-ub (save-restriction (narrow-to-region 4 4) (insert "in") (buffer-string)))
   (rt-ub (save-restriction (widen) (delete-region 2 4)))
   (rt-ub (save-restriction (widen) (delete-region 4 6)))
   (rt-ub (save-restriction (widen) (delete-region 1 7)))
   (rt-ub (save-restriction (widen) (narrow-to-region 1 2) (error "boom")))
   (rt-ub (catch 'out (save-restriction (widen) (goto-char 6) (insert "x") (throw 'out 'thrown))))
   (rt-ub (save-restriction (narrow-to-region 4 5) (save-restriction (widen) (goto-char 1) (insert "N")) (list (point-min) (point-max))))
   (rt-ub (save-restriction (widen) (goto-char 7) (point)))
   (rt-ub (let ((b (current-buffer))) (prog1 (save-restriction (widen) (set-buffer (get-buffer-create " rt-other")) (list 'other (point-min))) (set-buffer b))))
   (let ((b (generate-new-buffer " rt-ukill"))) (with-current-buffer b (set-buffer-multibyte nil) (insert "abc") (narrow-to-region 2 3)) (list (with-current-buffer b (save-restriction (kill-buffer b) 'done)) (buffer-live-p b)))))"#;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", setup, "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    vm.eval_source(setup).unwrap();
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, expected);
    // Converting the buffer inside the form carries the saved bounds, as
    // GNU's markers do.
    let converted = r#"(prin1-to-string (with-temp-buffer (set-buffer-multibyte nil) (insert "abc") (narrow-to-region 2 3) (list (condition-case e (save-restriction (widen) (set-buffer-multibyte t)) (error (cadr e))) (point-min) (point-max) enable-multibyte-characters)))"#;
    let output = std::process::Command::new(std::env::var("ROMACS_GNU_ORACLE").unwrap())
        .args(["-Q", "--batch", "--eval", &format!("(princ {converted})")]).output().unwrap();
    let expected = String::from_utf8(output.stdout).unwrap();
    assert_eq!(expected, "(t 2 3 t)");
    let actual = vm.eval_source(converted).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&actual).unwrap_or(actual);
    assert_eq!(actual, expected);
}

// save-excursion and buffer-substring in a unibyte buffer, which holds no
// markers: the saved point moves as GNU's left-biased excursion marker does
// and is restored on return, error and throw unless the buffer was killed;
// substrings are unibyte. GNU's coding decision runs set-auto-coding there.
#[test]
fn save_excursion_unibyte_matches_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let setup = r#"(defmacro rt-ux (&rest body)
  `(with-temp-buffer (set-buffer-multibyte nil) (insert "ab\351def") (goto-char 4)
     (list (condition-case e (progn ,@body) (error (list 'E e))) (point) (buffer-string))))"#;
    let source = r#"(prin1-to-string
  (list
   (rt-ux (save-excursion (goto-char 1) (insert "B") (point)))
   (rt-ux (save-excursion (insert "S") (point)))
   (rt-ux (save-excursion (goto-char 2) (delete-region 2 5) (point)))
   (rt-ux (save-excursion (goto-char 5) (delete-region 5 7)))
   (rt-ux (save-excursion (narrow-to-region 1 2) (goto-char 1)) (list (point-min) (point-max)))
   (rt-ux (save-excursion (goto-char 6) (error "boom")))
   (rt-ux (catch 'out (save-excursion (goto-char 6) (save-excursion (goto-char 2) (insert "n")) (throw 'out (point)))))
   (rt-ux (let ((b (current-buffer))) (save-excursion (set-buffer (get-buffer-create " rt-ux-other")) (insert "zz")) (eq b (current-buffer))))
   (rt-ux (list (buffer-substring 2 4) (multibyte-string-p (buffer-substring 2 4)) (progn (goto-char 1) (re-search-forward "\\(\351\\)d" nil t) (match-string 1))))
   (let ((b (generate-new-buffer " rt-ux-kill")) (o (get-buffer-create " rt-ux-survivor"))) (with-current-buffer b (set-buffer-multibyte nil) (insert "abc")) (list (with-current-buffer b (save-excursion (kill-buffer b) (set-buffer o) 'done)) (buffer-live-p b)))))"#;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", setup, "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    vm.eval_source(setup).unwrap();
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, expected);
}

// The runtime pieces Gnosis and emacs-hermes reach, on the full loadup chain.
#[test]
fn daily_driver_load_slice_matches_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let scratch = std::env::temp_dir().join(format!("rt-recursive-load-{}.el", std::process::id()));
    std::fs::write(&scratch, ";;; -*- lexical-binding: t -*-\n(defvar rt-n 0)\n(setq rt-n (1+ rt-n))\n(when (< rt-n 10) (load load-file-name nil t t))\n").unwrap();
    let recursive = format!("(list (condition-case e (load {:?} nil t t) (error (list (car e) (cadr e) (length (cddr e))))) rt-n)", scratch.to_str().unwrap());
    let expressions: Vec<String> = [
        r###"(prin1-to-string
 (mapcar (lambda (f) (condition-case e (funcall f) (error (list 'E e))))
  (list (lambda () (list (floor 2.5) (floor -2.5) (floor 7 2) (floor -7 2) (floor 7 -2) (floor 1.0 0.1) (floor -1.0 0.1) (floor 5) (floor 7.0 2)))
        (lambda () (floor 1 0)) (lambda () (floor 1.0 0.0)) (lambda () (floor 1.0e+INF)) (lambda () (floor 'a)) (lambda () (list (floor 1e30) (round 1e300) (ceiling -1e20) (truncate 9.3e18) (round (ash 1 62)) (floor 1e19 1.0) (round -0.0) (prin1-to-string (list 1.5e-323 5e-324 1e-310 0.0 -0.0 2.2250738585072014e-308))))
        (lambda () (list (sqrt 4) (sqrt 2) (sqrt -1) (sqrt 0) (sqrt (ash 1 80))))
        (lambda () (list (sin 0) (cos 0) (tan 1) (asin 1) (acos 2) (sin 1e22) (cos 1.5)))
        (lambda () (list (isnan 0.0e+NaN) (isnan 1.0))) (lambda () (isnan 1))
        (lambda () (list (copysign 1.0 -0.0) (copysign -2.0 3.0) (let ((x 1.5)) (eq x (copysign x 2.0))))) (lambda () (copysign 1 2.0))
        (lambda () (list (frexp 8.0) (frexp 0.0) (frexp -3) (frexp 1e-310) (frexp 1.0e+INF) (frexp 0.1)))
        (lambda () (list (ldexp 0.5 4) (ldexp 1 -1) (ldexp 1.0 2000) (ldexp 1.0 -1080) (ldexp 3.0 -1074) (ldexp 1.0 most-positive-fixnum))) (lambda () (ldexp 1.0 1.0))
        (lambda () (list (expt 2 10) (expt 2 100) (expt -2 3) (expt 0 0) (expt 0 5) (expt -1 (ash 1 70)) (expt 1 (ash 1 70)) (expt 2 -1) (expt 2.0 3) (expt 4 0.5) (expt -8 (/ 1.0 3)) (expt 3 0)))
        (lambda () (expt 2 65535)) (lambda () (expt 2 65536)) (lambda () (expt 3 100000)) (lambda () (expt 'a 2)) (lambda () (expt 2 (ash 1 70)))
        (lambda () (list (float 3) (float 2.5) (float (ash 1 70)) (let ((x 2.5)) (eq x (float x))))) (lambda () (float "x"))
        (lambda () (list (ffloor 2.5) (fceiling 2.1) (fround 2.5) (fround 3.5) (fround -2.5) (ftruncate -2.7))) (lambda () (ffloor 2)))))"###,
        r###"(with-temp-buffer
  (insert "abcdefghij")
  (put-text-property 3 5 'p 'text)
  (put-text-property 6 8 'q 'front)
  (put-text-property 6 8 'front-sticky '(q))
  (put-text-property 8 9 'r 'rear)
  (put-text-property 8 9 'rear-nonsticky t)
  (let ((o1 (make-overlay 2 4)) (o2 (make-overlay 4 6 nil t nil)) (o3 (make-overlay 7 9 nil nil t)) (o4 (make-overlay 2 4)))
    (overlay-put o1 'o 'one) (overlay-put o2 'o 'two) (overlay-put o3 'o 'three) (overlay-put o4 'o 'four) (overlay-put o4 'priority 5)
    (list (mapcar (lambda (i) (get-pos-property i 'o)) '(1 2 3 4 5 6 7 8 9 10))
          (mapcar (lambda (i) (get-pos-property i 'p)) '(1 2 3 4 5 6))
          (mapcar (lambda (i) (get-pos-property i 'q)) '(5 6 7 8 9))
          (mapcar (lambda (i) (get-pos-property i 'r)) '(8 9 10))
          (let ((text-property-default-nonsticky '((p . t)))) (mapcar (lambda (i) (get-pos-property i 'p)) '(4 5)))
          (get-pos-property (copy-marker 4) 'p)
          (condition-case e (get-pos-property 20 'p) (error e))
          (get-pos-property 1 'p "xyz")
          (get-pos-property 1 'p (propertize "xyz" 'p 7))
          (progn (narrow-to-region 4 8) (list (get-pos-property 4 'p) (get-pos-property 5 'p))))))"###,
        r###"(with-temp-buffer
  (insert "0123456789")
  (list (save-restriction (narrow-to-region 3 6) (list (point-min) (point-max) (buffer-string)))
        (list (point-min) (point-max))
        (progn (narrow-to-region 2 8)
               (save-restriction (widen) (goto-char 1) (insert "ab") (list (point-min) (point-max))))
        (list (point-min) (point-max) (buffer-string))
        (progn (widen) (save-restriction (narrow-to-region 2 4) (goto-char 3) (insert "XYZ")) (list (point-min) (point-max)))
        (let ((other (generate-new-buffer " rt"))) (with-current-buffer other (insert "abcdef") (narrow-to-region 2 4))
          (list (save-restriction (set-buffer other) (widen) (buffer-string)) (eq (current-buffer) other) (with-current-buffer other (list (point-min) (point-max)))))
        (condition-case e (save-restriction (narrow-to-region 2 3) (error "boom")) (error (list e (point-min) (point-max))))
        (with-current-buffer (generate-new-buffer " rt-unibyte") (set-buffer-multibyte nil) (insert "abc")
          (list (save-restriction (narrow-to-region 2 3) (buffer-string)) (point-min) (point-max)))
        (let ((killed (generate-new-buffer " rt-killed"))) (list (with-current-buffer killed (save-restriction (kill-buffer killed) 'done)) (buffer-live-p killed)))
        (save-restriction)
        (save-restriction 1 2 3)))"###,
        r###"(with-temp-buffer
  (insert "abcdef")
  (let* ((g (current-global-map)) (l (make-sparse-keymap "L")) (k (make-sparse-keymap "K")) (lm (make-sparse-keymap "LM"))
         (m1 (make-sparse-keymap "M1")) (m2 (make-sparse-keymap "M2")) (ov (make-sparse-keymap "OV")) (em (make-sparse-keymap "EM"))
         (name (lambda (maps) (mapcar (lambda (m) (if (eq m g) 'G (keymap-prompt m))) maps))))
    (use-local-map l)
    (put-text-property 3 4 'keymap k)
    (put-text-property 5 6 'local-map lm)
    (defvar rt-m1 nil) (defvar rt-m2 nil) (defvar rt-em nil)
    (let ((minor-mode-map-alist (list (cons 'rt-m1 m1) (cons 'rt-m2 m2)))
          (minor-mode-overriding-map-alist (list (cons 'rt-m2 ov)))
          (emulation-mode-map-alists (list (list (cons 'rt-em em)) 'rt-void-alist)))
      (list (funcall name (current-active-maps))
            (progn (setq rt-m1 t rt-m2 t rt-em t) (funcall name (current-active-maps)))
            (progn (goto-char 3) (funcall name (current-active-maps)))
            (progn (goto-char 5) (funcall name (current-active-maps)))
            (funcall name (current-active-maps nil 3))
            (let ((overriding-local-map (make-sparse-keymap "OLM"))) (list (funcall name (current-active-maps t)) (funcall name (current-active-maps nil))))
            (condition-case e (current-active-maps nil 99) (error (car e)))
            (posn-point '(nil 7)) (posn-point '(nil (8 . 9))) (posn-point '(w 1 2 3 4 10))))))"###,
        r###"(with-temp-buffer
  (let ((m (make-sparse-keymap)) (pm (make-sparse-keymap)) (o (make-sparse-keymap)) (g (make-sparse-keymap)))
    (define-key m "o" 'rt-kb-open) (set-keymap-parent m button-map) (define-key o "q" 'rt-kb-ov) (define-key pm "p" 'rt-kb-local)
    (define-key g [t] 'rt-kb-default) (define-key g [remap rt-kb-open] 'rt-kb-remapped)
    (insert "abc") (put-text-property 1 2 'keymap m) (put-text-property 2 3 'local-map pm) (overlay-put (make-overlay 3 4) 'keymap o)
    (defvar rt-kb-mode nil)
    (list (progn (goto-char 1) (list (key-binding "o") (key-binding "o" nil t) (key-binding (kbd "RET")) (key-binding [])))
          (progn (goto-char 2) (list (key-binding "o") (key-binding "p") (key-binding "o" nil nil 1) (key-binding "o" nil nil (copy-marker 1))))
          (progn (goto-char 3) (list (key-binding "q") (key-binding "o")))
          (let ((overriding-local-map g)) (list (key-binding "z") (key-binding "z" t)))
          (let ((rt-kb-mode t) (minor-mode-map-alist (list (cons 'rt-kb-mode g)))) (goto-char 3) (list (key-binding "q") (key-binding "zz" t)))
          (let ((r (make-sparse-keymap))) (define-key r [remap rt-kb-ov] 'rt-kb-remapped) (use-local-map r) (goto-char 3) (prog1 (list (key-binding "q") (key-binding "q" nil t)) (use-local-map nil)))
          (key-binding "\C-x\C-f") (key-binding [remap undo])
          (condition-case e (key-binding "o" nil nil 99) (error (car e))))))"###,
        r###"(let ((b (generate-new-buffer " rt-click")) (m (make-sparse-keymap)) (k (make-sparse-keymap)) (window (selected-window)) (old (window-buffer)))
  (define-key m [mouse-1] 'rt-click) (define-key k [(mouse-2 (nil 5))] 'rt-k2) (define-key k [(control ?x)] 'rt-cx)
  (set-window-buffer window b)
  (with-current-buffer b (insert "abc") (put-text-property 2 3 'keymap m))
  (unwind-protect
      (let ((posn (list window 2 '(0 . 0) 0 nil 2 nil nil nil nil)) (posn3 (list window 3 '(0 . 0) 0 nil 3 nil nil nil nil)))
        (list (key-binding (vector (list 'mouse-1 posn))) (key-binding [mouse-1] nil nil posn)
              (progn (put 'mouse-1 'event-kind 'mouse-click)
                     (list (key-binding (vector (list 'mouse-1 posn))) (key-binding (vector (list 'mouse-1 posn3)))
                           (key-binding (vector 'mode-line (list 'mouse-1 posn))) (key-binding (vector (list 'mouse-1 posn)) nil nil posn3)))
              (lookup-key m [(mouse-1 (nil 9))]) (lookup-key k [mouse-2]) (lookup-key k [(control ?x)]) (lookup-key k [(mode-line mouse-2)])))
    (put 'mouse-1 'event-kind nil)
    (set-window-buffer window old)
    (kill-buffer b)))"###,
        r###"(list (mapcar (lambda (a) (secure-hash a "abc")) (secure-hash-algorithms)) (mapcar (lambda (a) (secure-hash a "")) '(md5 sha1 sha224 sha384 sha3-224 sha3-256 sha3-384 sha3-512)))"###,
        r###"(list (secure-hash 'sha256 (make-string 200 ?x)) (secure-hash 'sha3-224 (make-string 144 ?y)) (secure-hash 'sha3-224 (make-string 143 ?y)) (secure-hash 'sha3-512 (make-string 72 ?z)) (secure-hash 'sha3-512 (make-string 71 ?z)) (secure-hash 'sha3-256 (make-string 1000 ?q)) (secure-hash 'sha512 (make-string 111 ?w)))"###,
        r###"(list (secure-hash 'sha1 "abcdef" 1 -1) (secure-hash 'sha1 "abcdef" -3) (secure-hash 'sha1 (string-to-unibyte "\377\0\200")) (secure-hash 'md5 (string-to-unibyte "\377") 0 1) (string-to-list (secure-hash 'md5 "abc" nil nil t)) (multibyte-string-p (secure-hash 'md5 "abc")) (multibyte-string-p (secure-hash 'md5 "abc" nil nil t)) (length (secure-hash 'sha512 "abc" nil nil t)) (md5 "abc") (md5 "abcdef" 2 4) (md5 (string-to-unibyte "\377")))"###,
        r###"(list (condition-case e (secure-hash 'nope "a") (error e)) (condition-case e (secure-hash "md5" "a") (error e)) (condition-case e (secure-hash 'md5 3) (error e)) (condition-case e (secure-hash 'md5 nil) (error e)) (condition-case e (secure-hash 'md5 '(1 2)) (error e)) (condition-case e (secure-hash 'md5 "abc" 2 9) (error e)) (condition-case e (secure-hash 'md5 "abc" 'x) (error e)) (condition-case e (secure-hash 'md5 'iv-auto) (error e)) (condition-case e (md5 "abc" nil nil 'nosuch) (error e)) (md5 "abc" nil nil 'nosuch t))"###,
        r###"(list (find-operation-coding-system 'write-region 1 2 "a.tzst") (find-operation-coding-system 'insert-file-contents "x.el")
      (find-operation-coding-system 'insert-file-contents (cons "x.elc" (current-buffer))) (find-operation-coding-system 'write-region 1 2 "plain.txt")
      (let ((file-coding-system-alist (list (cons "\\.fn\\'" 'rt-foc-pair) (cons "\\.sym\\'" 'latin-1) (cons "\\.lam\\'" (lambda (args) 'latin-1))
                                            (cons "\\.bad\\'" 3) (cons "\\.up\\'" 'utf-8) (cons "\\.ret\\'" 'rt-foc-coding) (cons "\\.no\\'" 'rt-foc-none))))
        (defalias 'rt-foc-pair (lambda (args) (cons 'got (length args)))) (defalias 'rt-foc-coding (lambda (args) 'latin-1)) (defalias 'rt-foc-none (lambda (args) 'nosuch))
        (list (find-operation-coding-system 'write-region 1 2 "a.fn") (find-operation-coding-system 'write-region 1 2 "a.sym")
              (find-operation-coding-system 'write-region 1 2 "a.bad") (find-operation-coding-system 'write-region 1 2 "A.UP")
              (find-operation-coding-system 'write-region 1 2 "a.ret") (find-operation-coding-system 'write-region 1 2 "a.no") (find-operation-coding-system 'write-region 1 2 "a.lam") (let ((case-fold-search t)) (find-operation-coding-system 'write-region 1 2 "a.UP"))))
      (let ((process-coding-system-alist '(("cat" . utf-8)))) (find-operation-coding-system 'call-process "cat"))
      (let ((network-coding-system-alist '((80 . latin-1)))) (find-operation-coding-system 'open-network-stream 1 2 3 80))
      (mapcar (lambda (args) (condition-case e (apply #'find-operation-coding-system args) (error e)))
              '(() (write-region) (car 1) ("x" 1) (write-region 1 2) (write-region 1 2 3) (insert-file-contents ("a" . 3))))
      (progn (string-match "b" "abc") (find-operation-coding-system 'write-region 1 2 "a.tzst") (match-beginning 0))
      (mapcar (lambda (s) (get s 'target-idx)) '(insert-file-contents write-region call-process start-process open-network-stream call-process-region))
      (with-temp-buffer (insert "héllo") (setq buffer-file-name "/nonexistent/rt-x.txt") (secure-hash 'sha1 (current-buffer)))
      (with-temp-buffer (insert "héllo") (setq buffer-file-name "/nonexistent/rt-x.tzst") (secure-hash 'sha1 (current-buffer)))
      (condition-case e (secure-hash (make-symbol "sha256") "abc") (error e)))"###,
        r###"(progn (defvar rt-fb nil)
  (list (recent-keys) (recent-keys t)
        (list (find-buffer 'major-mode 'rt-nonesuch-mode) (condition-case e (find-buffer 3 nil) (error e))
              (with-temp-buffer (setq-local rt-fb (list 7)) (eq (find-buffer 'rt-fb (list 7)) (current-buffer)))
              (eq (find-buffer 'rt-fb nil) (car (buffer-list))) (condition-case e (find-buffer 'rt-void 1) (error e)))
        (list buffer-saved-size (default-value 'buffer-saved-size) (local-variable-if-set-p 'buffer-saved-size)
              (with-temp-buffer (setq buffer-saved-size 5) (list buffer-saved-size (local-variable-p 'buffer-saved-size)))
              (default-value 'buffer-saved-size) (condition-case e (setq buffer-saved-size 'x) (error e)))
        (list (face-font 'default) (face-font 'bold) (face-font 'default nil ?a) (face-font 'bold t) (face-font 'italic t)
              (face-font 'default t) (face-font "bold" t) (face-font "bold") (condition-case e (face-font "nosuch") (error e)) (condition-case e (face-font 'nosuch) (error e)) (condition-case e (face-font 'default 3) (error e)))
        (list (window-pixel-top) (window-pixel-top (minibuffer-window)) (window-pixel-left)
              (progn (split-window) (window-pixel-top (next-window)))
              (progn (delete-other-windows) (split-window-right) (prog1 (window-pixel-left (next-window)) (delete-other-windows)))
              (condition-case e (window-pixel-top 3) (error e)))))"###,
        r###"(list (window-hscroll) (set-window-hscroll nil 5) (window-hscroll) (set-window-hscroll nil -3) (= (set-window-hscroll nil most-positive-fixnum) most-positive-fixnum)
      (condition-case e (set-window-hscroll nil 1.5) (error e)) (condition-case e (set-window-hscroll nil (expt 2 70)) (error (car e))) (condition-case e (window-hscroll 3) (error e))
      (progn (set-window-hscroll nil 4) (set-window-buffer nil (current-buffer)) (window-hscroll)) (progn (set-window-hscroll nil 4) (set-window-buffer nil (current-buffer) t) (window-hscroll))
      (progn (set-window-hscroll nil 4) (set-window-buffer nil (get-buffer-create "rt-h")) (window-hscroll))
      (progn (set-window-hscroll nil 7) (let ((c (current-window-configuration))) (set-window-hscroll nil 0) (set-window-configuration c) (window-hscroll)))
      (let ((w (split-window))) (prog1 (window-hscroll w) (delete-window w)))
      (eq (select-frame (selected-frame)) (selected-frame)) (eq (select-frame (selected-frame) t) (selected-frame))
      (condition-case e (select-frame nil) (error e)) (condition-case e (select-frame 3) (error e))
      (with-temp-buffer (list (recent-auto-save-p) (progn (insert "x") (set-buffer-modified-p 'autosaved) (recent-auto-save-p)) (progn (set-buffer-modified-p nil) (recent-auto-save-p))))
      (condition-case e (set-window-combination-limit (selected-window) t) (error e)) (condition-case e (set-window-combination-limit 3 t) (error e))
      (let ((w (split-window))) (prog1 (list (set-window-combination-limit (window-parent w) t) (window-combination-limit (window-parent w))) (delete-window w)))
      (minibuffer-selected-window)
      (let ((executing-kbd-macro t) (minibuffer-setup-hook (list (lambda () (setq rt-msw (list (minibuffer-selected-window) (selected-window))) (throw 'exit nil)))))
        (defvar rt-msw nil) (condition-case e (read-from-minibuffer "x") (error e)) (list (eq (car rt-msw) (frame-first-window)) (window-minibuffer-p (cadr rt-msw))))
      (minibuffer-selected-window)
      ;; A recursive minibuffer entered from another window records it; its
      ;; exit restores the outer caller.
      (let* ((enable-recursive-minibuffers t) (executing-kbd-macro t) (a (selected-window)) (b (split-window)) (depth 0) (inner nil) (result nil)
             (minibuffer-setup-hook (list (lambda () (setq depth (1+ depth))
                                            (if (= depth 1)
                                                (let ((mini (selected-window)))
                                                  (select-window b) (condition-case nil (read-from-minibuffer "inner") (error nil)) (select-window mini)
                                                  (setq result (list (eq (minibuffer-selected-window) a) (eq (minibuffer-selected-window) b))) (throw 'exit nil))
                                              (setq inner (eq (minibuffer-selected-window) b)) (throw 'exit nil))))))
        (condition-case e (read-from-minibuffer "outer") (error e)) (delete-window b) (list inner result (minibuffer-selected-window)))
      (condition-case e (set-window-combination-limit nil t) (error e)))"###,
        r###"(prin1-to-string
 (list (progn (defvar rt-eal nil) (with-eval-after-load "thingatpt" (setq rt-eal 'file)) (load "thingatpt" nil t) rt-eal)
       (progn (with-eval-after-load 'rtfeat (setq rt-eal 'provided)) (provide 'rtfeat) rt-eal)
       (let ((seen nil)) (add-hook 'after-load-functions (lambda (f) (push (file-name-sans-extension (file-name-nondirectory f)) seen)))
         (load "thingatpt" nil t) (remove-hook 'after-load-functions (car after-load-functions)) seen)
       (let ((n 0)) (with-eval-after-load 'rtfeat2 (setq n (1+ n))) (provide 'rtfeat2) (provide 'rtfeat2) n)
       (condition-case e (provide 3) (error e)) (condition-case e (provide 'x 3) (error e))
       (subr-arity (symbol-function 'provide)) (provide 'rt2 '(a b)) (get 'rt2 'subfeatures) (featurep 'rt2 'a)
       (let ((after-load-alist '((rtbad . 3)))) (condition-case e (provide 'rtbad) (error (car e))))
       ;; Hooks match the name the file was found by, through a symlinked
       ;; directory; the file is in load-history when they run; a collection
       ;; inside a hook leaves the load intact.
       (let* ((d (file-name-as-directory (progn (or (boundp 'default-directory) (setq default-directory temporary-file-directory)) (make-temp-file "rtal" t)))) (real (concat d "real/")) (alias (concat d "alias")) (log nil))
         (make-directory real) (call-process "ln" nil nil nil "-s" "real" alias)
         (write-region "(setq rt-al-x 1)\n" nil (concat real "pkg.el")) (write-region "(list 41 42 43)\n" nil (concat real "res.el"))
         (prog1 (list (progn (with-eval-after-load "alias/pkg" (push 'alias log)) (with-eval-after-load "real/pkg" (push 'real log)) (load (concat alias "/pkg.el") nil t t) log)
                      (let ((n 0)) (with-eval-after-load "real/pkg" (setq n (1+ n)) (with-eval-after-load "real/pkg" (setq n (+ n 10)))) (load (concat real "pkg.el") nil t t) n)
                      (equal (car (assoc (concat real "pkg.el") load-history)) (concat real "pkg.el"))
                      (progn (with-eval-after-load "real/res" (garbage-collect)) (load (concat real "res.el") nil t t))
                      (string-suffix-p "alias/pkg.el" (progn (load (concat alias "/pkg.el") nil t t) (caar load-history))))
           (delete-directory d t)))
       ;; `..' after a symlinked directory is resolved lexically, as the
       ;; file is named; a file stays in progress while its hooks reload it.
       (let* ((d (file-name-as-directory (make-temp-file "rtal" t))) (outer (concat d "outer/")))
         (make-directory (concat outer "real") t) (call-process "ln" nil nil nil "-s" "outer/real" (concat d "alias"))
         (write-region "(setq rt-which 'top)\n" nil (concat d "target.el")) (write-region "(setq rt-which 'outer)\n" nil (concat outer "target.el"))
         (write-region "(setq rt-rec-n (1+ rt-rec-n))\n" nil (concat d "rtrec.el")) (defvar rt-which nil) (defvar rt-rec-n 0)
         (prog1 (list (progn (load (concat d "alias/../target.el") nil t t) rt-which)
                      (progn (with-eval-after-load "rtrec" (when (< rt-rec-n 7) (load (concat d "rtrec.el") nil t t)))
                             (list (condition-case e (load (concat d "rtrec.el") nil t t) (error (car e))) rt-rec-n
                                   (length (seq-filter (lambda (e) (equal (car e) (concat d "rtrec.el"))) load-history)))))
           (delete-directory d t)))))"###,
        r###"(prin1-to-string
 (list (minibufferp) (minibufferp (window-buffer (minibuffer-window))) (minibufferp (window-buffer (minibuffer-window)) t) (minibufferp " *Minibuf-0*")
       (condition-case e (minibufferp 3) (error e)) (condition-case e (minibufferp "nosuch") (error e))
       (let ((executing-kbd-macro t) (minibuffer-setup-hook (list (lambda () (setq rt-mbp (list (minibufferp) (minibufferp nil t))) (throw 'exit nil)))))
         (defvar rt-mbp nil) (condition-case e (read-from-minibuffer "x") (error e)) rt-mbp)
       (let ((s (copy-sequence "ab"))) (put-text-property 0 1 'face 'bold s) (get-text-property 0 'face s))))"###,
        r###"(progn (defvar rt-tl 0)
  (prin1-to-string
   (with-temp-buffer
     (let ((b (current-buffer)))
       (list (condition-case e (buffer-local-toplevel-value 'rt-tl) (error e)) (progn (setq-local rt-tl 1) (buffer-local-toplevel-value 'rt-tl))
             (let ((rt-tl 2)) (list rt-tl (buffer-local-toplevel-value 'rt-tl) (set-buffer-local-toplevel-value 'rt-tl 3) rt-tl (buffer-local-toplevel-value 'rt-tl)))
             rt-tl (let ((rt-tl 9)) (with-temp-buffer (set-buffer-local-toplevel-value 'rt-tl 5 b)) rt-tl) rt-tl
             (with-temp-buffer (set-buffer-local-toplevel-value 'rt-tl 7) (list rt-tl (local-variable-p 'rt-tl)))
             (with-temp-buffer (set-buffer-local-toplevel-value 'rt-tl 8 b) (list rt-tl (buffer-local-value 'rt-tl b)))
             (progn (setq-local fill-column 40) (let ((fill-column 3)) (list (buffer-local-toplevel-value 'fill-column) (set-buffer-local-toplevel-value 'fill-column 50) fill-column)))
             fill-column (let ((fill-column 3)) (condition-case e (with-temp-buffer (buffer-local-toplevel-value 'fill-column)) (error e)))
             (condition-case e (buffer-local-toplevel-value 3) (error e)) rt-tl (default-value 'rt-tl)
             ;; A local killed and made again inside the let: the let's own
             ;; saved value is found and restored into the new local.
             (with-temp-buffer (setq-local rt-tl 1)
               (list (let ((rt-tl 2)) (kill-local-variable 'rt-tl) (setq-local rt-tl 3)
                       (list (buffer-local-toplevel-value 'rt-tl) (set-buffer-local-toplevel-value 'rt-tl 4) rt-tl))
                     rt-tl))
             (with-temp-buffer (setq-local rt-tl 1) (list (let ((rt-tl 2)) (kill-local-variable 'rt-tl) rt-tl) rt-tl (local-variable-p 'rt-tl)))
             (with-temp-buffer (setq-local rt-tl 1) (let ((rt-tl 2)) (kill-local-variable 'rt-tl) (setq-local rt-tl 5)) rt-tl)
             (let ((k (generate-new-buffer "rt-kb")) r)
               (with-current-buffer k (setq-local rt-tl 1) (let ((rt-tl 2)) (with-temp-buffer (kill-buffer k) (setq r (condition-case e (set-buffer-local-toplevel-value 'rt-tl 9 k) (error (list 'E e)))))))
               r)
             (with-temp-buffer (setq-local inhibit-modification-hooks t)
               (list (let ((inhibit-modification-hooks nil)) (set-buffer-local-toplevel-value 'inhibit-modification-hooks '(hello)) (buffer-local-toplevel-value 'inhibit-modification-hooks))
                     inhibit-modification-hooks)))))))"###,
        r###"(progn (require 'subr-x)
  (prin1-to-string
   (mapcar (lambda (f) (condition-case e (funcall f) (error e)))
           (list (lambda () (string-glyph-split "short")) (lambda () (find-composition-internal 0 1 "abc" nil))
                 (lambda () (with-temp-buffer (find-composition-internal 0 1 "e\u0301x" nil))) (lambda () (with-temp-buffer (string-glyph-split "e\u0301x👍🏽")))
                 (lambda () (with-temp-buffer (insert "e\u0301x") (find-composition-internal 1 nil nil nil)))
                 (lambda () (find-composition-internal 5 nil "abc" nil)) (lambda () (find-composition-internal -1 nil "abc" nil))
                 (lambda () (find-composition-internal "x" nil "abc" nil)) (lambda () (find-composition-internal 0 nil 3 nil))
                 (lambda () (find-composition-internal 0 "y" "abc" nil)) (lambda () (with-temp-buffer (find-composition-internal 5 nil nil nil)))
                 (lambda () (find-composition-internal (make-marker) nil "abc" nil)) (lambda () (find-composition-internal 0 2 "abc" t))))))"###,
        r###"(list (secure-hash 'sha256 "héllo ✓") (md5 "héllo") (md5 "héllo" nil nil 'latin-1) (md5 "héllo" nil nil 'nosuch t)
      (condition-case e (md5 "héllo" nil nil 'nosuch) (error e))
      (with-temp-buffer (insert "héllo\nworld")
        (list (secure-hash 'sha256 (current-buffer)) (secure-hash 'sha1 (current-buffer) 2 5) (secure-hash 'sha1 (current-buffer) 5 2)
              (progn (narrow-to-region 3 6) (list (secure-hash 'md5 (current-buffer)) (condition-case e (secure-hash 'md5 (current-buffer) 1 4) (error e))))
              (let ((coding-system-for-write 'latin-1)) (widen) (secure-hash 'md5 (current-buffer))) (md5 (current-buffer) nil nil 'utf-16le)))
      (with-temp-buffer (set-buffer-multibyte nil) (insert 200 10 65) (secure-hash 'sha1 (current-buffer)))
      (with-temp-buffer (insert "plain ascii") (secure-hash 'sha256 (current-buffer)))
      (list (find-coding-systems-region-internal "abc" nil) (find-coding-systems-region-internal "aé✓" nil) (find-coding-systems-region-internal "日本" nil)
            (find-coding-systems-region-internal "aé✓" nil '(utf-8 utf-16)) (find-coding-systems-region-internal (string 233 #x3fffe9) nil)
            (with-temp-buffer (insert "abé") (list (find-coding-systems-region-internal 1 3) (find-coding-systems-region-internal 1 4)
                                                   (progn (narrow-to-region 1 2) (length (find-coding-systems-region-internal 1 4)))
                                                   (condition-case e (find-coding-systems-region-internal 0 3) (error e)) (find-coding-systems-region-internal (copy-marker 1) 2)))
            (with-temp-buffer (set-buffer-multibyte nil) (insert 200) (find-coding-systems-region-internal 1 2))))"###,
        r###"(let* ((sub (make-sparse-keymap "Sub")) (m (make-sparse-keymap)) (calls 0))
  (define-key sub [x] 'sub-x)
  (define-key m [menu-bar foo] `(menu-item "Foo" ,sub :filter (lambda (b) (setq calls (1+ calls)) b)))
  (define-key m [menu-bar bad] '(menu-item "Bad" ignore :filter (lambda (_) (error "boom"))))
  (define-key m [menu-bar swap] `(menu-item "Swap" ignore :filter (lambda (_) 'swapped)))
  (define-key m [menu-bar both] `(menu-item "Both" ,sub :visible t :filter (lambda (b) b)))
  (list (keymapp (lookup-key m [menu-bar foo])) (lookup-key m [menu-bar foo x]) calls
        (lookup-key m [menu-bar bad]) (lookup-key m [menu-bar swap]) (lookup-key m [menu-bar both x])
        (lookup-key m [menu-bar nope]) (lookup-key m [menu-bar foo x y]) (condition-case e (lookup-key 3 [a]) (error e))
        (lookup-key (list m) [menu-bar swap])))"###,
        r###"(list (framep (selected-frame)) window-system (display-graphic-p) (display-mouse-p) (display-popup-menus-p) (display-images-p) (display-selections-p) (display-symbol-keys-p) (display-screens) (display-pixel-height) (display-pixel-width) (display-mm-height) (display-mm-width) (display-backing-store) (display-save-under) (display-planes) (display-color-cells) (display-visual-class) (display-color-p) (display-grayscale-p) (frame-parameter nil 'display-type) (frame-parameter nil 'background-mode) (tty-color-alist) (length (display-monitor-attributes-list)) (color-values "red") (color-values "#123456") (color-defined-p "#123456") (defined-colors) (color-values "chartreuse") (color-defined-p "nosuch") (color-name-to-rgb "red") (color-values-from-color-spec "rgb:f/80/abc") (color-values-from-color-spec "rgbi:1/0.5/0") (color-values-from-color-spec "#12345") (hash-table-p (frame--face-hash-table)) (condition-case e (frame--face-hash-table 3) (error e)))"###,
        r###"(list (make-char 'ascii 65) (make-char 'latin-iso8859-1 #x69) (make-char 'japanese-jisx0208 #x30 #x21) (make-char 'japanese-jisx0208) (condition-case e (make-char 'ascii 300) (error e)) (condition-case e (make-char 'japanese-jisx0208 1 1) (error e)) (make-char 'chinese-gb2312 #xb0 #xa1))"###,
        r###"(mapcar (lambda (f) (condition-case e (string-to-list (prin1-to-string (funcall f))) (error (list 'E e)))) (list (lambda () (encode-coding-string "a" 'utf-16-be)) (lambda () (encode-coding-string "a" 'utf-16be)) (lambda () (encode-coding-string "aé\U0001F600" 'utf-16le)) (lambda () (encode-coding-string "a" 'utf-16)) (lambda () (encode-coding-string "a" 'utf-16le-with-signature)) (lambda () (encode-coding-string "a\nb" 'utf-16be-dos)) (lambda () (encode-coding-string "a\nb" 'utf-16be-mac)) (lambda () (encode-coding-string (string #x3fff80 ?a) 'utf-16be)) (lambda () (progn (encode-coding-string "a" 'utf-16be) last-coding-system-used)) (lambda () (decode-coding-string "\0a" 'utf-16be)) (lambda () (decode-coding-string "a\0" 'utf-16le)) (lambda () (decode-coding-string "\376\377\0a" 'utf-16be-with-signature)) (lambda () (decode-coding-string "\0a" 'utf-16-be)) (lambda () (decode-coding-string "\330\075\336\000" 'utf-16be)) (lambda () (decode-coding-string "\330\075\0a\0b" 'utf-16be)) (lambda () (decode-coding-string "\0a\0" 'utf-16be)) (lambda () (decode-coding-string "\0a\330\075" 'utf-16be)) (lambda () (decode-coding-string "\0a\0\r\0\n\0b" 'utf-16be)) (lambda () (progn (decode-coding-string "\0a\0\r\0\n\0b" 'utf-16be) last-coding-system-used)) (lambda () (decode-coding-string "\0a\0\r\0\n" 'utf-16be-dos)) (lambda () (aref (decode-coding-string "\334\000" 'utf-16be) 0))))"###,
        r###"(list (read "?\\N{LATIN SMALL LETTER A}") (read "\"\\N{left-to-right override}x\"") (read "?\\N{LATIN  SMALL\nLETTER B}") (read "?\\N{U+1F600}") (read "?\\N{VARIATION SELECTOR-17}") (read "?\\N{CJK COMPATIBILITY IDEOGRAPH-F900}") (read "?\\N{HANGUL SYLLABLE GA}") (car (condition-case e (read "?\\N{NO SUCH NAME}") (error e))))"###,
        r###"(list (symbol-name (read "##")) (eq (read "##") (intern "")) (let ((s (read "#:foo"))) (list (symbol-name s) (intern-soft s) (eq s (intern "foo")))) (symbol-name (read "#:")) (symbol-name (read "#_foo")) (symbolp (read "#:12")) (symbol-name (read "#_1.5")) (read "#!/usr/bin/emacs --script\n(a b)") (symbol-name (read "#:a\\ b")) (gethash 'a (eval (car (read-from-string "#s(hash-table data (a 9))")))))"###,
        r###"(list (funcall (lambda (a a) a) 1 2) (funcall (lambda (_ _) 'ok) 1 2) (funcall (lambda (a &optional a) a) 1) (funcall (lambda (a &rest a) a) 1 2 3) (progn (defvar rt-base 1) (defvaralias 'rt-a1 'rt-base) (defvaralias 'rt-a2 'rt-a1) (list (indirect-variable 'rt-a2) (indirect-variable 3) (indirect-variable 'rt-other))))"###,
        r###"(list (condition-case e (condition-case err (signal 'file-missing '("a" "b")) (file-missing (signal err))) (error e)) (condition-case e (signal '(arith-error 1 2)) (arith-error (list 'caught e))) (condition-case e (signal nil 3) (error e)) (condition-case e (apply #'signal '((error "g"))) (error e)) (condition-case e (signal 'error nil) (error e)))"###,
        r###"(let* ((f (make-temp-file "rt-x")) (d (make-temp-file "rt-y" t)) (n (make-temp-file-internal (expand-file-name "rt-z" temporary-file-directory) 0 ".txt" nil))) (prog1 (list (file-regular-p f) (file-directory-p d) (file-exists-p n) (string-suffix-p ".txt" n) (length (file-name-nondirectory f)) (format "%o" (file-modes f)) (format "%o" (file-modes d)) (condition-case e (make-temp-file-internal "/nonexistent-dir/x" nil "" nil) (error (list (car e) (cadr e) (caddr e)))) (condition-case e (delete-directory (expand-file-name "rt-none-dir" temporary-file-directory)) (error (list (car e) (cadr e))))) (delete-file f) (delete-directory d)))"###,
        r###"(list (num-processors 'current) (keymap-prompt (make-sparse-keymap "Hi")) (keymap-prompt (list 'keymap (make-sparse-keymap "Inner") "Outer")) (list system-configuration movemail-program-name) (stringp user-mail-address) custom-delayed-init-variables)"###,
        r###"(let ((tt (make-translation-table '((?a . ?b) (?c . ?é) (?e . ?e))))
      (seq (make-translation-table-from-alist '(([?l ?o] . ?L) ([?q] . [?Q ?Q]) ([?d] . [?x ?y])))))
  (list (with-temp-buffer (insert (propertize "abcde" 'face 'bold)) (list (translate-region-internal 1 6 tt) (buffer-string) (point)))
        (with-temp-buffer (insert "hello quiz d") (list (translate-region-internal 1 (point-max) seq) (buffer-string)))
        (with-temp-buffer (insert "abc") (list (translate-region-internal 1 4 "zyxwvutsrqponmlkjihgfedcbaZYXWVUTSRQPONMLKJIHGFEDCBA9876543210zyxwvutsrqponmlkjihgfedcbaABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789zyxwvutsrqponmlkjihgfedcba") (buffer-string)))
        (with-temp-buffer (insert "abc") (condition-case e (translate-region-internal 1 4 (make-char-table 'foo)) (error e)))
        (with-temp-buffer (insert "abc") (condition-case e (translate-region-internal 1 9 tt) (error (car e))))
        (with-temp-buffer (insert "aaaa") (let ((m (copy-marker 3))) (translate-region-internal 1 5 tt) (list (buffer-string) (marker-position m))))
        (progn (require 'ucs-normalize) (list (ucs-normalize-NFD-string "é") (ucs-normalize-NFC-string "é") (ucs-normalize-NFKC-string "ﬁ")))))"###,
        r###"(list (read "18446744073709551615") (read "-18446744073709551616") (read "+123456789012345678901234567890") (read "2305843009213693952") (read "123.") (bignump (read "4611686018427387904")))"###,
        r###"(let ((vb (make-translation-table-from-alist '((?b . [?B]))))
      (ce (make-translation-table '((?b . ?é))))
      (seq (make-translation-table-from-alist '(([?b ?c] . [?X ?Y ?Z]))))
      (bad (make-translation-table-from-alist '((?b . [?B nil])))))
  (list (with-temp-buffer (insert (propertize "abc" 'face 'bold)) (translate-region-internal 1 4 vb) (buffer-string))
        (with-temp-buffer (insert "abc") (goto-char 2) (let ((m (copy-marker 3)) (m2 (copy-marker 2))) (translate-region-internal 1 4 ce) (list (buffer-string) (point) (marker-position m) (marker-position m2))))
        (with-temp-buffer (insert "abcd") (goto-char 3) (let ((m1 (copy-marker 2)) (m2 (copy-marker 3)) (m3 (copy-marker 4)) (m4 (copy-marker 5))) (list (translate-region-internal 1 5 seq) (buffer-string) (point) (mapcar #'marker-position (list m1 m2 m3 m4)))))
        (with-temp-buffer (insert "abc") (goto-char 4) (translate-region-internal 1 4 seq) (list (buffer-string) (point)))
        (with-temp-buffer (insert "abc") (condition-case e (translate-region-internal 1 4 bad) (error e)))
        (with-temp-buffer (insert "abc") (set-buffer-modified-p nil) (list (translate-region-internal 1 4 (make-translation-table '((?z . ?y)))) (buffer-modified-p)))
        (with-temp-buffer (set-buffer-modified-p nil) (list (translate-region-internal 1 1 vb) (buffer-modified-p)))))"###,
        r###"(let ((up (make-translation-table '((?b . ?B)))))
  (list (with-temp-buffer (insert "abc") (put-text-property 2 3 'read-only t) (set-buffer-modified-p nil) (list (condition-case e (translate-region-internal 1 4 up) (error e)) (buffer-string) (buffer-modified-p)))
        (with-temp-buffer (insert "abc") (put-text-property 1 4 'inhibit-read-only t) (setq buffer-read-only t) (list (condition-case e (translate-region-internal 1 4 up) (error e)) (buffer-string)))
        (with-temp-buffer (insert "abc") (setq buffer-read-only t) (list (condition-case e (translate-region-internal 1 4 up) (error (car e))) (buffer-string)))
        (with-temp-buffer (insert "abc") (put-text-property 2 3 'read-only 'rt-tag) (list (let ((inhibit-read-only '(rt-tag))) (translate-region-internal 1 4 up)) (buffer-string)))
        (with-temp-buffer (insert "abc") (put-text-property 2 3 'read-only "nope") (condition-case e (translate-region-internal 1 4 up) (error e)))
        (with-temp-buffer (insert "abc") (let ((after-change-functions (list #'ignore))) (list (translate-region-internal 1 4 up) (buffer-string))))))"###,
        r###"(let ((tt (make-char-table 'translation-table)))
  (list (with-temp-buffer (insert "abc") (put-text-property 1 4 'read-only t) (set-buffer-modified-p nil) (list (condition-case e (translate-region-internal 2 2 tt) (error e)) (buffer-modified-p)))
        (with-temp-buffer (insert "abc") (put-text-property 1 2 'read-only t) (set-buffer-modified-p nil) (list (condition-case e (translate-region-internal 2 2 tt) (error e)) (buffer-modified-p)))
        (with-temp-buffer (insert "abc") (put-text-property 2 3 'read-only t) (set-buffer-modified-p nil) (list (condition-case e (translate-region-internal 2 2 tt) (error e)) (buffer-modified-p)))
        (with-temp-buffer (insert "abc") (put-text-property 1 4 'read-only t) (let ((inhibit-read-only t)) (translate-region-internal 2 2 tt)))
        (with-temp-buffer (insert "abc") (put-text-property 1 4 'read-only t) (setq buffer-read-only t) (set-buffer-modified-p nil) (list (condition-case e (translate-region-internal 2 2 tt) (error (car e))) (buffer-modified-p)))))"###,
        // A buffer-local window-scroll-functions runs in its window's buffer.
        r###"(let ((b (get-buffer-create "wsf-target")) (log nil)) (set-window-buffer (selected-window) b) (with-current-buffer b (add-hook 'window-scroll-functions (lambda (_w _s) (push (buffer-name) log)) nil t)) (with-temp-buffer (run-window-scroll-functions)) (list log (length (buffer-local-value 'window-scroll-functions b)) (default-value 'window-scroll-functions)))"###,
        // special-form-p (subr.el) over the special forms' subrs.
        r###"(mapcar #'special-form-p '(and catch cond condition-case defconst defvar function if inline interactive let let* or prog1 progn quote save-current-buffer save-excursion save-restriction setq unwind-protect while when car))"###,
        // Window configurations, set-window-buffer and buffer histories.
        r###"(let ((a (get-buffer-create "wc-a")) (b (get-buffer-create "wc-b")))
  (with-current-buffer a (erase-buffer) (insert "aaaa\naaaa\naaaa"))
  (with-current-buffer b (erase-buffer) (insert "bbbbbbbb"))
  (switch-to-buffer a) (delete-other-windows) (goto-char 3)
  (list
   (let ((c (current-window-configuration)))
     (list (window-configuration-p c) (type-of c) (prin1-to-string c) (eq (window-configuration-frame c) (selected-frame))
           (progn (split-window) (length (window-list)))
           (progn (switch-to-buffer b) (buffer-name (window-buffer)))
           (set-window-configuration c)
           (length (window-list)) (buffer-name (window-buffer)) (buffer-name) (point)))
   (let ((c (current-window-configuration)))
     (goto-char 7)
     (set-window-configuration c)
     (point))
   (let* ((w (split-window)) (c (current-window-configuration)))
     (delete-window w)
     (list (window-live-p w) (progn (set-window-configuration c) (window-live-p w)) (length (window-list)) (eq (selected-window) (frame-first-window))
           (progn (delete-other-windows) (length (window-list)))))
   (progn (save-window-excursion (switch-to-buffer b) (split-window) (goto-char 5)) (list (buffer-name) (length (window-list)) (point) (with-current-buffer b (point))))
   (let* ((w2 (split-window)) (c nil))
     (set-window-buffer w2 b) (set-window-point w2 4)
     (setq c (current-window-configuration))
     (set-window-point w2 2) (set-window-buffer w2 a)
     (set-window-configuration c)
     (prog1 (list (buffer-name (window-buffer w2)) (window-point w2)) (delete-other-windows)))
   (condition-case e (set-window-configuration 1) (error e))
   (condition-case e (window-configuration-frame 'x) (error e))
   (window-configuration-p 1)))"###,
        r###"(let ((a (get-buffer-create "swb-a")) (b (get-buffer-create "swb-b")) (log nil))
  (delete-other-windows) (set-window-buffer nil a) (set-window-prev-buffers nil nil)
  (list
   (progn (set-window-buffer nil b) (mapcar (lambda (e) (buffer-name (car e))) (window-prev-buffers)))
   (progn (set-window-buffer nil b) (length (window-prev-buffers)))
   (progn (switch-to-buffer a) (mapcar (lambda (e) (buffer-name (car e))) (window-prev-buffers)))
   (let ((window-scroll-functions (list (lambda (w s) (push (list (buffer-name) s) log))))) (set-window-buffer nil b) log)
   (condition-case e (set-window-buffer nil "no-such-buffer-x") (error e))
   (condition-case e (set-window-buffer nil 7) (error e))
   (condition-case e (set-window-buffer 7 a) (error e))
   (condition-case e (let ((k (get-buffer-create "swb-k"))) (kill-buffer k) (set-window-buffer nil k)) (error e))
   (set-window-buffer nil "swb-a")
   (buffer-name (window-buffer))))"###,
        r###"(let ((a (get-buffer-create "wc3-a")) (b (get-buffer-create "wc3-b")))
  (delete-other-windows) (set-window-buffer nil a) (set-window-prev-buffers nil nil)
  (let ((c (current-window-configuration)))
    (set-window-buffer nil b) (set-window-prev-buffers nil nil)
    (set-window-configuration c)
    (list (buffer-name (window-buffer)) (mapcar (lambda (e) (buffer-name (car e))) (window-prev-buffers)))))"###,
        r###"(let ((a (get-buffer-create "wc-a")) (k (get-buffer-create "wc-k")))
  (with-current-buffer a (erase-buffer) (dotimes (i 50) (insert (format "line %d\n" i))))
  (switch-to-buffer a) (delete-other-windows)
  (list
   (let* ((w2 (split-window)) c)
     (set-window-buffer w2 k)
     (setq c (current-window-configuration))
     (kill-buffer k)
     (set-window-configuration c)
     (prog1 (list (length (window-list)) (buffer-live-p (window-buffer w2)) (eq (window-buffer w2) k)) (delete-other-windows)))
   (let (c)
     (set-window-parameter nil 'clone-of 'x)
     (set-window-parameter nil 'my-param 'y)
     (setq c (current-window-configuration))
     (set-window-parameter nil 'clone-of 'z)
     (set-window-parameter nil 'my-param 'w)
     (set-window-configuration c)
     (list (window-parameter nil 'clone-of) (window-parameter nil 'my-param)))
   (let* ((w2 (split-window)) c)
     (set-window-buffer w2 a) (set-window-start w2 31)
     (setq c (current-window-configuration))
     (set-window-start w2 1)
     (set-window-configuration c)
     (prog1 (window-start w2) (delete-other-windows)))
   (condition-case e (current-window-configuration 'x) (error e))
   (save-window-excursion (split-window) (save-window-excursion (split-window) (delete-other-windows)) (length (window-list)))
   (length (window-list))
   (list (set-window-next-buffers nil '(x)) (window-next-buffers) (set-window-next-buffers nil nil) (condition-case e (window-prev-buffers 1) (error e)))))"###,
        // set-window-point coercion and window-point-insertion-type.
        r###"(let ((a (get-buffer-create "wc4-a")) (b (get-buffer-create "wc4-b")) (e (get-buffer-create "wc4-empty")))
  (with-current-buffer a (erase-buffer) (insert "abcdef"))
  (with-current-buffer e (erase-buffer))
  (delete-other-windows) (set-window-buffer nil a)
  (list
   (boundp 'window-point-insertion-type)
   (progn (set-window-buffer nil e) (with-current-buffer b (set-window-point nil 42)))
   (progn (set-window-buffer nil a)
          (with-current-buffer a (setq-local window-point-insertion-type t))
          (with-current-buffer b (set-window-point nil 3))
          (let ((c (current-window-configuration)))
            (with-current-buffer a (goto-char 3) (insert "X"))
            (set-window-configuration c)
            (window-point)))
   (progn (with-current-buffer a (kill-local-variable 'window-point-insertion-type) (erase-buffer) (insert "abcdef"))
          (set-window-buffer nil a)
          (with-current-buffer b (set-window-point nil 3))
          (let ((c (current-window-configuration)))
            (with-current-buffer a (goto-char 3) (insert "X"))
            (set-window-configuration c)
            (window-point)))))"###,
    ].iter().map(|s| s.to_string()).chain([recursive]).collect();
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let mut failures = Vec::new();
    for expression in &expressions {
        // As character codes: the printed text may hold raw bytes.
        let source = format!("(string-to-list (prin1-to-string (condition-case err {expression} (error (list 'signal err)))))");
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = String::from_utf8(output.stdout).unwrap();
        let actual = vm.eval_source(&source).unwrap().display().unwrap();
        if actual != expected {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {}", text(&expected), text(&actual)));
        }
    }
    let _ = std::fs::remove_file(&scratch);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// insdel.c and undo.c's records under GNU's own undo (simple.el): typed,
// deleted and killed text undoes to GNU's states, and the command loop's
// boundaries and amalgamation give GNU's buffer-undo-list.
#[test]
fn edit_undo_matches_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let expressions = [
        r###"(with-temp-buffer (buffer-enable-undo) (insert "hello") (undo-boundary) (insert " world") (undo-boundary) (goto-char 3) (delete-char 2) (undo-boundary) (put-text-property 1 3 'face 'bold) (undo-boundary) (let ((states (list (buffer-string))) (last-command nil)) (undo) (setq states (cons (buffer-string) states)) (let ((last-command 'undo)) (undo) (setq states (cons (buffer-string) states)) (undo) (setq states (cons (buffer-string) states)) (undo) (setq states (cons (buffer-string) states))) (list (nreverse states) (point) (buffer-modified-p))))"###,
        r###"(with-temp-buffer (buffer-enable-undo) (insert "abcdef") (undo-boundary) (let ((m (copy-marker 3)) (n (copy-marker 4 t))) (delete-region 2 5) (undo-boundary) (primitive-undo 1 (cdr buffer-undo-list)) (list (buffer-string) (marker-position m) (marker-position n) (point))))"###,
        r###"(with-temp-buffer (buffer-enable-undo) (insert "one two") (undo-boundary) (erase-buffer) (undo-boundary) (insert-before-markers "Z") (undo-boundary) (let ((last-command nil)) (undo) (let ((last-command 'undo)) (undo)) (list (buffer-string) (point))))"###,
        r###"(with-temp-buffer (buffer-enable-undo) (insert "aXbXc") (undo-boundary) (subst-char-in-region 1 6 ?X ?Y) (list (buffer-string) buffer-undo-list (progn (undo-boundary) (primitive-undo 1 (cdr buffer-undo-list)) (buffer-string))))"###,
        r###"(with-temp-buffer (buffer-enable-undo) (let ((log nil)) (add-hook 'after-change-functions (lambda (b e l) (push (list b e l) log)) nil t) (insert "abc") (undo-boundary) (delete-region 1 3) (undo-boundary) (primitive-undo 1 (cdr buffer-undo-list)) (list (buffer-string) (nreverse log) buffer-undo-list)))"###,
        r###"(with-temp-buffer (buffer-enable-undo) (dotimes (i 5) (insert "x")) (indent-to 12) (move-to-column 3 t) (insert "\t") (move-to-column 10 t) (undo-boundary) (list (buffer-string) buffer-undo-list (progn (primitive-undo 1 (cdr buffer-undo-list)) (buffer-string))))"###,
        r###"(with-temp-buffer (buffer-enable-undo) (let ((log nil)) (add-hook 'before-change-functions (lambda (b e) (push (list 'before b e) log)) nil t) (insert "abc") (kill-region 1 3) (yank) (list (buffer-string) (car kill-ring) (nreverse log) buffer-undo-list)))"###,
        r###"(with-temp-buffer (buffer-enable-undo) (insert "ab") (setq buffer-undo-list t) (insert "cd") (list buffer-undo-list (progn (buffer-enable-undo) (insert "e") buffer-undo-list)))"###,
        r###"(with-temp-buffer (buffer-enable-undo) (insert "abcdef") (undo-boundary) (setq buffer-undo-list nil) (let ((log nil)) (add-hook 'before-change-functions (lambda (b e) (narrow-to-region 4 6)) nil t) (list (condition-case e (delete-region 1 3) (error e)) buffer-undo-list (progn (setq before-change-functions nil) (widen) (primitive-undo 1 buffer-undo-list) (buffer-string)))))"###,
    ];
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let gnu = |source: &str| {
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", source]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        String::from_utf8(output.stdout).unwrap()
    };
    let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
        .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
    let mut failures = Vec::new();
    for expression in expressions {
        let source = format!("(string-to-list (prin1-to-string (condition-case err {expression} (error (list 'signal err)))))");
        let expected = gnu(&format!("(prin1 {source})"));
        let actual = vm.eval_source(&source).unwrap().display().unwrap();
        if actual != expected {
            failures.push(format!("{expression}\n  GNU    {}\n  Romacs {}", text(&expected), text(&actual)));
        }
    }
    // Typing in the command loop: GNU's keys as one keyboard macro, each
    // command's state taken by post-command-hook (command_loop_1 also runs
    // it once on entry, hence the last nine); Romacs's keys through its
    // command boundary. The states print at the end, as the undo entries
    // they share keep growing.
    let state = "(setq rt-states (cons (list (buffer-string) (point) buffer-undo-list) rt-states))";
    let keys: [i64; 9] = [97, 98, 99, 127, 100, 2, 101, 67108911, 67108911];
    let script = format!("(progn (switch-to-buffer (get-buffer-create \"typing\")) (buffer-enable-undo) (defvar rt-states nil) (add-hook 'post-command-hook (lambda () {state})) (execute-kbd-macro (kbd \"a b c DEL d C-b e C-/ C-/\")) (prin1 (string-to-list (prin1-to-string (last (nreverse rt-states) 9)))))");
    let expected = gnu(&script);
    vm.eval_source("(progn (switch-to-buffer (get-buffer-create \"typing\")) (buffer-enable-undo) (defvar rt-states nil))").unwrap();
    for code in keys {
        let outcome = vm.command_event(romacs_vm::CommandEvent::Character(code)).unwrap();
        assert!(outcome.signal.is_none(), "{code}: {:?}", outcome.signal);
        vm.eval_source(state).unwrap();
    }
    let actual = vm.eval_source("(string-to-list (prin1-to-string (nreverse rt-states)))").unwrap().display().unwrap();
    if actual != expected {
        failures.push(format!("typing\n  GNU    {}\n  Romacs {}", text(&expected), text(&actual)));
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// process.c wait_reading_process_output runs keyboard.c's timer_check: GNU's
// timer.el timers fire inside accept-process-output and sleep-for, and
// with-timeout can end a wait for a process that never answers.
#[test]
fn process_waits_run_timers_like_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let source = r###"(prin1-to-string
 (progn (setq default-directory (file-name-as-directory (getenv "TMPDIR")))
 (list (let ((x 0)) (run-at-time 0.02 nil (lambda () (setq x 1))) (while (= x 0) (accept-process-output nil 0.01)) x)
       (let ((n 0)) (let ((tm (run-at-time 0 0.01 (lambda () (setq n (1+ n)))))) (while (< n 3) (sleep-for 0.01)) (cancel-timer tm)) (list (>= n 3) timer-list))
       (let ((p (make-process :name "never" :command (list "sleep" "10") :connection-type 'pipe :noquery t)))
         (prog1 (with-timeout (0.05 'timed-out) (accept-process-output p) 'answered) (delete-process p)))
       (let ((y nil)) (run-at-time 0 nil (lambda () (setq y (current-buffer)))) (with-temp-buffer (accept-process-output nil 0.02)) (bufferp y))
       (let ((log nil)) (run-at-time 0 nil (lambda () (error "timer boom"))) (accept-process-output nil 0.02) timer-list))))"###;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, expected);
}

// keyboard.c timer_check and timer_check_2 through timer.el's
// timer-event-handler: ripe timers run in due order within each wait, a timer
// may wait in turn (its sit-for runs the timers due meanwhile), a timer that
// reactivates itself is checked again, and a wait inside Lisp is not idle, so
// idle timers wait and `current-idle-time' stays nil. Pinned GNU's answers.
#[test]
fn timer_order_in_waits_matches_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let source = r###"(prin1-to-string
 (list (let ((log nil))
         (run-at-time 0.03 nil (lambda () (push 'c log)))
         (run-at-time 0.01 nil (lambda () (push 'a log)))
         (run-at-time 0.02 nil (lambda () (push 'b log)))
         (sleep-for 0.06)
         (nreverse log))
       (let ((log nil))
         (run-at-time 0 nil (lambda () (push 'outer log) (sit-for 0.04) (push 'outer-done log)))
         (run-at-time 0.01 nil (lambda () (push 'inner log)))
         (accept-process-output nil 0.08)
         (nreverse log))
       (let ((n 0) (tm (timer-create)))
         (timer-set-time tm nil)
         (timer-set-function tm (lambda () (setq n (1+ n)) (if (< n 3) (timer-activate tm))))
         (timer-activate tm)
         (accept-process-output nil 0)
         (prog1 n (cancel-timer tm)))
       (let ((log nil))
         (let ((idle (run-with-idle-timer 0 nil (lambda () (push 'idle log)))))
           (run-at-time 0 nil (lambda () (push (list 'ordinary (current-idle-time)) log)))
           (accept-process-output nil 0.03)
           (sit-for 0.02)
           (cancel-timer idle))
         (list (nreverse log) (current-idle-time)))))"###;
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(binary).args(["-Q", "--batch", "--eval", &format!("(princ {source})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let printed = vm.eval_source(source).unwrap().display().unwrap();
    let actual: String = serde_json::from_str(&printed).unwrap_or(printed);
    assert_eq!(actual, expected);
}

// fileio.c Fcopy_file, Fadd_name_to_file, Fmake_symbolic_link and
// Fset_file_times, dired.c Ffile_name_all_completions, and the files.el
// callers that reach them, on the loadup chain. Each case runs in DIR, a
// fresh copy of one fixture; its printed value and DIR afterwards (names,
// kinds, link targets, contents, modes, link counts and any time not
// "now") must be pinned GNU's.
#[test]
fn file_names_links_and_copies_match_gnu() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("rt-file-links-{}", std::process::id()));
    const OLD: i64 = 1_000_000_000;
    let fixture = || {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let times = std::fs::FileTimes::new()
            .set_accessed(std::time::UNIX_EPOCH + std::time::Duration::new(OLD as u64, 500))
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::new(OLD as u64, 500));
        for (name, text, mode) in [("a", "hello", 0o640), ("sub/inner", "inner\n", 0o600), ("Big.TXT", "", 0o644), ("x.elc", "", 0o644)] {
            std::fs::write(dir.join(name), text).unwrap();
            std::fs::set_permissions(dir.join(name), std::fs::Permissions::from_mode(mode)).unwrap();
            std::fs::File::options().write(true).open(dir.join(name)).unwrap().set_times(times).unwrap();
        }
        for (target, name) in [("a", "ln"), ("sub", "dl"), ("absent", "dangling")] {
            std::os::unix::fs::symlink(target, dir.join(name)).unwrap();
        }
    };
    fn snapshot(base: &Path, at: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(at).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let time = |seconds: i64, nanos: i64| if seconds < 1_500_000_000 { format!("{seconds}.{nanos}") } else { "now".into() };
            let times = format!("m{} a{}", time(meta.mtime(), meta.mtime_nsec()), time(meta.atime(), meta.atime_nsec()));
            let name = path.strip_prefix(base).unwrap().display().to_string();
            if meta.file_type().is_symlink() {
                out.push(format!("{name} -> {} {times}", std::fs::read_link(&path).unwrap().display()));
            } else if meta.is_dir() {
                out.push(format!("{name}/ {:o}", meta.mode() & 0o7777));
                snapshot(base, &path, out);
            } else {
                out.push(format!("{name} {:o} n{} {times} {:?}", meta.mode() & 0o7777, meta.nlink(), std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let cases = [
        // copy-file
        r#"(copy-file "a" "b")"#,
        r#"(copy-file "a" "ln")"#,
        r#"(copy-file "a" "a" t)"#,
        r#"(copy-file "absent" "b")"#,
        r#"(copy-file "sub" "b")"#,
        r#"(copy-file "a" "sub/")"#,
        // FILE's last component is expanded under a directory NEWNAME: `~'
        // names the home directory, which exists (nothing is written).
        r#"(progn (copy-file "a" "./~") (copy-file "./~" "sub/"))"#,
        r#"(copy-file "a" "sub" t)"#,
        r#"(copy-file "a" "nodir/b")"#,
        r#"(copy-file "a" "dangling" t)"#,
        r#"(copy-file "ln" "b" nil t)"#,
        r#"(copy-file "a" "b" 0)"#,
        r#"(list (copy-file "a" "b") (copy-file "sub/inner" "b" t) (file-modes "b"))"#,
        r#"(progn (set-file-modes "a" #o751) (copy-file "a" "b" nil nil t))"#,
        r#"(progn (set-file-modes "a" #o751) (copy-file "a" "b" nil nil nil t))"#,
        r#"(list (copy-file "a" "b") (copy-file "a" "b" 'yes t nil t) (file-modes "b"))"#,
        r#"(copy-file 1 "b")"#,
        r#"(copy-file "a" 1)"#,
        "(copy-file \"a\\0\" \"b\")",
        // add-name-to-file
        r#"(list (add-name-to-file "a" "h") (file-attribute-link-number (file-attributes "a")))"#,
        r#"(add-name-to-file "a" "ln")"#,
        r#"(add-name-to-file "sub/inner" "a" t)"#,
        r#"(add-name-to-file "a" "sub" t)"#,
        r#"(add-name-to-file "absent" "h")"#,
        r#"(add-name-to-file "a" "sub/")"#,
        r#"(add-name-to-file "ln" "h")"#,
        // make-symbolic-link
        r#"(list (make-symbolic-link "absent" "l2") (file-symlink-p "l2"))"#,
        r#"(list (make-symbolic-link "/:/x" "l2") (make-symbolic-link "~/foo" "l3"))"#,
        r#"(list (make-symbolic-link "/:/x" "l2" 0) (make-symbolic-link "~/foo" "l3" 0) (make-symbolic-link "x/a" "sub/"))"#,
        r#"(make-symbolic-link "a" "ln")"#,
        r#"(make-symbolic-link "zz" "ln" t)"#,
        r#"(make-symbolic-link "zz" "sub" t)"#,
        r#"(make-symbolic-link "x/.." "sub/")"#,
        r#"(make-symbolic-link "x/" "sub/")"#,
        r#"(make-symbolic-link "a" "nodir/l")"#,
        "(make-symbolic-link \"a\\0b\" \"l2\")",
        r#"(make-symbolic-link 1 "l2")"#,
        // set-file-times
        r#"(list (set-file-times "a" '(1234 . 1)) (file-attribute-modification-time (file-attributes "a")) (file-attribute-access-time (file-attributes "a")))"#,
        r#"(list (set-file-times "a" 1234.25) (set-file-times "sub/inner" '(0 100 5 7)) (set-file-times "x.elc" -1.5))"#,
        r#"(list (set-file-times "ln" 777 'nofollow) (set-file-times "dl" 778))"#,
        r#"(list (set-file-times "a") (< (abs (float-time (time-subtract (file-attribute-modification-time (file-attributes "a")) nil))) 60))"#,
        r#"(set-file-times "absent" 0)"#,
        r#"(set-file-times "absent" 'foo)"#,
        r#"(set-file-times 1 0)"#,
        // file-name-all-completions
        r#"(sort (file-name-all-completions "" default-directory) #'string<)"#,
        // Regexps match with the standard canonical table, not the buffer's.
        r#"(with-temp-buffer (let ((tbl (copy-case-table (standard-case-table)))) (set-case-syntax-pair ?X ?b tbl) (set-case-table tbl) (let ((completion-ignore-case t) (completion-regexp-list (list "X"))) (sort (file-name-all-completions "" default-directory) #'string<))))"#,
        r#"(list (file-name-all-completions "s" default-directory) (file-name-all-completions "zz" default-directory) (sort (file-name-all-completions "." "sub") #'string<))"#,
        r#"(let ((completion-ignore-case t)) (sort (file-name-all-completions "b" default-directory) #'string<))"#,
        r#"(let ((completion-ignore-case nil)) (file-name-all-completions "b" default-directory))"#,
        r#"(let ((completion-regexp-list '("^[a-d]" "l"))) (sort (file-name-all-completions "" default-directory) #'string<))"#,
        r#"(let ((completion-regexp-list '("\\("))) (file-name-all-completions "" default-directory))"#,
        r#"(file-name-all-completions "" "absent/")"#,
        r#"(file-name-all-completions "" "a")"#,
        r#"(list (condition-case e (file-name-all-completions 1 "sub") (error e)) (file-name-all-completions "" 1))"#,
        // Handlers receive the expanded names and every argument.
        r#"(progn (defalias 'rt-fh (lambda (op &rest args) (list op args))) (put 'rt-fh 'operations '(copy-file add-name-to-file make-symbolic-link set-file-times file-name-all-completions)) (let ((file-name-handler-alist '(("\\`/rt-fh:" . rt-fh)))) (list (copy-file "/rt-fh:x" "b" 1 2 3 4) (copy-file "a" "/rt-fh:y/") (add-name-to-file "/rt-fh:x" "h" 7) (make-symbolic-link "t" "/rt-fh:z" 7) (set-file-times "/rt-fh:x" 5 'nofollow) (file-name-all-completions "q" "/rt-fh:d/") (file-name-all-completions "/rt-fh:q" "sub"))))"#,
        // Existing names, by other save-side primitives.
        r#"(list (condition-case e (make-directory-internal "sub") (error e)) (condition-case e (rename-file "a" "ln") (error e)))"#,
        // files.el's directory walk. Names are made relative by hand:
        // `file-relative-name' and `copy-directory' (through `file-truename')
        // probe every ancestor's case sensitivity, which VM2 refuses when a
        // shared ancestor changes during the probe.
        r#"(mapcar (lambda (f) (substring f (length default-directory))) (directory-files-recursively default-directory "" t))"#,
        r#"(mapcar (lambda (f) (substring f (length default-directory))) (directory-files-recursively default-directory "e" nil nil t))"#,
    ];
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let directory = format!("{}/", dir.display());
    let mut failures = Vec::new();
    for expression in cases {
        let source = format!("(progn (setq default-directory {directory:?}) (string-to-list (prin1-to-string (condition-case err {expression} (error (list 'signal err))))))");
        fixture();
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let mut expected = (String::from_utf8(output.stdout).unwrap(), vec![]);
        snapshot(&dir, &dir, &mut expected.1);
        fixture();
        let mut actual = (vm.eval_source(&source).unwrap().display().unwrap(), vec![]);
        snapshot(&dir, &dir, &mut actual.1);
        if actual != expected {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {} {:?}\n  Romacs {} {:?}", text(&expected.0), expected.1, text(&actual.0), actual.1));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// fileio.c's DEFUNs that Romacs lacked, each against pinned GNU (built
// with libacl, without libselinux), in a fixture DIR whose files, links
// and modes afterwards are compared too: ACLs (a plain file, a directory,
// one with named entries made by setfacl, and an unreadable path),
// SELinux contexts, `access-file', `file-newer-than-file-p',
// `directory-files-and-attributes', the stream and sync DEFUNs, the
// auto-save state, `do-auto-save' and the backup `save-buffer' makes.
#[test]
fn fileio_defuns_match_gnu() {
    use std::os::unix::fs::PermissionsExt;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("fileio-defuns-{}", std::process::id()));
    let clear = || {
        if dir.join("locked").exists() { std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0o755)).unwrap(); }
        let _ = std::fs::remove_dir_all(&dir);
    };
    let fixture = || {
        clear();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::create_dir_all(dir.join("locked")).unwrap();
        for (name, text) in [("a", "hello\n"), ("b", "bee\n"), ("ext", "ext\n"), ("sub/inner", "inner\n"), ("locked/x", "x\n")] {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let at = |name: &str, seconds: u64| std::fs::File::options().write(true).open(dir.join(name)).unwrap()
            .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds)).unwrap();
        at("a", 2_000_000_000);
        at("b", 1_000_000_000);
        let status = std::process::Command::new("setfacl").args(["-m", "u:root:rwx,g:wheel:r-x,m::r--"]).arg(dir.join("ext")).status().unwrap();
        assert!(status.success(), "setfacl");
        std::fs::set_permissions(dir.join("locked"), std::fs::Permissions::from_mode(0)).unwrap();
    };
    fn snapshot(base: &Path, at: &Path, out: &mut Vec<String>) {
        use std::os::unix::fs::PermissionsExt;
        let Ok(entries) = std::fs::read_dir(at) else { return };
        let mut entries: Vec<_> = entries.map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            let name = path.strip_prefix(base).unwrap().display().to_string();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            let mode = meta.permissions().mode() & 0o7777;
            if meta.file_type().is_symlink() {
                out.push(format!("{name} -> {}", std::fs::read_link(&path).unwrap().display()));
            } else if meta.is_dir() {
                out.push(format!("{name}/ {mode:o}"));
                snapshot(base, &path, out);
            } else {
                out.push(format!("{name} {mode:o} {:?}", std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let cases = [
        r#"(list (file-acl "a") (file-acl "sub") (file-acl "sub/") (file-acl "missing") (file-acl "ext") (condition-case e (file-acl "locked/x") (error e)))"#,
        r#"(list (set-file-acl "a" "user::rw-,group::r--,other::---") (file-modes "a") (set-file-acl "b" (file-acl "ext")) (file-acl "b") (set-file-acl "a" "bogus") (set-file-acl "a" 7) (condition-case e (set-file-acl "missing" "user::rw-\ngroup::r--\nother::r--\n") (error e)) (condition-case e (set-file-acl "locked/x" "user::rw-\ngroup::r--\nother::r--\n") (error e)))"#,
        r#"(list (file-selinux-context "a") (file-selinux-context "missing") (file-selinux-context "sub/") (set-file-selinux-context "a" '("u" "r" "t" "s")))"#,
        r#"(list (access-file "a" "Reading") (condition-case e (access-file "missing" "Opening input file") (error e)) (condition-case e (access-file "locked/x" "Reading") (error e)) (condition-case e (access-file 'x "y") (error e)))"#,
        r#"(list (file-newer-than-file-p "a" "b") (file-newer-than-file-p "b" "a") (file-newer-than-file-p "missing" "a") (file-newer-than-file-p "a" "missing") (file-newer-than-file-p "sub/" "a") (condition-case e (file-newer-than-file-p "a" 7) (error e)))"#,
        r#"(list (mapcar (lambda (e) (list (car e) (file-attribute-type (cdr e)) (file-attribute-modes (cdr e)) (stringp (file-attribute-user-id (cdr e))))) (directory-files-and-attributes "sub" nil nil nil 'string)) (mapcar #'car (directory-files-and-attributes "sub" t "inn")) (length (directory-files-and-attributes "sub" nil nil t nil 1)) (condition-case e (directory-files-and-attributes "missing") (error (car e))))"#,
        r#"(list (unix-sync) (commandp 'unix-sync) (commandp 'do-auto-save) (set-binary-mode 'stdout t) (set-binary-mode 'stdin nil) (condition-case e (set-binary-mode 'bogus t) (error e)) (condition-case e (set-binary-mode "x" t) (error e)) (condition-case e (file--close-stream 'bogus) (error e)))"#,
        r#"(with-temp-buffer (insert "x") (list (buffer-modified-p) (progn (set-buffer-auto-saved) (buffer-modified-p)) (recent-auto-save-p) buffer-saved-size (clear-buffer-auto-save-failure) (progn (insert "y") (buffer-modified-p))))"#,
        // An auto-save of a modified visiting buffer, its list, then nothing
        // more to save.
        r#"(let ((auto-save-list-file-name (expand-file-name "list")) (b (find-file-noselect "a"))) (with-current-buffer b (goto-char (point-max)) (insert "more\n") (auto-save-mode 1)) (prog1 (list (do-auto-save t) (with-current-buffer b (list (buffer-modified-p) (recent-auto-save-p) buffer-saved-size (file-name-nondirectory buffer-auto-save-file-name))) (do-auto-save t t)) (with-current-buffer b (set-buffer-modified-p nil)) (kill-buffer b)))"#,
        // Auto-saving into the visited file is a real save: the buffer is
        // left unmodified and its file unlocked.
        r#"(let ((auto-save-visited-file-name t) (b (find-file-noselect "a"))) (with-current-buffer b (goto-char (point-max)) (insert "more\n") (auto-save-mode 1)) (prog1 (list (do-auto-save t) (with-current-buffer b (list (buffer-modified-p) (recent-auto-save-p) (file-locked-p buffer-file-name) (equal buffer-auto-save-file-name buffer-file-name)))) (kill-buffer b)))"#,
        // save-buffer backs the visited file up with the default
        // `make-backup-files', by renaming it to A~ (DIR is under
        // `temporary-file-directory', which the default predicate exempts).
        r#"(let ((backup-enable-predicate #'always) (b (find-file-noselect "a"))) (with-current-buffer b (goto-char (point-max)) (insert "new\n") (save-buffer)) (kill-buffer b) (list (file-exists-p "a~") (file-acl "a~") (file-acl "a")))"#,
    ];
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let directory = format!("{}/", dir.display());
    let mut failures = Vec::new();
    for expression in cases {
        let source = format!("(progn (setq default-directory {directory:?}) (string-to-list (prin1-to-string (condition-case err {expression} (error (list 'signal err))))))");
        fixture();
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let mut expected = (String::from_utf8(output.stdout).unwrap(), vec![]);
        snapshot(&dir, &dir, &mut expected.1);
        fixture();
        let mut actual = (vm.eval_source(&source).and_then(|v| v.display()).unwrap_or_else(|e| e), vec![]);
        snapshot(&dir, &dir, &mut actual.1);
        if actual != expected {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {} {:?}\n  Romacs {} {:?}", text(&expected.0), expected.1, text(&actual.0), actual.1));
        }
    }
    clear();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// File name handlers, as GNU dispatches them. Tramp's completion handler
// claims single-component names such as "/home" for `expand-file-name' and
// declines through `tramp-run-real-handler'; every primitive that expands
// such a name must then do what GNU does. RT-H logs each operation it is
// asked for and declines the same way, so each primitive's dispatch, the
// names it passes, and `inhibit-file-name-operation' are compared with
// pinned GNU's, along with the values and DIR's files afterwards.
#[test]
fn file_name_handlers_match_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("fnhandlers-{}", std::process::id()));
    let fixture = || {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("rt-h-sub")).unwrap();
        for (name, text) in [("rt-h-a", "hello\n"), ("rt-h-sub/inner", "inner\n"), ("plain", "plain\n")] {
            std::fs::write(dir.join(name), text).unwrap();
        }
        std::os::unix::fs::symlink("rt-h-a", dir.join("rt-h-link")).unwrap();
    };
    fn snapshot(dir: &Path) -> Vec<String> {
        let mut entries: Vec<_> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        entries.iter().map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            match std::fs::read_link(path) {
                Ok(target) => format!("{name} -> {}", target.display()),
                Err(_) if path.is_dir() => format!("{name}/"),
                Err(_) => format!("{name} {:?}", std::fs::read_to_string(path).unwrap()),
            }
        }).collect()
    }
    // Each case runs under RT-H with an empty log and yields (VALUE LOG).
    const HANDLER: &str = r#"(progn (defvar rt-log nil) (defvar rt-expanding nil) (defun rt-h (op &rest args) (push (cons op args) rt-log) (when (eq op 'expand-file-name) (run-hooks 'rt-expanding)) (let ((inhibit-file-name-handlers (cons 'rt-h (and (eq inhibit-file-name-operation op) inhibit-file-name-handlers))) (inhibit-file-name-operation op)) (apply op args))))"#;
    let handled = [
        r#"(file-symlink-p "rt-h-link")"#,
        r#"(list (file-attribute-type (file-attributes "rt-h-link")) (file-attribute-size (file-attributes "rt-h-a")) (file-attribute-type (file-attributes "rt-h-sub" 'string)))"#,
        r#"(file-truename "rt-h-link")"#,
        r#"(list (directory-files "rt-h-sub") (directory-files "rt-h-sub" t "inn") (mapcar #'car (directory-files-and-attributes "rt-h-sub" nil "inn")))"#,
        r#"(list (file-accessible-directory-p "rt-h-sub") (file-accessible-directory-p "rt-h-a"))"#,
        r#"(with-temp-buffer (insert-file-contents "rt-h-a") (buffer-string))"#,
        r#"(with-temp-buffer (list (insert-file-contents "rt-h-a" t) buffer-file-name (verify-visited-file-modtime) (progn (set-visited-file-modtime) (verify-visited-file-modtime))))"#,
        r#"(progn (write-region "x\n" nil "rt-h-new") (write-region "y\n" nil "rt-h-new" t) (with-temp-buffer (insert "z\n") (write-region nil nil "rt-h-vis" nil "rt-h-other")) (file-exists-p "rt-h-other"))"#,
        r#"(list (file-locked-p "rt-h-a") (with-temp-buffer (insert-file-contents "rt-h-a" t) (insert "m") (lock-file buffer-file-name) (prog1 (file-locked-p buffer-file-name) (unlock-file buffer-file-name))))"#,
        r#"(list (consp (file-system-info "rt-h-sub")) (file-name-case-insensitive-p "rt-h-a"))"#,
        r#"(list (get-file-buffer "rt-h-a") (let ((b (find-file-noselect "rt-h-a"))) (prog1 (eq b (get-file-buffer "rt-h-a")) (kill-buffer b))))"#,
        r#"(list (locate-file "rt-h-a" (list default-directory)) (file-newer-than-file-p "rt-h-a" "plain") (file-writable-p "rt-h-a") (file-modes "rt-h-a"))"#,
        // validate_region orders the bounds, and turns a marker into its
        // position, before the name expands: a handler sees positions, and
        // a marker the expansion moves does not change what is written.
        r#"(with-temp-buffer (insert "abcdef") (let ((m (copy-marker 3))) (defalias 'rt-move (lambda () (set-marker m 1))) (add-hook 'rt-expanding #'rt-move) (unwind-protect (list (write-region 6 2 "rt-h-rev") (write-region m 5 "rt-h-mark") (marker-position m)) (remove-hook 'rt-expanding #'rt-move))))"#,
    ];
    let cases: Vec<String> = handled.iter().map(|form| format!(
        "(let ((file-name-handler-alist (cons '(\"rt-h\" . rt-h) file-name-handler-alist))) (setq rt-log nil) (let ((value {form})) (list value (reverse rt-log))))"))
        .chain([
            // Tramp's own completion handler, as a session that loaded tramp has it.
            r#"(progn (require 'tramp) (list (find-file-name-handler "/home" 'expand-file-name) (file-symlink-p "/bin") (file-truename "/bin") (file-attribute-type (file-attributes "/bin")) (file-accessible-directory-p "/tmp") (file-name-case-insensitive-p "/home") (file-exists-p "/home") (file-directory-p "/tmp") (consp (file-system-info "/tmp")) (and (member "bin" (directory-files "/usr")) t) (with-temp-buffer (condition-case e (insert-file-contents "/tmp") (error (car e))))))"#.to_owned(),
        ]).collect();
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let directory = format!("{}/", dir.display());
    let mut failures = Vec::new();
    for expression in &cases {
        let source = format!("(progn {HANDLER} (setq default-directory {directory:?}) (string-to-list (prin1-to-string (condition-case err {expression} (error (list 'signal err))))))");
        fixture();
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let expected = (String::from_utf8(output.stdout).unwrap(), snapshot(&dir));
        fixture();
        let value = vm.eval_source(&source).and_then(|v| v.display()).unwrap_or_else(|e| e);
        let actual = (value, snapshot(&dir));
        if actual != expected {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {} {:?}\n  Romacs {} {:?}", text(&expected.0), expected.1, text(&actual.0), actual.1));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

// default-directory is GNU's per-buffer C variable (buffer.c:5426): always
// local, inherited by new buffers, kept by kill-all-local-variables, and
// what every file primitive expands against, so a `let' of it redirects
// them; expand-file-name follows fileio.c's rules and handlers, so a
// `/:'-quoted directory reaches file-name-non-special. Each case runs with
// default-directory DIR, a fresh copy of one fixture; every write names a
// subdirectory that exists only there, so a wrong base cannot touch the
// process directory. Its printed value and DIR afterwards must be pinned
// GNU's.
#[test]
fn default_directory_and_expansion_match_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut vm = bootstrapped_with(&root, "rlisp/romacs-loadup-bootstrap.el");
    let dir = std::env::temp_dir().join(format!("rt-default-directory-{}", std::process::id()));
    let fixture = || {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub/deep")).unwrap();
        for (name, text) in [("a", "hello"), ("sub/inner", "inner\n"), ("sub/deep/leaf", "leaf")] {
            std::fs::write(dir.join(name), text).unwrap();
        }
    };
    fn snapshot(base: &Path, at: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(at).unwrap().map(|e| e.unwrap().path()).collect();
        entries.sort();
        for path in entries {
            let name = path.strip_prefix(base).unwrap().display().to_string();
            if path.is_dir() {
                out.push(format!("{name}/"));
                snapshot(base, &path, out);
            } else {
                out.push(format!("{name} {:?}", std::fs::read_to_string(&path).unwrap()));
            }
        }
    }
    let cases = [
        // The variable itself.
        r#"(list (local-variable-p 'default-directory) (default-value 'default-directory) (assq 'default-directory (buffer-local-variables)) (special-variable-p 'default-directory))"#,
        r#"(let ((b (generate-new-buffer "rt-dd"))) (prog1 (list (equal (buffer-local-value 'default-directory b) default-directory) (with-current-buffer b (local-variable-p 'default-directory))) (kill-buffer b)))"#,
        r#"(list (with-temp-buffer default-directory) (let ((default-directory "/tmp/")) (with-temp-buffer default-directory)) default-directory)"#,
        r#"(with-temp-buffer (setq default-directory "/x/") (list (progn (kill-all-local-variables) default-directory) (progn (kill-all-local-variables t) default-directory)))"#,
        r#"(list (condition-case e (setq default-directory 3) (error e)) default-directory (progn (setq-default default-directory "/d/") (list (default-value 'default-directory) default-directory (with-temp-buffer default-directory))))"#,
        r#"(let ((b (generate-new-buffer "rt-dd")) r) (let ((default-directory "/tmp/")) (setq r (with-current-buffer b default-directory))) (prog1 (list r (with-current-buffer b (let ((default-directory "/q/")) (set-buffer (get-buffer-create " rt-other")) default-directory))) (kill-buffer b)))"#,
        // vc-setup-buffer sets the value under a `let' of it.
        r#"(list (let ((default-directory "/tmp/")) (set-buffer-local-toplevel-value 'default-directory "/var/") (list default-directory (buffer-local-toplevel-value 'default-directory))) default-directory)"#,
        // expand-file-name.
        r#"(list (expand-file-name "/tmp/x/") (expand-file-name "a//b/") (expand-file-name "//a") (expand-file-name "///a") (expand-file-name "/a/./b/../c") (expand-file-name "/a/b/..") (expand-file-name "..") (expand-file-name "../../..") (expand-file-name ".") (expand-file-name "") (expand-file-name "/.") (expand-file-name "/..") (expand-file-name "/x/../..") (expand-file-name "a" "//") (expand-file-name "a" "/b//"))"#,
        r#"(list (expand-file-name "~") (expand-file-name "~/") (expand-file-name "~/a/../b") (expand-file-name "a" "~/r") (expand-file-name "~root/x") (expand-file-name "~root") (expand-file-name "~nosuchuser-rt/a") (expand-file-name "a" "sub") (expand-file-name "x" "sub/../sub/deep/") (expand-file-name "/:/a/../b") (expand-file-name "/:~/x"))"#,
        r#"(list (let ((default-directory nil)) (expand-file-name "a")) (let ((default-directory 7)) (expand-file-name "a")) (equal (let ((default-directory "rel/")) (expand-file-name "a")) (concat invocation-directory "rel/a")) (expand-file-name "a" 7) (let ((x "/tmp/y")) (eq x (expand-file-name x))) (condition-case e (expand-file-name "a\0b") (error e)) (condition-case e (expand-file-name 1) (error e)) (condition-case e (expand-file-name "a" "/b\0") (error e)) (multibyte-string-p (expand-file-name (string-to-unibyte "a") "/")) (multibyte-string-p (expand-file-name "é" "/")) (multibyte-string-p (expand-file-name "\351" "/é/")))"#,
        r#"(progn (defalias 'rt-efn (lambda (op &rest args) (list op args))) (let ((file-name-handler-alist '(("\\`/rt-efn:" . rt-efn)))) (list (expand-file-name "/rt-efn:x") (expand-file-name "a" "/rt-efn:d/") (let ((default-directory "/rt-efn:dd/")) (expand-file-name "/tmp/z")) (expand-file-name "/rt-efn:x/../../q") (let ((inhibit-file-name-handlers '(rt-efn)) (inhibit-file-name-operation 'expand-file-name)) (expand-file-name "/rt-efn:x")))))"#,
        r#"(progn (defalias 'rt-efn2 (lambda (&rest _) 7)) (let ((file-name-handler-alist '(("\\`/rt-efn2:" . rt-efn2)))) (condition-case e (expand-file-name "/rt-efn2:x") (error e))))"#,
        // A let-bound directory reaches the file primitives.
        r#"(let ((default-directory (expand-file-name "sub/"))) (list (expand-file-name "f") (file-exists-p "inner") (file-readable-p "inner") (file-regular-p "inner") (file-directory-p "deep") (file-directory-p "") (file-exists-p "a") (file-writable-p "new") (file-modes "inner") (file-name-absolute-p "inner") (directory-files "." nil "^[^.]") (locate-file "leaf" '("deep")) (file-truename "deep/leaf")))"#,
        r#"(let ((default-directory (expand-file-name "sub/"))) (write-region "data" nil "w") (list (with-temp-buffer (insert-file-contents "w") (buffer-string)) (with-temp-buffer (insert-file-contents "inner" t) (list buffer-file-name (buffer-string)))))"#,
        r#"(let ((default-directory (expand-file-name "sub/"))) (copy-file "inner" "Makefile") (list (file-exists-p "Makefile") (progn (delete-file "inner") (file-exists-p "inner")) (progn (make-directory "made") (file-directory-p "made")) (progn (rename-file "Makefile" "made/") (directory-files "made" nil "^[^.]")) (progn (delete-directory "made" t) (file-exists-p "made"))))"#,
        r#"(with-temp-buffer (setq default-directory (expand-file-name "sub/deep/")) (list (file-exists-p "leaf") (with-temp-buffer (file-exists-p "leaf")) (progn (kill-all-local-variables) (file-exists-p "leaf"))))"#,
        // copy-directory into a NEWNAME ending in a slash. One call only:
        // its file-truename probes each ancestor's case sensitivity, which
        // VM2 refuses when a shared ancestor changes during the probe.
        r#"(progn (make-directory "sub/dest") (copy-directory "sub/deep" "sub/dest/") (directory-files "sub/dest" nil "^[^.]"))"#,
        // `/:'-quoted names and directories go through file-name-non-special.
        r#"(list (file-name-quote default-directory) (file-name-unquote (file-name-quote default-directory)) (file-name-quoted-p (file-name-quote "a")) (executable-find "sh") (let ((default-directory (file-name-quote default-directory))) (list (expand-file-name "a") (expand-file-name "/:~/x") (file-exists-p "a") (file-directory-p "sub") (file-regular-p "sub/inner") (file-exists-p "absent") (executable-find "sh") (locate-file "inner" '("sub")))))"#,
        r#"(list (file-exists-p (concat "/:" default-directory "a")) (file-directory-p (concat "/:" default-directory "sub/")) (file-modes (concat "/:" default-directory "a")) (expand-file-name (concat "/:" default-directory "sub/../a")) (progn (delete-file (concat "/:" default-directory "sub/inner")) (file-exists-p "sub/inner")))"#,
        // delete-directory-internal always takes the directory file name,
        // so a directory-file-name handler maps a name without a slash too.
        r#"(progn (make-directory "victim") (make-directory "mapped") (defalias 'rt-dfn (lambda (op &rest args) (if (eq op 'directory-file-name) (expand-file-name "mapped") (let ((inhibit-file-name-handlers '(rt-dfn)) (inhibit-file-name-operation op)) (apply op args))))) (list (let ((file-name-handler-alist '(("victim/?\\'" . rt-dfn)))) (delete-directory-internal "victim")) (make-directory "mapped") (let ((file-name-handler-alist '(("victim/?\\'" . rt-dfn)))) (delete-directory-internal "victim/")) (file-exists-p "victim") (file-exists-p "mapped")))"#,
        r#"(progn (make-directory "victim") (defalias 'rt-dfn7 (lambda (op &rest args) (if (eq op 'directory-file-name) 7 (let ((inhibit-file-name-handlers '(rt-dfn7)) (inhibit-file-name-operation op)) (apply op args))))) (let ((file-name-handler-alist '(("victim\\'" . rt-dfn7)))) (condition-case e (delete-directory-internal "victim") (error e))))"#,
        r#"(list (condition-case e (delete-file "a/") (error e)) (condition-case e (delete-file-internal "a/") (error e)) (file-exists-p "a"))"#,
        // rename-file into a directory expands FILE's last component there.
        r#"(let ((process-environment (cons (concat "HOME=" default-directory "home") process-environment))) (write-region "t" nil "./~") (rename-file "./~" "sub/") (list (file-exists-p "home") (file-exists-p "sub/~")))"#,
        // HOME comes from process-environment; its bytes join the result as
        // build_string and DECODE_FILE make them.
        r#"(let ((process-environment (cons (string-to-unibyte "HOME=/rt\351h") process-environment))) (mapcar (lambda (r) (list r (multibyte-string-p r) (string-bytes r))) (list (expand-file-name (string-to-unibyte "~/x") (string-to-unibyte "/")) (expand-file-name (string-to-multibyte "~/x") (string-to-multibyte "/")) (expand-file-name "~/é" "/") (expand-file-name (string-to-unibyte "~") "/d/") (expand-file-name (string-to-unibyte "~root/x") (string-to-unibyte "/")))))"#,
        r#"(let ((process-environment (cons "HOME=/rtéh" process-environment))) (mapcar (lambda (r) (list r (multibyte-string-p r))) (list (expand-file-name (string-to-unibyte "~/x") (string-to-unibyte "/")) (expand-file-name (string-to-multibyte "~/x") "/") (expand-file-name "~" "/"))))"#,
        r#"(let ((process-environment (list "HOME" "LOGNAME=root"))) (list (expand-file-name "~") (expand-file-name "~/x")))"#,
        // get_homedir reads HOME as a C string and takes a relative or empty
        // HOME from the working directory Emacs started in.
        r#"(let ((process-environment (cons "HOME=" process-environment))) (list (expand-file-name "~/tmp/victim") (expand-file-name "~") (expand-file-name "~/")))"#,
        r#"(let ((process-environment (cons "HOME=rel/h" process-environment))) (list (expand-file-name "~/x") (expand-file-name "~")))"#,
        r#"(let ((process-environment (cons (concat "HOME=/rt" (string 0) "junk") process-environment))) (list (expand-file-name "~/x") (expand-file-name "~")))"#,
        // set-file-modes and set-file-times expand against the buffer's
        // directory explicitly, which a handler sees.
        r#"(progn (setq rt-log nil rt-dir default-directory) (defalias 'rt-rec (lambda (op &rest args) (setq rt-log (cons (cons op args) rt-log)) (if (eq op 'expand-file-name) (concat rt-dir "a") (let ((inhibit-file-name-handlers '(rt-rec)) (inhibit-file-name-operation op)) (apply op args))))) (let ((file-name-handler-alist '(("\\`rt-rel" . rt-rec)))) (list (with-temp-buffer (setq default-directory "rt-rel/") (list (set-file-modes "f" 384) (set-file-times "g" '(0 0)))) (reverse rt-log) (file-modes (concat rt-dir "a")) (time-convert (file-attribute-modification-time (file-attributes (concat rt-dir "a"))) 'integer))))"#,
    ];
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let directory = format!("{}/", dir.display());
    let mut failures = Vec::new();
    for expression in cases {
        let source = format!("(progn (setq default-directory {directory:?}) (string-to-list (prin1-to-string (condition-case err {expression} (error (list 'signal err))))))");
        fixture();
        let output = std::process::Command::new(&binary).args(["-Q", "--batch", "--eval", &format!("(prin1 {source})")]).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let mut expected = (String::from_utf8(output.stdout).unwrap(), vec![]);
        snapshot(&dir, &dir, &mut expected.1);
        fixture();
        let mut actual = (vm.eval_source(&source).and_then(|v| v.display()).unwrap_or_else(|e| e), vec![]);
        snapshot(&dir, &dir, &mut actual.1);
        if actual != expected {
            let text = |codes: &str| codes.trim_matches(|c| c == '(' || c == ')').split_whitespace()
                .filter_map(|n| n.parse::<u32>().ok().and_then(char::from_u32)).collect::<String>();
            failures.push(format!("{expression}\n  GNU    {} {:?}\n  Romacs {} {:?}", text(&expected.0), expected.1, text(&actual.0), actual.1));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
