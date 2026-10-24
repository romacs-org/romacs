;;; gnu-primitives.el --- list GNU's C primitives (subr-primitive-p) -*- lexical-binding: t -*-
(let (names)
  (mapatoms (lambda (s) (when (and (fboundp s) (subr-primitive-p (symbol-function s)))
                          (push (symbol-name s) names))))
  (setq names (sort names #'string<))
  (with-temp-file (car command-line-args-left)
    (dolist (n names) (insert n "\n")))
  (setq command-line-args-left nil))
