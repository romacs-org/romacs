;;; gnu_driver.el --- batch GNU oracle for generated primitive cases  -*- lexical-binding: t -*-
;; Reads PRIMH_CASES (JSON array of {"id","source"}), evaluates every case
;; BYTE-COMPILED (matching VM2's compiled regime) in this one process, and
;; prints one JSON object per line.  One process is sound only for primitives
;; declared `pure'; primh.py starts a fresh process per case otherwise.
(setq byte-compile-warnings nil)
(set-language-environment "UTF-8")
(set-terminal-coding-system 'utf-8-unix)
(set-keyboard-coding-system 'utf-8-unix)
;; `message' goes to stderr and is flushed per line in batch, so a case on
;; which GNU itself never returns leaves a parseable prefix behind.
(defun primh-out (x) (message "%s" (decode-coding-string (json-serialize x) 'utf-8)))
(let* ((cases (with-temp-buffer
                (insert-file-contents (getenv "PRIMH_CASES"))
                (json-parse-buffer :object-type 'alist :array-type 'list)))
       (head (list (cons 'version emacs-version)
                   (cons 'revision (or emacs-repository-version ""))
                   (cons 'subrs
                         (vconcat (mapcar (lambda (name)
                                   (let* ((f (symbol-function (intern name)))
                                          (a (and (subrp f) (subr-arity f))))
                                     (vector name (if (subrp f) "subr" "lisp")
                                           (if a (car a) -1)
                                           (cond ((null a) -1)
                                                 ((eq (cdr a) 'many) "many")
                                                 ((eq (cdr a) 'unevalled) "unevalled")
                                                 (t (cdr a))))))
                                 (split-string (getenv "PRIMH_NAMES") ",")))))))
  (primh-out head)
  (dolist (c cases)
    (let* ((source (alist-get 'source c))
           (out (condition-case e
                    (funcall (byte-compile `(lambda () ,(car (read-from-string source)))))
                  (error (format "HARNESS-ERROR %S" e))))
           ;; The SUBR regime: interpreted eval calls the C DEFUN itself, while
           ;; byte-compiled code may take bytecode.c fast paths or byte-opt
           ;; rewrites (e.g. Bnth, bytecode.c:1045) that signal different data.
           (interp (condition-case e (eval (car (read-from-string source)) t)
                     (error (format "HARNESS-ERROR %S" e)))))
      (primh-out (list (cons 'id (alist-get 'id c))
                       (cons 'expected out) (cons 'interp interp))))))
