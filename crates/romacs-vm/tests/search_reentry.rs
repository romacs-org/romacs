//! A search's callees run other searches and collections, under forced
//! moving collections (`Vm::set_gc_stress`), and every search still answers
//! as pinned GNU's: an outer buffer search loop whose variable watcher
//! searches, a string search loop calling Lisp between matches, and the
//! native completion matcher (`completion-regexp-list') calling a predicate
//! between candidates.
//!
//! No Lisp and no collection can run inside a match: the matcher borrows
//! the VM shared (`regexp_syntax.rs' `VmSyntax'), and running Lisp needs it
//! mutable. What is left to prove is the space between matches, where the
//! outer search's state is match data, point and the native matcher's own.
use romacs_vm::Vm;

// `save-match-data' is subr.el's, which the bare VM has not loaded: the
// case brings its own, from the same primitives, in a form before the one
// that uses it.
const SETUP: &str = r#"(progn
  (defvar rs-log nil) (defvar rs-gcs nil) (defvar rs-watched nil)
  (defmacro rs-saving (&rest body)
    (list 'let '((rs-data (match-data)))
          (list 'unwind-protect (cons 'progn body) '(set-match-data rs-data t)))))"#;

const CASE: &str = r#"(progn
  (fset 'rs-inner
        (lambda (tag)
          (setq rs-log (cons (list tag (and (string-match "\\(b+\\)\\(c\\)?" "aabbbd")
                                          (match-string 1 "aabbbd"))
                                   (match-data))
                             rs-log))
          (with-temp-buffer
            (insert "foo bar") (goto-char 1)
            (setq rs-log (cons (list (re-search-forward "ba\\(r\\)" nil t) (match-beginning 1)) rs-log)))
          (setq rs-gcs (cons (consp (garbage-collect)) rs-gcs))
          (make-string 64 ?x)))
  (add-variable-watcher 'rs-watched (lambda (&rest _) (rs-inner 'watcher)))
  (setq post-gc-hook
        (list (lambda () (rs-saving
                           (setq rs-log (cons (list 'hook (string-match "q" "aq")) rs-log))))))
  (prog1
      (list
       (with-temp-buffer
         (insert "key1=val1; key22=val22; key333=val333")
         (goto-char (point-min))
         (let (out)
           (while (re-search-forward "\\(key[0-9]+\\)=\\(val[0-9]+\\)" nil t)
             (let ((md (match-data)) (k (match-string 1)))
               (rs-saving (setq rs-watched (copy-sequence k)))
               (setq out (cons (list k (match-string 2) (equal md (match-data)) (point)) out))
               (rs-inner 'bare)))
           (nreverse out)))
       (let ((s "a1 b22 c333") (start 0) (out nil))
         (while (string-match "[0-9]+" s start)
           (let ((m (match-string 0 s)) (end (match-end 0)))
             (rs-saving (rs-inner 'loop))
             (setq out (cons (* 2 (string-to-number m)) out) start end)))
         (nreverse out))
       ;; Collections made at run time, so stress moves them: a list, an
       ;; alist, a hash table and an obarray, each walked by the native
       ;; matcher between predicate calls that search and collect.
       (let* ((names (mapcar #'copy-sequence '("foo" "fob" "fax" "food" "fib" "f-o" "fo-b")))
              (alist (mapcar (lambda (n) (cons n (length n))) names))
              (table (make-hash-table :test 'equal))
              (symbols (obarray-make))
              (pred (lambda (s &rest _)
                      (rs-inner 'pred)
                      (not (string-match "d\\'" (cond ((symbolp s) (symbol-name s)) ((consp s) (car s)) (t s)))))))
         (dolist (n names) (puthash n (length n) table) (intern n symbols))
         (list
          (let ((completion-regexp-list '("o\\(o\\|b\\)")))
            (list (all-completions "f" names pred) (all-completions "f" alist pred)
                  (sort (all-completions "f" table pred) 'string-lessp)
                  (sort (all-completions "f" symbols pred) 'string-lessp)
                  (try-completion "f" names pred)))
          ;; A pattern only a syntax table decides: `-' is a word
          ;; constituent here, a symbol constituent in the predicate's own
          ;; searches, which run in buffers of the standard table.
          (let ((words (copy-syntax-table (standard-syntax-table))))
            (modify-syntax-entry ?- "w" words)
            (with-temp-buffer
              (set-syntax-table words)
              (let ((completion-regexp-list '("\\`f\\sw+\\'")))
                (list (all-completions "f" names pred)
                      (sort (all-completions "f" table pred) 'string-lessp)))))))
       ;; The hook's entries fall where collections do, and Romacs runs it
       ;; once for the collections before a safe point (VM2-POST-GC).
       (length (delq nil (mapcar (lambda (e) (unless (eq (car-safe e) 'hook) e)) rs-log)))
       (delq nil (mapcar (lambda (e) (unless (eq (car-safe e) 'hook) (prin1-to-string e))) rs-log))
       (not (memq nil rs-gcs))
       (length rs-gcs))
    (setq post-gc-hook nil)))"#;

fn candidate(stress: bool) -> (String, u64) {
    let mut vm = Vm::new();
    vm.install_lisp_prelude().unwrap();
    if stress {
        vm.set_gc_stress(1);
    }
    vm.eval_source(SETUP).unwrap();
    let before = vm.heap_stats().collections;
    let printed = vm.eval_source(&format!("(prin1-to-string {CASE})")).unwrap().display().unwrap();
    (serde_json::from_str(&printed).unwrap(), vm.heap_stats().collections - before)
}

#[test]
fn a_search_survives_nested_searches_and_moving_collections() {
    let oracle = std::env::var("ROMACS_GNU_ORACLE").expect("pinned GNU oracle");
    let output = std::process::Command::new(oracle)
        .args(["-Q", "--batch", "--eval", SETUP, "--eval", &format!("(prin1 {CASE})")]).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let gnu = String::from_utf8(output.stdout).unwrap();
    // The searches' answers, the syntax table deciding `f-o' and `fo-b'
    // (`-' is `_' in the standard table), and all 38 callbacks collecting.
    let answers = r#"((("key1" "val1" t 10) ("key22" "val22" t 23) ("key333" "val333" t 38)) (2 44 666) ((("foo" "fob") ("foo" "fob") ("fob" "foo") ("fob" "foo") "fo") (("foo" "fob" "fax" "fib" "f-o" "fo-b") ("f-o" "fax" "fib" "fo-b" "fob" "foo"))) 76 "#;
    assert!(gnu.starts_with(answers) && gnu.ends_with(" t 38)"), "pinned GNU: {gnu}");
    let (plain, collected) = candidate(false);
    assert_eq!(plain, gnu, "candidate");
    let (stressed, stress_collected) = candidate(true);
    assert_eq!(stressed, gnu, "under moving stress");
    assert!(stress_collected > collected, "stress collected beyond the callbacks: {stress_collected} against {collected}");
}
