;;; where.el --- which file defines each journey command in GNU -*- lexical-binding: t -*-
;; emacs -Q --batch -l where.el.  "C" = no symbol-file (a C primitive).
(dolist (k '("C-x C-f" "C-x C-s" "C-n" "C-p" "C-f" "C-b" "C-a" "C-e" "M-<" "M->" "C-v" "M-v" "C-l"
             "a" "DEL" "C-k" "C-/" "C-x 2" "C-x 3" "C-x o" "C-x 1" "C-x b" "C-x C-b" "M-:"
             "C-x C-e" "M-x" "C-d" "C-x C-c"))
  (let* ((cmd (key-binding (kbd k))) (file (and (symbolp cmd) (symbol-file cmd 'defun))))
    (princ (format "global  %-8s %-30s %s\n" k cmd (or file "C")))))
(require 'org)
(with-temp-buffer
  (org-mode)
  (dolist (k '("C-a" "C-e" "DEL" "C-k" "TAB" "M-RET"))
    (let* ((cmd (key-binding (kbd k))) (file (and (symbolp cmd) (symbol-file cmd 'defun))))
      (princ (format "org     %-8s %-30s %s\n" k cmd (or file "C"))))))
(princ (format "auto-mode for .org: %s\n" (assoc-default "x.org" auto-mode-alist #'string-match)))
