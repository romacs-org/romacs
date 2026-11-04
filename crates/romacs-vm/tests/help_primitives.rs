//! The help primitives (rlisp/romacs-help.el, keymap.c and syntax.c in
//! GNU) against pinned GNU 32.0.50 on the loadup chain, each case's value
//! with the text it inserted and that text's `font-lock-face' runs:
//! `text-char-description', `internal-describe-syntax-value',
//! `describe-vector' over vectors, char-tables and syntax tables,
//! `help--describe-vector' with prefixes, suppression, shadowing and
//! ranges split by shadowing, `current-minor-mode-maps' and
//! `describe-buffer-bindings' under a prefix and with an overriding map.
//! A character above #x10FFFF, which a Romacs buffer cannot hold, is
//! U+FFFD in GNU's text too (DIVERGENCES.md VM2-HELP-EXTENDED).
use romacs_vm::Vm;
use std::path::Path;

fn bootstrapped(root: &Path) -> Vm {
    let mut vm = Vm::new();
    vm.eval_source("(setq noninteractive t)").unwrap();
    vm.set_frame_geometry(80, 25, 1, 1).unwrap();
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
    vm.load_file(&root.join("rlisp/romacs-loadup-bootstrap.el")).unwrap();
    vm
}

/// Definitions both sides evaluate first: HP-OUT runs BODY in a temporary
/// buffer and gives its value, text and face runs.
const SETUP: &str = r#"(progn
 (defun hp-norm (s)
   (if (fboundp 'romacs-help--insertable) s
     (replace-regexp-in-string (concat "[" (string #x110000) "-" (string #x3FFF7F) "]") (lambda (_) (string #xFFFD)) s t t)))
 (defmacro hp-out (&rest body)
   `(with-temp-buffer
      (let ((ret (condition-case err (progn ,@body) (error (list 'ERROR err)))))
        (list ret (hp-norm (buffer-substring-no-properties (point-min) (point-max)))
              (let (runs (p (point-min)))
                (while (< p (point-max))
                  (let ((f (get-text-property p 'font-lock-face))) (when f (push (cons p f) runs)))
                  (setq p (1+ p)))
                (nreverse runs))))))
 (defun hp-map ()
   (let ((m (make-keymap)))
     (define-key m "a" 'hp-foo) (define-key m "b" 'hp-foo) (define-key m "c" 'hp-bar)
     (define-key m "x" 'hp-supp) (define-key m (kbd "C-a") 'hp-ctl) (define-key m "\e" 'hp-esc)
     m))
 (put 'hp-supp 'suppress-keymap t)
 (defvar hp-minor t) (defvar hp-mode-a t) (defvar hp-mode-b nil) (defvar hp-mode-c t)
 t)"#;

const CASES: &[&str] = &[
    "(mapcar (lambda (c) (let ((s (text-char-description c))) (list s (multibyte-string-p s)))) '(0 9 27 31 32 65 127 128 233 945 #x3FFF80 #x3FFFFF))",
    "(mapcar (lambda (c) (condition-case e (text-char-description c) (error e))) '(a -1 134217825))",
    r#"(mapcar (lambda (d) (hp-out (internal-describe-syntax-value (string-to-syntax d))))
       '(" " "." "w" "_" "()" ")(" "'" "\"" "$" "\\" "/" "<" ">" "@" "!" "|" ". 124" ". 23b" "w p" "< 1n" "> c" "w 1234pbcn"))"#,
    "(mapcar (lambda (s) (hp-out (internal-describe-syntax-value s))) (list nil (make-char-table 'syntax-table) 'foo '(a) '(1 . x) '(16) '(-1)))",
    r#"(hp-out (describe-vector [a a b nil c c c "s" (x . y) (x . y)]))"#,
    "(hp-out (describe-vector [a nil a b] (lambda (x) (insert (format \"<%S>\" x)))))",
    "(list (hp-out (describe-vector [])) (hp-out (describe-vector '(1 2))))",
    "(hp-out (let ((ct (make-char-table 'test))) (set-char-table-range ct '(65 . 90) 'up) (aset ct 97 'a) (aset ct 98 'a) (aset ct 200 'b) (set-char-table-range ct '(#x3FFF00 . #x3FFFFF) 'raw) (describe-vector ct)))",
    "(hp-out (let ((ct (make-char-table 'test))) (set-char-table-range ct nil 'dflt) (aset ct 65 'x) (describe-vector ct)))",
    "(hp-out (let ((p (make-char-table 'test)) (ct (make-char-table 'test))) (aset p 66 'parent) (aset ct 65 'own) (set-char-table-parent ct p) (list (describe-vector ct) (eq (char-table-parent ct) p))))",
    "(hp-out (let ((ct (make-char-table 'test))) (aset ct 65 '(\"Menu\" . cmd)) (aset ct 66 '(menu-item \"x\" cmd2 :help \"h\")) (describe-vector ct)))",
    "(hp-out (describe-vector text-mode-syntax-table #'internal-describe-syntax-value))",
    "(hp-out (describe-vector emacs-lisp-mode-syntax-table #'internal-describe-syntax-value))",
    "(hp-out (let ((m (hp-map))) (help--describe-vector (cadr m) nil #'help--describe-command nil nil m nil)))",
    "(hp-out (let ((m (hp-map))) (help--describe-vector (cadr m) [?\\C-x] #'help--describe-command nil nil m nil)))",
    "(hp-out (let ((m (hp-map))) (help--describe-vector (cadr m) nil #'help--describe-command t nil m nil)))",
    "(hp-out (let ((m (hp-map)) (s (make-sparse-keymap))) (define-key s \"a\" 'hp-shadow) (help--describe-vector (cadr m) nil #'help--describe-command nil (list s) m nil)))",
    "(hp-out (let ((m (hp-map)) (s (make-sparse-keymap))) (define-key s \"a\" 'hp-shadow) (define-key s \"c\" (make-sparse-keymap)) (help--describe-vector (cadr m) nil #'help--describe-command nil (list s) m t)))",
    "(hp-out (let ((v (make-vector 5 nil))) (aset v 1 'one) (aset v 2 'one) (aset v 4 'four) (help--describe-vector v [?\\C-c] #'help--describe-command nil nil nil nil)))",
    "(hp-out (let ((m (make-keymap)) (s (make-sparse-keymap))) (set-char-table-range (cadr m) '(?a . ?z) 'self-insert-command) (define-key s \"m\" 'hp-m) (let ((describe-bindings-check-shadowing-in-ranges t)) (help--describe-vector (cadr m) nil #'help--describe-command nil (list s) m t))))",
    "(hp-out (help--describe-vector 'x nil #'ignore nil nil nil nil))",
    "(let ((minor-mode-map-alist (list (cons 'hp-mode-a '(keymap (1 . a))) (cons 'hp-mode-b '(keymap (2 . b))) (cons 'hp-mode-c '(keymap (3 . c))))) (minor-mode-overriding-map-alist (list (cons 'hp-mode-c '(keymap (4 . over))))) (emulation-mode-map-alists nil)) (current-minor-mode-maps))",
    "(let ((buf (generate-new-buffer \"hp\")) (local (make-sparse-keymap)) (minor (make-sparse-keymap))) (define-key local (kbd \"<f12> a\") 'hp-local-a) (define-key local (kbd \"<f12> b\") 'hp-local-b) (define-key minor (kbd \"<f12> a\") 'hp-minor-a) (with-current-buffer buf (use-local-map local) (setq major-mode 'hp-major-mode)) (let ((minor-mode-map-alist (cons (cons 'hp-minor minor) minor-mode-map-alist))) (prog1 (hp-out (describe-buffer-bindings buf [f12])) (kill-buffer buf))))",
    "(let ((buf (generate-new-buffer \"hp\")) (local (make-sparse-keymap))) (define-key local (kbd \"<f12> a\") 'hp-local-a) (with-current-buffer buf (use-local-map local)) (prog1 (hp-out (describe-buffer-bindings buf [f12] t)) (kill-buffer buf)))",
    "(let ((buf (generate-new-buffer \"hp\")) (over (make-sparse-keymap))) (define-key over (kbd \"<f12> o\") 'hp-over) (let ((overriding-local-map over)) (prog1 (hp-out (describe-buffer-bindings buf [f12])) (kill-buffer buf))))",
    "(condition-case e (describe-buffer-bindings \"nope\") (error e))",
];

fn gnu(oracle: &str, form: &str) -> String {
    let program = format!("(progn {SETUP} (princ (prin1-to-string {form})))");
    let output = std::process::Command::new(oracle).args(["-Q", "--batch", "--eval", &program])
        .stdin(std::process::Stdio::null()).output().unwrap();
    assert!(output.status.success(), "{form}: {}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn help_primitives_match_pinned_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expected: Vec<String> = CASES.iter().map(|form| gnu(&oracle, form)).collect();
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.eval_source(SETUP).unwrap();
        CASES.iter().map(|form| vm.eval_source(&format!("(prin1-to-string {form})")).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))).collect::<Vec<_>>()
    }).unwrap().join().unwrap();
    for ((form, gnu), mine) in CASES.iter().zip(&expected).zip(&actual) {
        assert_eq!(mine.as_deref(), Ok(gnu.as_str()), "{form}");
    }
}
