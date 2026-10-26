use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped(root: &Path) -> Vm {
    bootstrapped_with(root, "rlisp/romacs-gnu-ert-bootstrap.el")
}

fn bootstrapped_with(root: &Path, bootstrap: &str) -> Vm {
    let mut vm = Vm::new();
    // GNU batch, as the gnu-ert runner sets it before the chain loads.
    vm.eval_source("(setq noninteractive t)").unwrap();
    // GNU's installed load-path (gnu-tests/run.py load_path): the Lisp root,
    // its subdirectories in reverse name order with obsolete last, then rlisp.
    let lisp = root.join("gnu-lisp");
    let mut names: Vec<String> = std::fs::read_dir(&lisp).unwrap()
        .filter_map(|entry| entry.ok().filter(|e| e.path().is_dir()).and_then(|e| e.file_name().into_string().ok()))
        .filter(|name| !["term", "RCS", "CVS", "Old"].contains(&name.as_str()) && !name.starts_with('=')
            && !name.ends_with('~') && !name.ends_with(".orig") && !name.ends_with(".rej"))
        .collect();
    names.sort();
    let mut paths = vec![lisp.clone()];
    paths.extend(names.iter().rev().filter(|n| *n != "obsolete").map(|n| lisp.join(n)));
    if names.iter().any(|n| n == "obsolete") { paths.push(lisp.join("obsolete")); }
    paths.push(root.join("rlisp"));
    vm.set_load_path(&paths).unwrap();
    vm.install_lisp_prelude().unwrap();
    for name in [bootstrap, "gnu-lisp/help-fns.el", "rlisp/romacs-cl-print.el", "gnu-lisp/emacs-lisp/ert.el"] {
        if let Err(error) = vm.load_file(&root.join(name)).map(|_| ()) {
            let report = vm.load_report();
            let failed = report["steps"].as_array().unwrap().iter()
                .filter(|step| step["status"] != "ok").collect::<Vec<_>>();
            panic!("{name}: {error}\n{failed:?}");
        }
    }
    vm
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
