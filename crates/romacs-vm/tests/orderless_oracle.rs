//! rlisp/romacs-orderless.el against orderless itself: pinned GNU loads
//! orderless.el e271a8e (tests/fixtures/orderless-e271a8e) and Romacs its
//! native port, and the same program must print the same, case by case:
//! the candidates `orderless-all-completions' returns, in order, with the
//! faces it highlights them with (eagerly and lazily), what
//! `orderless-try-completion' and `orderless-compile' return, the cached
//! file name table across a change to its directory, and minibuffer.el's
//! two candidate sorts, which Romacs answers natively.
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
    vm.load_file(&root.join("rlisp/romacs-orderless.el")).unwrap();
    vm
}

/// One printed line per case.  DIR is a fresh directory for the file
/// name table's cases.
const PROGRAM: &str = r#"
(let* ((print-escape-newlines t)
       ;; Each string as its text and the face of each character, so that
       ;; how equal adjacent intervals are split does not count.
       (faces (lambda (s) (if (stringp s)
                              (cons (substring-no-properties s)
                                    (let (fs) (dotimes (i (length s)) (push (get-text-property i 'face s) fs)) (nreverse fs)))
                            s)))
       (normal (lambda (r) (if (consp r)
                               (let ((out nil)) (while (consp r) (push (funcall faces (car r)) out) (setq r (cdr r)))
                                    (nconc (nreverse out) r))
                             r)))
       (completion-lazy-hilit nil)
       (cands '("find-file" "find-file-other-window" "find-file-literally"
                "file-0101.txt" "file-1999.txt" "File-Name.TXT"
                "completion-all-completions" "completion-all-sorted-completions"
                "orderless-flex" "a.b" "axb" "x*y" "foo[1]" "$HOME/bin" "C++"
                "two words" "line1\nline2-x" "αλφα-σ" "ΣΊΣΥΦΟΣ" "naïve-café"
                "Straße" "日本語-text" "fooBar" "foo-bar-baz" "f-b f-b-q" "ab" "a" ""))
       (styles '((nil "f" "fi fi" "FI" "file-1" "a.b" "[1" "o w" "^fi" "=a.b" ",ff"
                      "~ffow" "!find" "x*" "σ" "Σ" "ß" "café" "e-1 tx" "\\.txt"
                      "fi\\ le" "o-w$" "%cafe" "%" "foo b" "1 0" "日本")
                 ((orderless-flex) "ffow" "fl1" "aa" "αλ" "F" "fb")
                 ((orderless-initialism) "ffow" "fo" "cac" "fft" "nc" "fbq" "fb" "ft" "lx")
                 ((orderless-prefixes) "fi-fi-o" "co-al-co" "fil-1" "fi-" "f" "fo-ba")
                 ((orderless-literal-prefix) "fi" "Fi" "ab")
                 ((orderless-literal orderless-flex) "ffo" "fi")
                 ((orderless-without-literal) "file" "a")
                 ((orderless-regexp) "f.l" "^[a-c]")
                 ((orderless-initialism orderless-literal) "ff" "cac")))
       (strings (lambda (r) (let (out) (while (consp r) (push (car r) out) (setq r (cdr r))) (nreverse out))))
       (table (lambda (s p a) (complete-with-action a cands s p)))
       (out nil))
  (dolist (group styles)
    (dolist (input (cdr group))
      (let ((orderless-matching-styles (or (car group) orderless-matching-styles)))
        (push (list (car group) input
                    (condition-case e (funcall normal (orderless-all-completions input cands nil (length input)))
                      (error (list 'error e)))
                    (condition-case e (orderless-try-completion input cands nil (length input))
                      (error (list 'error e)))
                    (cdr (orderless-compile input)))
              out))))
  ;; Smart case off: `completion-ignore-case' decides.
  (dolist (ic '(nil t))
    (let ((orderless-smart-case nil) (completion-ignore-case ic))
      (push (list 'ignore-case ic (funcall normal (orderless-all-completions "FI" cands nil 2))) out)))
  ;; A function table, with a predicate and without.
  (push (list 'table (funcall normal (orderless-all-completions "o f" table nil 3))
              (funcall normal (orderless-all-completions "o f" table (lambda (c) (not (string-search "w" c))) 3)))
        out)
  ;; Lazy highlighting: the candidates bare, the faces on demand.
  (let* ((completion-lazy-hilit t) (completion-lazy-hilit-fn nil)
         (all (funcall strings (orderless-all-completions "fi l" cands nil 4))))
    (push (list 'lazy (funcall normal all) (funcall normal (mapcar #'completion-lazy-hilit all))) out))
  ;; The file name table, before and after its directory changes.
  (let* ((dir (file-name-as-directory (make-temp-file "orderless-oracle" t)))
         (default-directory dir)
         (sorted (lambda (r) (funcall normal (sort (funcall strings r) #'string<))))
         (files '("file-01.txt" "file-10.txt" "File-02.TXT" "a$b.txt" "notes.org")))
    (make-directory (expand-file-name "sub" dir))
    (dolist (f (cons "sub/file-11.txt" files)) (write-region "" nil (expand-file-name f dir)))
    (dolist (input (quote ("fi 1" "sub/fi" "s" "org" "a$$b")))
      (push (list 'files input (funcall sorted (orderless-all-completions input #'read-file-name-internal nil (length input))))
            out))
    ;; Regexps match the entry's name: before a directory's slash, and
    ;; before `read-file-name-internal' doubles its dollars.
    (dolist (input '("sub\\'" "a.b" "a\\$b"))
      (push (list 'files-raw input (funcall sorted (orderless-all-completions input #'read-file-name-internal nil (length input)))
                  (funcall sorted (orderless-all-completions input #'completion-file-name-table nil (length input))))
            out))
    ;; read-file-name's predicate.
    (push (list 'files-exist (funcall sorted (orderless-all-completions "1 fi" #'read-file-name-internal #'file-exists-p 4)))
          out)
    (write-region "" nil (expand-file-name "file-12.txt" dir))
    (push (list 'files-after (funcall sorted (orderless-all-completions "fi 1" #'read-file-name-internal nil 4)))
          out)
    (delete-directory dir t))
  ;; minibuffer.el's sorts.
  (let ((l '("bb" "a" "ccc" "b" "B" "ab" "é" "e" "" "aa" "a")))
    (push (list 'sort (minibuffer--sort-by-length-alpha (copy-sequence l))
                (minibuffer--sort-by-position '("ccc" "b" "x" "ccc" "aa") (copy-sequence l))
                (minibuffer--sort-by-position nil (copy-sequence l)))
          out))
  (mapconcat #'prin1-to-string (nreverse out) "\n"))
"#;

/// orderless.el requires compat, which pinned GNU lacks; it calls one
/// compat function, which GNU 32 has.
const COMPAT: &str = "(progn (provide 'compat) (defmacro compat-call (f &rest args) (cons f args)))";

#[test]
fn romacs_orderless_matches_orderless_on_pinned_gnu() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let reference = root.join("crates/romacs-vm/tests/fixtures/orderless-e271a8e/orderless.el");
    let output = std::process::Command::new(&binary)
        .args(["-Q", "--batch", "--eval", COMPAT, "-l", reference.to_str().unwrap(),
               "--eval", &format!("(princ {PROGRAM})")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let expected = String::from_utf8(output.stdout).unwrap();
    let actual = std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.eval_source(PROGRAM).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
    }).unwrap().join().unwrap().unwrap();
    let (expected, actual): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    assert_eq!(expected.len(), actual.len(), "case count");
    let failures: Vec<String> = expected.iter().zip(&actual).filter(|(e, a)| e != a)
        .map(|(e, a)| format!("GNU    {e}\nRomacs {a}")).collect();
    assert!(failures.is_empty(), "{} of {} cases differ:\n{}", failures.len(), expected.len(), failures.join("\n\n"));
}
