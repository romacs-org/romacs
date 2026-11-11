;;; romacs-mini-ert-audit.el --- what each should-error caught, for run.py  -*- lexical-binding: t -*-

;; NOT a harness and NOT GNU's ert.el.  The romacs-gnu-ert adapter loads this
;; right after `romacs-mini-ert.el' (mini mode, `--audit').  It wraps the
;; harness's `romacs-gnu-tests--should-error' WITHOUT changing what it does,
;; and records each should-error that was satisfied by `void-function',
;; `void-variable' or `invalid-function' when the test did not ask for that
;; condition.  Under the reduced harness those are how absent machinery reads
;; as a pass: (should-error (read "#<")) "passes" when `read' does not exist.
;;
;; test/gnu/run.py decides from these whether a romacs pass is real (pinned
;; GNU caught the same condition) or vacuous; this file only reports.  It is a
;; separate file so the harness, whose sha256 every GNU baseline is bound to,
;; stays byte-identical.  GNU's own side of the same guard is gnu-run.el's
;; `vacuous_void_function'.

(defvar romacs-gnu-tests--audit-caught nil
  "Absent-machinery catches of the running test, newest first: (CONDITION . DATUM).")

(defvar romacs-gnu-tests--audit-original (symbol-function 'romacs-gnu-tests--should-error)
  "The harness's own should-error, called unchanged.")

(defun romacs-gnu-tests--should-error (form thunk type exclude-subtypes)
  (let ((condition (funcall romacs-gnu-tests--audit-original form thunk type exclude-subtypes))
        (wanted (if (listp type) type (list type))))
    (if (and (memq (car condition) '(void-function void-variable invalid-function))
             (not (memq (car condition) wanted)))
        (setq romacs-gnu-tests--audit-caught
              (cons (cons (car condition) (car (cdr condition))) romacs-gnu-tests--audit-caught)))
    condition))

(defun romacs-gnu-tests--audit-take ()
  "Return this test's catches as \"CONDITION DATUM\" lines, oldest first, and reset.
DATUM is the symbol's name for a symbol, empty otherwise."
  (let ((lines "") (entries (reverse romacs-gnu-tests--audit-caught)))
    (setq romacs-gnu-tests--audit-caught nil)
    (while entries
      (let ((datum (cdr (car entries))))
        (setq lines (concat lines (symbol-name (car (car entries))) " "
                            (if (and datum (symbolp datum)) (symbol-name datum) "") "\n")))
      (setq entries (cdr entries)))
    lines))

;;; romacs-mini-ert-audit.el ends here
