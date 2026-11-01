//! rlisp/romacs-fzf.el's completion semantics against pinned GNU.  GNU has
//! no fzf style, so each case is answered there by GNU's own machinery:
//! a single fuzzy term by GNU's `flex' style, which matches the same
//! subsequences, and an extended query by its terms' meaning over the
//! candidates GNU's table lists past the boundary it reports, with GNU's
//! regexps and case folding.  Romacs answers every case through
//! `completion-all-completions' and `completion-try-completion' with
//! `completion-styles' set to (fzf), and the two must print the same:
//! the matches (as a set: GNU has no fzf ranking), the base size, and
//! the try-completion result.  The ranking itself is fzf's own tests'
//! (src/fzf.rs); here Romacs's metadata sort is checked to give it.
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
    vm.load_file(&root.join("rlisp/romacs-fzf.el")).unwrap();
    vm
}

/// GNU's answers.  A query's MEANING is a list of sets, all of which must
/// match, a set when any of its terms does: (KIND TEXT SENSITIVE) with
/// KIND `fuzzy', `exact', `prefix', `suffix', `equal' or `not'.
const GNU: &str = r#"
(progn
  (defun oracle--term (term c)
    (pcase-let* ((`(,kind ,text ,sensitive) term)
                 (case-fold-search (not sensitive))
                 (q (regexp-quote text)))
      (pcase kind
        ('fuzzy (string-match-p (mapconcat (lambda (ch) (regexp-quote (char-to-string ch))) text ".*") c))
        ('exact (string-match-p q c))
        ('prefix (string-match-p (concat "\\`" q) c))
        ('suffix (string-match-p (concat q "\\'") c))
        ('equal (string-match-p (concat "\\`" q "\\'") c))
        ('not (not (string-match-p q c))))))
  (defun oracle-all (input meaning table pred)
    (if (null meaning)
        (let* ((completion-styles '(flex))
               (r (completion-all-completions input table pred (length input))))
          (list (sort (mapcar #'substring-no-properties (oracle--strings r)) #'string<) (cdr (last r))))
      (let* ((limit (car (completion-boundaries input table pred "")))
             (prefix (substring input 0 limit))
             (all (all-completions prefix table pred))
             (names (mapcar (lambda (c) (if (string-suffix-p "/" c) (substring c 0 -1) c)) all))
             (out nil))
        (while all
          (when (seq-every-p (lambda (set) (seq-some (lambda (term) (oracle--term term (car names))) set)) meaning)
            (push (car all) out))
          (setq all (cdr all) names (cdr names)))
        (list (sort out #'string<) (and out (/= limit 0) limit)))))
  (defun oracle-try (input meaning table pred)
    (if (null meaning)
        'flex
      (or (completion-emacs21-try-completion input table pred (length input))
          (let* ((m (car (oracle-all input meaning table pred)))
                 (prefix (substring input 0 (car (completion-boundaries input table pred "")))))
            (cond ((null m) nil)
                  ((seq-some (lambda (x) (not (equal x (car m)))) (cdr m)) (cons input (length input)))
                  (t (let ((one (concat prefix (car m))))
                       (or (equal input one) (cons one (length one)))))))))))
"#;

/// Romacs's answers, through the style machinery.
const ROMACS: &str = r#"
(progn
  (defun oracle-all (input _meaning table pred)
    (let* ((completion-styles '(fzf))
           (r (completion-all-completions input table pred (length input))))
      (list (sort (mapcar #'substring-no-properties (oracle--strings r)) #'string<) (cdr (last r)))))
  (defun oracle-try (input meaning table pred)
    (if (null meaning)
        'flex
      (let ((completion-styles '(fzf)))
        (completion-try-completion input table pred (length input))))))
"#;

/// The cases, one printed line each.  A case without a meaning is a
/// single fuzzy term, which GNU's flex answers.
const PROGRAM: &str = r#"
(progn
  (defun oracle--strings (r) (let (out) (while (consp r) (push (car r) out) (setq r (cdr r))) (nreverse out)))
  (let* ((print-escape-newlines t)
         (cands '("find-file" "find-file-other-window" "find-file-literally" "file-0101.txt"
                  "file-1999.txt" "File-Name.TXT" "completion-all-completions" "two words"
                  "foo/bar.el" "fooBar" "αλφα-σ" "Straße" "naïve-café" "old.txt" "a.txt" "ab" "a" ""))
         (hash (let ((h (make-hash-table :test #'equal))) (dolist (c cands) (puthash c t h)) h))
         (tables `((list . ,cands) (alist . ,(mapcar #'list cands)) (hash . ,hash)
                   (function . ,(lambda (s p a) (complete-with-action a cands s p)))))
         (fuzzy '("f" "ff" "fil" "ffow" "fl1" "comp" "αλ" "fb" "xyz" "" "ab"))
         ;; (QUERY . MEANING): MEANING in GNU's terms, as fzf parses QUERY.
         (extended
          '(("fi 1" ((fuzzy "fi" nil)) ((fuzzy "1" nil)))
            ("'file" ((exact "file" nil)))
            ("^fi" ((prefix "fi" nil)))
            ("txt$" ((suffix "txt" nil)))
            ("^ab$" ((equal "ab" nil)))
            ("!old txt$" ((not "old" nil)) ((suffix "txt" nil)))
            ("^old | ^a. txt" ((prefix "old" nil) (prefix "a." nil)) ((fuzzy "txt" nil)))
            ("TXT" ((fuzzy "TXT" t)))
            ("Fi 'name" ((fuzzy "Fi" t)) ((exact "name" nil)))
            ("two\\ w" ((fuzzy "two w" nil)))
            ("ffow" ((fuzzy "ffow" nil)))
            ("!f" ((not "f" nil)))
            ("STRASSE" ((fuzzy "STRASSE" t)))))
         (out nil))
    (dolist (ic '(t nil))
      (let ((completion-ignore-case ic) (romacs-fzf-smart-case nil))
        (dolist (table tables)
          (dolist (input fuzzy)
            (push (list ic (car table) input (oracle-all input nil (cdr table) nil)) out)))
        ;; A predicate, which the table applies.
        (dolist (input '("fi" "o"))
          (push (list ic 'pred input (oracle-all input nil cands (lambda (c) (not (string-search "w" c))))) out))))
    ;; Smart case, as Romacs defaults it.
    (dolist (table tables)
      (dolist (case extended)
        (let ((completion-ignore-case nil))
          (push (list 'smart (car table) (car case)
                      (oracle-all (car case) (cdr case) (cdr table) nil)
                      (oracle-try (car case) (cdr case) (cdr table) nil))
                out))))
    ;; The file name table: boundaries at the last slash, a directory
    ;; matched by its name, a change to the directory seen.
    (let* ((dir (file-name-as-directory (make-temp-file "fzf-oracle" t)))
           (default-directory dir)
           (completion-ignore-case nil)
           (files '("file-01.txt" "file-10.txt" "File-02.TXT" "notes.org" "a$b.txt" "sub/file-11.txt" "sub/x.el" "xsub/y.el")))
      (make-directory (expand-file-name "sub" dir))
      (make-directory (expand-file-name "xsub" dir))
      (dolist (f files) (write-region "" nil (expand-file-name f dir)))
      (let ((romacs-fzf-smart-case nil))
        (dolist (input '("fi" "fil1" "sub/fi" "s" "org" "nt"))
          (push (list 'files input (oracle-all input nil #'read-file-name-internal nil)) out)))
      ;; A `$' ends a query only doubled: a single one starts an
      ;; environment variable's name, GNU's boundary.
      (dolist (case '(("fi 1" ((fuzzy "fi" nil)) ((fuzzy "1" nil)))
                      ("txt$$" ((suffix "txt" nil)))
                      ("sub/^x" ((prefix "x" nil)))
                      ("sub$$" ((suffix "sub" nil)))
                      ("^n" ((prefix "n" nil)))
                      ("a$$b" ((fuzzy "a$b" nil)))))
        (push (list 'files (car case)
                    (oracle-all (car case) (cdr case) #'read-file-name-internal nil)
                    (oracle-try (car case) (cdr case) #'read-file-name-internal nil))
              out))
      ;; `read-directory-name''s predicate, which the table applies.
      (push (list 'directories (oracle-all "sub$$" '(((suffix "sub" nil))) #'read-file-name-internal #'file-directory-p))
            out)
      (write-region "" nil (expand-file-name "file-12.txt" dir))
      (let ((romacs-fzf-smart-case nil))
        (push (list 'files-after (oracle-all "fi" nil #'read-file-name-internal nil)) out))
      (delete-directory dir t))
    (mapconcat #'prin1-to-string (nreverse out) "\n")))
"#;

/// What only Romacs has: the metadata sort ranks as fzf does, lazily and
/// eagerly highlighted characters agree.
const ROMACS_ONLY: &str = r#"
(let* ((cands '("fuzzyfinder" "fuzzy-blurry-finder" "fuzzy-finder" "xff"))
       (md (list 'metadata '(category . test)))
       (completion-styles '(fzf))
       (all (completion-all-completions "ff" cands nil 2 md))
       (sorted (funcall (completion-metadata-get md 'display-sort-function) (oracle--strings all)))
       (faces (lambda (s) (let (fs) (dotimes (i (length s)) (push (and (get-text-property i 'face s) i) fs)) (delq nil (nreverse fs)))))
       (eager (let ((completion-lazy-hilit nil)) (oracle--strings (completion-all-completions "fb 'bar" '("foo/bar.el") nil 7))))
       (lazy (let ((completion-lazy-hilit t))
               (mapcar #'completion-lazy-hilit (oracle--strings (completion-all-completions "fb 'bar" '("foo/bar.el") nil 7)))))
       ;; With a sort of its own, the table's order breaks ties.
       (md2 (list 'metadata (cons 'display-sort-function (lambda (l) (sort l #'string>)))))
       (all2 (completion-all-completions "ab" '("xab" "zab" "yab") nil 2 md2))
       ;; File names: matched unquoted and by name, highlighted and
       ;; re-sorted the same way.
       (files (let* ((dir (file-name-as-directory (make-temp-file "fzf-only" t)))
                     (default-directory dir))
                (make-directory "sub") (make-directory "xsub") (write-region "" nil "a.txt")
                (prog1 (list (let ((completion-lazy-hilit nil))
                               (mapcar faces (oracle--strings (completion-all-completions "txt$$" #'read-file-name-internal nil 5))))
                             (let* ((md (list 'metadata (cons 'display-sort-function (lambda (l) (sort l #'string>)))))
                                    (all (completion-all-completions "sub$$" #'read-file-name-internal #'file-directory-p 5 md)))
                               (mapcar #'substring-no-properties
                                       (funcall (completion-metadata-get md 'display-sort-function) (oracle--strings all)))))
                  (delete-directory dir t)))))
  (list (mapcar #'substring-no-properties sorted)
        (mapcar faces eager) (mapcar faces lazy)
        (mapcar #'substring-no-properties
                (funcall (completion-metadata-get md2 'display-sort-function) (oracle--strings all2)))
        (oracle--strings (completion-all-completions "!x" '("b" "a" "x") nil 2 (list 'metadata)))
        files))
"#;

fn run_gnu(binary: &str, program: &str) -> String {
    let output = std::process::Command::new(binary)
        .args(["-Q", "--batch", "--eval", &format!("(princ {program})")])
        .output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    String::from_utf8(output.stdout).unwrap()
}

fn run_romacs(root: std::path::PathBuf, program: String) -> String {
    std::thread::Builder::new().stack_size(256 << 20).spawn(move || {
        let mut vm = bootstrapped(&root);
        vm.eval_source(&program).and_then(|v| v.display())
            .map(|s| serde_json::from_str::<String>(&s).unwrap_or(s))
    }).unwrap().join().unwrap().unwrap()
}

#[test]
fn fzf_style_keeps_gnu_completion_semantics() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let binary = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let expected = run_gnu(&binary, &format!("(progn {GNU} {PROGRAM})"));
    let actual = run_romacs(root, format!("(progn {ROMACS} {PROGRAM})"));
    let (expected, actual): (Vec<&str>, Vec<&str>) = (expected.lines().collect(), actual.lines().collect());
    assert_eq!(expected.len(), actual.len(), "case count");
    let failures: Vec<String> = expected.iter().zip(&actual).filter(|(e, a)| e != a)
        .map(|(e, a)| format!("GNU    {e}\nRomacs {a}")).collect();
    assert!(failures.is_empty(), "{} of {} cases differ:\n{}", failures.len(), expected.len(), failures.join("\n\n"));
}

#[test]
fn fzf_style_ranks_and_highlights() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let program = format!(
        "(progn (defun oracle--strings (r) (let (out) (while (consp r) (push (car r) out) (setq r (cdr r))) (nreverse out))) \
         (prin1-to-string {ROMACS_ONLY}))");
    let actual = run_romacs(root, program);
    assert_eq!(actual, concat!(
        r#"(("fuzzy-finder" "fuzzyfinder" "fuzzy-blurry-finder" "xff") "#,
        r#"((0 4 5 6)) ((0 4 5 6)) ("zab" "yab" "xab") ("b" "a") (((2 3 4)) ("sub/" "xsub/")))"#));
}
