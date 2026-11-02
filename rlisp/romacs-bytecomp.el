;;; romacs-bytecomp.el --- byte-compile of a function value  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; VM2 compiles every function when it is defined, so a function value is
;; already compiled code, and its closures are opaque (VM2-CLOSEQ1):
;; bytecomp's `byte-compile--reify-function' cannot read one back into
;; source.  `byte-compile' of a function value, or of a symbol whose
;; definition is one, therefore returns that function; a `(lambda ...)'
;; list goes through bytecomp's own front end (macroexpansion, then
;; closure conversion as a check) and its expansion is evaluated in the
;; dialect GNU would compile it in, so each macro expands once; a body
;; that front end refuses gives GNU's diagnostic string, as
;; `byte-compile' returns it, and no function.  Any other
;; argument, and file compilation, stay GNU's.  DIVERGENCES.md
;; VM2-BYTECOMP1.
;;
;; The advice is added before bytecomp.el loads: nadvice keeps it through
;; the autoload (`defalias-fset-function'), so bytecomp's own load-time
;; `byte-compile' calls already see it.

;;; Code:

(defun romacs-bytecomp--eval-when-compile (&rest body)
  "Bytecomp's `eval-when-compile' handler for the front end below.
GNU's compiles each of BODY's top-level forms to byte code and runs it
(`byte-compile-eval'), which VM2 cannot (VM2-BYTECODE1); evaluate each
form's expansion instead, in turn and in the dialect being compiled, and
give the last value quoted, as GNU does."
  (let ((result nil))
    (byte-compile-recurse-toplevel
     (macroexp-progn body)
     (lambda (form)
       (setq result (eval (macroexpand-all form macroexpand-all-environment) lexical-binding))))
    (list 'quote result)))

(defun romacs-bytecomp--front-end (fun lexical)
  "Run `byte-compile''s front end on the `(lambda ...)' list FUN.
LEXICAL is the dialect it compiles in.  Return FUN macroexpanded, or
the diagnostic string `byte-compile' returns when the front end
signals.  `byte-compile-preprocess''s two steps run separately: its
closure conversion yields bytecomp's internal forms, so it only
checks.  The macros are bytecomp's, so the form is evaluated once
bytecomp is loaded."
  (require 'bytecomp)
  (eval `(displaying-byte-compile-warnings
          (byte-compile-close-variables
           (let* ((lexical-binding ,lexical)
                  (expanded (macroexpand-all
                             ',fun (cons '(eval-when-compile . romacs-bytecomp--eval-when-compile)
                                         byte-compile-macro-environment))))
             (cconv-closure-convert expanded byte-compile-bound-variables)
             expanded)))
        t))

(defun romacs-bytecomp--function-value (compile form)
  "Return FORM's function when it is already VM2 code, else call COMPILE.
FORM is `byte-compile''s argument: a symbol, a function value, a
`(lambda ...)' list or `(macro . FUNCTION)'."
  (let* ((fun (if (symbolp form) (symbol-function form) form))
         (macro (eq (car-safe fun) 'macro))
         (fun (if macro (cdr fun) fun))
         (wrap (lambda (fun) (if macro (cons 'macro fun) fun))))
    (cond
     ((interpreted-function-p fun) (funcall wrap fun))
     ((eq (car-safe fun) 'lambda)
      ;; bytecomp.el:2986: a symbol's list definition is dynamic code; a
      ;; list argument compiles in the caller's `lexical-binding'.
      (let* ((lexical (and (not (symbolp form)) lexical-binding))
             (expanded (romacs-bytecomp--front-end fun lexical)))
        (if (stringp expanded) expanded
          (let ((value (funcall wrap (eval expanded lexical))))
            (when (symbolp form) (fset form value))
            value))))
     (t (funcall compile form)))))

(advice-add 'byte-compile :around #'romacs-bytecomp--function-value)

(provide 'romacs-bytecomp)
;;; romacs-bytecomp.el ends here
