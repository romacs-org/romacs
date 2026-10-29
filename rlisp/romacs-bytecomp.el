;;; romacs-bytecomp.el --- byte-compile of a function value  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; VM2 compiles every function when it is defined, so a function value is
;; already compiled code, and its closures are opaque (VM2-CLOSEQ1):
;; bytecomp's `byte-compile--reify-function' cannot read one back into
;; source.  `byte-compile' of a function value, or of a symbol whose
;; definition is one, therefore returns that function; a `(lambda ...)'
;; list is evaluated in the dialect GNU would compile it in, once
;; bytecomp's own front end (macroexpansion and closure conversion) has
;; accepted it: a body that front end refuses gives GNU's diagnostic
;; string, as `byte-compile' returns it, and no function.  Any other
;; argument, and file compilation, stay GNU's.  DIVERGENCES.md
;; VM2-BYTECOMP1.
;;
;; The advice is added before bytecomp.el loads: nadvice keeps it through
;; the autoload (`defalias-fset-function'), so bytecomp's own load-time
;; `byte-compile' calls already see it.

;;; Code:

(defun romacs-bytecomp--front-end (fun lexical)
  "Run `byte-compile''s front end on the `(lambda ...)' list FUN.
LEXICAL is the dialect it compiles in.  Return the diagnostic string
`byte-compile' returns when the front end signals, else nil.  The
macros are bytecomp's, so the form is evaluated once bytecomp is
loaded."
  (require 'bytecomp)
  (eval `(displaying-byte-compile-warnings
          (byte-compile-close-variables
           (let ((lexical-binding ,lexical))
             (byte-compile-preprocess ',fun)
             nil)))
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
      (let ((lexical (and (not (symbolp form)) lexical-binding)))
        (or (romacs-bytecomp--front-end fun lexical)
            (let ((value (funcall wrap (eval fun lexical))))
              (when (symbolp form) (fset form value))
              value))))
     (t (funcall compile form)))))

(advice-add 'byte-compile :around #'romacs-bytecomp--function-value)

(provide 'romacs-bytecomp)
;;; romacs-bytecomp.el ends here
