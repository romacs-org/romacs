;;; -*- lexical-binding: t; -*-
(require 'bytecomp)
(defconst lr-scope-file (concat lr-dir "generated-constructor.el"))
(with-temp-file lr-scope-file
 (insert ";;; -*- lexical-binding: t; -*-\n(defmacro lr-generated () '(progn (defun lr-constructor () 42) (eval-and-compile (setq lr-answer (lr-constructor)))))\n(lr-generated)\n"))
(setq lr-answer nil)
(princ (format "source-load=%S answer=%S\n" (condition-case e (load lr-scope-file nil t t) (error e)) lr-answer))
(fmakunbound 'lr-constructor)
(fmakunbound 'lr-generated)
(setq lr-answer nil)
(princ (format "byte-compile=%S answer=%S constructor-bound=%S\n" (condition-case e (byte-compile-file lr-scope-file) (error e)) lr-answer (fboundp 'lr-constructor)))
(dolist (fn '(eval-buffer eval-region))
 (fset 'lr-m '(macro lambda nil 1))
 (setq lr-answer nil)
 (with-temp-buffer
  (insert ";;; -*- lexical-binding: t; -*-\n(progn (defmacro lr-m () 2) (setq lr-answer (lr-m)))")
  (princ (format "%S return=%S answer=%S\n" fn (if (eq fn 'eval-buffer) (eval-buffer) (eval-region (point-min) (point-max))) lr-answer))))
