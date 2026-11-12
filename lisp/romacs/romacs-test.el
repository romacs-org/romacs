;;; romacs-test.el --- Small source-native Romacs tests -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:

;; A deliberately small test API, not an ERT implementation.  Tests are source
;; closures in definition order.  The runner returns a summary for batch hosts.

;;; Code:

(put 'romacs-test-failed 'error-conditions '(romacs-test-failed error))
(put 'romacs-test-failed 'error-message "Romacs test assertion failed")

(defvar romacs-test--registry nil
  "List of (NAME DOCSTRING CLOSURE) entries in definition order.")

(defvar romacs-test-last-summary nil
  "Summary plist from the last completed `romacs-test-run'.")

(defun romacs-test--register (name doc body)
  "Register NAME with DOC and BODY, replacing an existing case in place."
  (let ((entry (list name doc body))
        (entries nil) (replaced nil))
    (dolist (old romacs-test--registry)
      (if (eq (car old) name)
          (progn (setq entries (cons entry entries)) (setq replaced t))
        (setq entries (cons old entries))))
    (setq romacs-test--registry
          (if replaced (nreverse entries)
            (append romacs-test--registry (list entry))))
    name))

(defmacro romacs-deftest (name args &rest body)
  "Define NAME with empty ARGS and optional docstring followed by BODY.
Capture BODY in its source lexical environment; do not define a function NAME."
  (declare (indent 2) (debug (symbolp () [&optional stringp] body)))
  (unless (and (symbolp name) name (not (eq name t)) (null args))
    (error "Expected (romacs-deftest NAME () [DOCSTRING] BODY...)"))
  (let ((doc (if (stringp (car body)) (car body) nil)))
    (when doc (setq body (cdr body)))
    `(romacs-test--register ',name ,doc (lambda () ,@body))))

(defmacro romacs-should (form)
  "Assert that FORM evaluates non-nil, evaluating it exactly once."
  (let ((value (make-symbol "romacs-test-value")))
    `(let ((,value ,form))
       (unless ,value
         (signal 'romacs-test-failed
                 (list :form ',form :value ,value :expected 'non-nil)))
       ,value)))

(defmacro romacs-should-not (form)
  "Assert that FORM evaluates nil, evaluating it exactly once."
  (let ((value (make-symbol "romacs-test-value")))
    `(let ((,value ,form))
       (when ,value
         (signal 'romacs-test-failed
                 (list :form ',form :value ,value :expected nil)))
       ,value)))

(defun romacs-test--should-error (thunk form type)
  "Assert that THUNK for FORM signals an error belonging to TYPE."
  (let ((outcome (condition-case condition
                     (list 'returned (funcall thunk))
                   (error (list 'signaled condition)))))
    (if (and (eq (car outcome) 'signaled)
             (memq type (get (car (car (cdr outcome))) 'error-conditions)))
        (car (cdr outcome))
      (signal 'romacs-test-failed
              (list :form form :expected type
                    (if (eq (car outcome) 'signaled) :condition :value)
                    (car (cdr outcome)))))))

(defmacro romacs-should-error (form &rest options)
  "Assert that FORM signals an error, evaluating it exactly once.
The only optional syntax is :type 'SYMBOL; the default type is `error'.
A quit is not an ordinary error and does not satisfy this assertion."
  (unless (or (null options)
              (and (= (length options) 2) (eq (car options) :type)
                   (consp (car (cdr options)))
                   (eq (car (car (cdr options))) 'quote)
                   (= (length (car (cdr options))) 2)
                   (symbolp (car (cdr (car (cdr options)))))
                   (car (cdr (car (cdr options))))))
    (error "romacs-should-error accepts only :type 'SYMBOL"))
  `(romacs-test--should-error (lambda () ,form) ',form
                             ,(if options (car (cdr options)) ''error)))

(defun romacs-test--select (selector)
  "Return registered entries matching SELECTOR, or signal on invalid input."
  (let ((selected
         (cond
          ((eq selector t) romacs-test--registry)
          ((and (symbolp selector) selector)
           (let ((entry (assq selector romacs-test--registry)))
             (unless entry (error "Unknown Romacs test: %S" selector))
             (list entry)))
          ((stringp selector)
           (let ((matches nil))
             (dolist (entry romacs-test--registry)
               (when (string-match selector (symbol-name (car entry)))
                 (setq matches (cons entry matches))))
             (nreverse matches)))
          ((and (consp selector) (eq (car selector) 'member))
           (dolist (name (cdr selector))
             (unless (and (symbolp name) (assq name romacs-test--registry))
               (error "Unknown Romacs test in member selector: %S" name)))
           (let ((matches nil))
             (dolist (entry romacs-test--registry)
               (when (memq (car entry) (cdr selector))
                 (setq matches (cons entry matches))))
             (nreverse matches)))
          (t (error "Unsupported Romacs test selector: %S" selector)))))
    (unless selected (error "Romacs test selector matched no tests: %S" selector))
    selected))

(defun romacs-test-run (selector)
  "Run SELECTOR and return a summary plist, also in `romacs-test-last-summary'.
SELECTOR is t, a registered name, a regexp string, or (member NAME...).
Selection follows definition order.  Invalid or empty selection signals error.
Attempt every selected body, reporting PASS, FAIL (assertion), or ERROR
(including quit).  Return :total, :passed, :failed, :errors and :results;
each result has :name, :status and :condition.  Batch hosts must exit nonzero
when :failed or :errors is nonzero.  There are no skips or expected failures."
  (setq romacs-test-last-summary nil)
  (let ((selected (romacs-test--select selector))
        (passed 0) (failed 0) (errors 0) (results nil))
    (dolist (entry selected)
      (let* ((condition nil)
             (status
              (condition-case caught
                  (progn (funcall (nth 2 entry)) 'PASS)
                (romacs-test-failed (setq condition caught) 'FAIL)
                (error (setq condition caught) 'ERROR)
                (quit (setq condition caught) 'ERROR))))
        (cond ((eq status 'PASS) (setq passed (1+ passed)))
              ((eq status 'FAIL) (setq failed (1+ failed)))
              (t (setq errors (1+ errors))))
        (setq results (cons (list :name (car entry) :status status
                                  :condition condition) results))
        (princ (format "%s %s%s\n" status (car entry)
                       (if condition (format " %S" condition) "")))))
    (setq romacs-test-last-summary
          (list :total (length selected) :passed passed :failed failed
                :errors errors :results (nreverse results)))
    (princ (format "SUMMARY total=%d passed=%d failed=%d errors=%d\n"
                   (length selected) passed failed errors))
    romacs-test-last-summary))

(provide 'romacs-test)
;;; romacs-test.el ends here
