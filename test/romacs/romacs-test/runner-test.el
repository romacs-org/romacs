;;; runner-test.el --- Self-tests for the small Romacs runner -*- lexical-binding: t; -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; Load with romacs-test on load-path, then run the registered tests normally.
;; Nested runs deliberately report failures; their enclosing tests verify them.

;;; Code:

(require 'romacs-test)

(romacs-deftest romacs-runner-assertions ()
  "Exercise successful assertions and typed error ancestry."
  (romacs-should (= 2 (+ 1 1)))
  (romacs-should-not nil)
  (romacs-should-error (car 1) :type 'wrong-type-argument)
  (romacs-should-error (car 1) :type 'error)
  (romacs-should-error (error "expected")))

(romacs-deftest romacs-runner-single-evaluation ()
  (let ((calls 0))
    (romacs-should (progn (setq calls (1+ calls)) t))
    (romacs-should (= calls 1))
    (romacs-should-not (progn (setq calls (1+ calls)) nil))
    (romacs-should (= calls 2))
    (romacs-should-error (progn (setq calls (1+ calls)) (error "once")))
    (romacs-should (= calls 3))
    (romacs-should-error
     (romacs-should (progn (setq calls (1+ calls)) nil))
     :type 'romacs-test-failed)
    (romacs-should (= calls 4))
    (romacs-should-error
     (romacs-should-not (progn (setq calls (1+ calls)) t))
     :type 'romacs-test-failed)
    (romacs-should (= calls 5))
    (romacs-should-error
     (romacs-should-error (progn (setq calls (1+ calls)) 42))
     :type 'romacs-test-failed)
    (romacs-should (= calls 6))
    (romacs-should-error
     (romacs-should-error
      (progn (setq calls (1+ calls)) (error "wrong type"))
      :type 'wrong-type-argument)
     :type 'romacs-test-failed)
    (romacs-should (= calls 7))))

(romacs-deftest romacs-runner-diagnostics ()
  (let ((caught (condition-case condition
                    (romacs-should-not (+ 1 2))
                  (romacs-test-failed condition))))
    (romacs-should (eq (car caught) 'romacs-test-failed))
    (romacs-should (equal (plist-get (cdr caught) :form) '(+ 1 2)))
    (romacs-should (= (plist-get (cdr caught) :value) 3)))
  (romacs-should-error (romacs-should-error 17) :type 'romacs-test-failed)
  (romacs-should-error
   (romacs-should-error (car 1) :type 'void-variable)
   :type 'romacs-test-failed))

(romacs-deftest romacs-runner-condition-hierarchy ()
  (romacs-should
   (equal (get 'romacs-test-failed 'error-conditions)
          '(romacs-test-failed error)))
  (romacs-should-error (romacs-should nil) :type 'error))

(romacs-deftest romacs-runner-outcomes-and-continuation ()
  (let ((romacs-test--registry nil)
        (romacs-test-last-summary nil)
        (visited nil))
    (romacs-deftest local-pass () (setq visited (cons 'pass visited)))
    (romacs-deftest local-fail () (romacs-should nil))
    (romacs-deftest local-error () (error "ordinary body error"))
    (romacs-deftest local-quit () (signal 'quit nil))
    (romacs-deftest local-last () (setq visited (cons 'last visited)))
    (let ((summary (romacs-test-run t)))
      (romacs-should (= (plist-get summary :total) 5))
      (romacs-should (= (plist-get summary :passed) 2))
      (romacs-should (= (plist-get summary :failed) 1))
      (romacs-should (= (plist-get summary :errors) 2))
      (romacs-should (eq summary romacs-test-last-summary))
      (romacs-should (equal visited '(last pass)))
      (romacs-should
       (equal (let ((statuses nil))
         (dolist (result (plist-get summary :results))
           (setq statuses (cons (plist-get result :status) statuses)))
         (nreverse statuses))
              '(PASS FAIL ERROR ERROR PASS))))))

(romacs-deftest romacs-runner-redefinition-and-closure ()
  (let ((romacs-test--registry nil)
        (romacs-test-last-summary nil)
        (value 7) (seen nil))
    (romacs-deftest local-first () "Old definition." (error "replaced"))
    (romacs-deftest local-second () (setq seen (cons 'second seen)))
    (romacs-deftest local-first () "Replacement." (setq seen (cons value seen)))
    (setq value 9)
    (let ((summary (romacs-test-run t)))
      (romacs-should (= (plist-get summary :total) 2))
      (romacs-should (= (plist-get summary :passed) 2))
      (romacs-should (equal seen '(second 9)))
      (romacs-should (equal (car (cdr (car romacs-test--registry))) "Replacement.")))))

(romacs-deftest romacs-runner-selectors ()
  (let ((romacs-test--registry nil)
        (romacs-test-last-summary nil))
    (romacs-deftest local-alpha () t)
    (romacs-deftest local-beta () t)
    (romacs-deftest other-gamma () t)
    (romacs-should (= (plist-get (romacs-test-run t) :total) 3))
    (romacs-should (= (plist-get (romacs-test-run 'local-beta) :total) 1))
    (romacs-should (= (plist-get (romacs-test-run "^local-") :total) 2))
    (let ((summary (romacs-test-run '(member other-gamma local-alpha local-alpha))))
      (romacs-should (= (plist-get summary :total) 2))
      (romacs-should
       (equal (let ((names nil))
         (dolist (result (plist-get summary :results))
           (setq names (cons (plist-get result :name) names)))
         (nreverse names))
              '(local-alpha other-gamma))))))

(romacs-deftest romacs-runner-invalid-selection ()
  (let ((romacs-test--registry nil)
        (romacs-test-last-summary 'stale))
    (romacs-should-error (romacs-test-run t))
    (romacs-should-not romacs-test-last-summary)
    (romacs-deftest local-only () t)
    (romacs-should-error (romacs-test-run 'missing))
    (romacs-should-error (romacs-test-run "no-match"))
    (romacs-should-error (romacs-test-run nil))
    (romacs-should-error (romacs-test-run '(member)))
    (romacs-should-error (romacs-test-run '(member local-only missing)))
    (romacs-should-error (romacs-test-run '(or local-only)))
    (romacs-should-error (romacs-test-run 42))
    (romacs-should-error (romacs-test-run "["))
    (romacs-should (= (plist-get (romacs-test-run t) :passed) 1))))

(romacs-deftest romacs-runner-invalid-macros ()
  (romacs-should-error (macroexpand '(romacs-deftest broken (arg) t)))
  (romacs-should-error (macroexpand '(romacs-deftest nil () t)))
  (romacs-should-error (macroexpand '(romacs-should-error t :unknown t)))
  (romacs-should-error (macroexpand '(romacs-should-error t :type)))
  (romacs-should-error (macroexpand '(romacs-should-error t :type 'error :other t)))
  (romacs-should-error (macroexpand '(romacs-should-error t :type error))))

(provide 'romacs-runner-test)
;;; runner-test.el ends here
