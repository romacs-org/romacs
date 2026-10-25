;;; romacs-real-ert-reporter.el --- Observer of GNU ERT batch execution

(require 'json)
(require 'nadvice)
(defvar ert-scoreboard-selector t)
(defvar ert-scoreboard--void-functions nil)
(defvar ert-scoreboard--observing nil)
(defun ert-scoreboard--audit-error (_describe condition type _exclude)
  (when (and (eq (car-safe condition) 'void-function)
             (not (memq 'void-function (if (listp type) type (list type)))))
    (push (symbol-name (cadr condition)) ert-scoreboard--void-functions)))
(defun ert-scoreboard--emit (value)
  ;; GNU's public wrapper localizes a C boolean only as a buffer optimization.
  ;; Use its serializer with inhibition scoped to this private output buffer.
  (let ((json-false :false))
    (message "ERT-SCOREBOARD %s"
             (with-output-to-string
               (with-current-buffer standard-output
                 (let ((inhibit-modification-hooks t))
                   (json--print value)))))))
(defun ert-scoreboard--result (test result)
  (ert-scoreboard--emit
   (list :event "result" :test (symbol-name (ert-test-name test))
         :outcome (cond ((ert-test-passed-p result) "passed")
                        ((ert-test-skipped-p result) "skipped")
                        ((ert-test-quit-p result) "quit")
                        ((ert-test-failed-p result) "failed")
                        (t "unknown"))
         :as_expected (if (ert-test-result-expected-p test result) t :false)
         :vacuous_void_function (vconcat (reverse ert-scoreboard--void-functions)))))
(defun ert-scoreboard--observe (original selector listener &rest args)
  (if ert-scoreboard--observing
      (apply original selector listener args)
    (let ((ert-scoreboard--observing t))
      (apply original selector
             (lambda (event &rest values)
               (cond
                ((eq event 'run-started)
                 (ert-scoreboard--emit
                  (list :event "selected"
                        :names (vconcat (mapcar (lambda (test)
                                                (symbol-name (ert-test-name test)))
                                              (ert--stats-tests (car values)))))))
                ((eq event 'test-started)
                 (setq ert-scoreboard--void-functions nil)
                 (ert-scoreboard--emit
                  (list :event "started" :test
                        (symbol-name (ert-test-name (nth 1 values))))))
                ((eq event 'test-ended)
                 (ert-scoreboard--result (nth 1 values) (nth 2 values))))
               (apply listener event values))
             args))))
(unwind-protect
    (progn
      (advice-add 'ert--should-error-handle-error :before #'ert-scoreboard--audit-error)
      (advice-add 'ert-run-tests :around #'ert-scoreboard--observe)
      (let* ((load-file-name nil) (load-true-file-name nil) (load-in-progress nil)
             (current-load-list nil) (ert-quiet nil)
             (ert-scoreboard--void-functions nil)
             (stats (ert-run-tests-batch ert-scoreboard-selector)))
        (when (zerop (ert-stats-total stats))
          (error "Real ERT selected no tests"))
        (ert-scoreboard--emit (list :event "done"))
        (if (zerop (ert-stats-completed-unexpected stats)) 0 1)))
  (advice-remove 'ert-run-tests #'ert-scoreboard--observe)
  (advice-remove 'ert--should-error-handle-error #'ert-scoreboard--audit-error))
