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
