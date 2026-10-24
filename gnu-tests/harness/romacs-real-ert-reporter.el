;;; romacs-real-ert-reporter.el --- Observer of unmodified GNU ERT
;; Same observer protocol as ert_report.py, but returns 0/1 to the Rust
;; embedder instead of asking Lisp to terminate the host process.
;; No summary or result is manufactured here: GNU owns both.

(require 'json)
(defvar ert-scoreboard--void-functions nil)
(defun ert-scoreboard--audit-error (_describe condition type _exclude)
  ;; An explicit void-function assertion tests that condition intentionally.
  ;; A broad should-error catching a missing implementation is not parity.
  (when (and (eq (car-safe condition) 'void-function)
             (not (memq 'void-function (if (listp type) type (list type)))))
    (push (symbol-name (cadr condition)) ert-scoreboard--void-functions)))
(advice-add 'ert--should-error-handle-error :before #'ert-scoreboard--audit-error)
(defun ert-scoreboard--emit (value)
  (message "ERT-SCOREBOARD %s" (json-serialize value)))
(defun ert-scoreboard--observe (original selector listener &rest args)
  (apply original selector
         (lambda (event &rest values)
           (when (eq event 'test-started) (setq ert-scoreboard--void-functions nil))
           (apply listener event values)
           (when (eq event 'test-ended)
             (let* ((test (nth 1 values)) (result (nth 2 values))
                    (outcome (cond ((ert-test-passed-p result) "passed")
                                   ((ert-test-skipped-p result) "skipped")
                                   ((ert-test-quit-p result) "quit")
                                   ((ert-test-failed-p result) "failed")
                                   (t "unknown"))))
               (ert-scoreboard--emit
                (list :event "result" :test (symbol-name (ert-test-name test))
                      :outcome outcome :as_expected
                      (if (ert-test-result-expected-p test result) t :false)
                      :vacuous_void_function (vconcat (reverse ert-scoreboard--void-functions)))))))
         args))
(advice-add 'ert-run-tests :around #'ert-scoreboard--observe)
(ert-scoreboard--emit
 (list :event "registered"
       :names (vconcat (mapcar (lambda (test) (symbol-name (ert-test-name test)))
                              (ert-select-tests t t)))))
(let* ((load-file-name nil) (load-true-file-name nil) (load-in-progress nil)
       (current-load-list nil) (ert-quiet nil)
       (stats (ert-run-tests-batch t)))
  (ert-scoreboard--emit (list :event "done"))
  (if (zerop (ert-stats-completed-unexpected stats)) 0 1))
