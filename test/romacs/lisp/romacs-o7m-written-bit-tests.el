;;; romacs-o7m-written-bit-tests.el --- O7m C4 written bits and registrations -*- lexical-binding: t; -*-

(require 'ert)
(require 'custom)

(defun romacs-o7m-bit-tests--state (symbol)
  "Read SYMBOL's written bit, retaining a missing primitive's condition."
  (condition-case err
      (romacs--written-p symbol)
    (void-function err)))

(ert-deftest romacs-o7m-bit-tests-defcustom-sets-written ()
  (let ((before (romacs-o7m-bit-tests--state 'romacs-o7m-bit-option)))
    (eval '(defcustom romacs-o7m-bit-option 42
             "Option used by the O7m written-bit test."
             :type 'integer :group 'environment) t)
    (should (= (symbol-value 'romacs-o7m-bit-option) 42))
    (should (equal (list before (romacs-o7m-bit-tests--state 'romacs-o7m-bit-option))
                   '(nil t)))))

(ert-deftest romacs-o7m-bit-tests-timer-sets-written ()
  (let* ((before (romacs-o7m-bit-tests--state 'romacs-o7m-bit-timer))
         (timer (run-at-time 0 nil
                            (lambda () (set 'romacs-o7m-bit-timer 42)))))
    (unwind-protect
        (progn
          (accept-process-output nil 0.05)
          (should (= (symbol-value 'romacs-o7m-bit-timer) 42))
          (should (equal (list before (romacs-o7m-bit-tests--state 'romacs-o7m-bit-timer))
                         '(nil t))))
      (cancel-timer timer))))

(ert-deftest romacs-o7m-bit-tests-filter-sets-written ()
  (let* ((before (romacs-o7m-bit-tests--state 'romacs-o7m-bit-filter))
         (process (make-process
                   :name "o7m-written-filter" :noquery t :connection-type 'pipe
                   :command '("/bin/echo" "o7m-output")
                   :filter (lambda (_process output)
                             (set 'romacs-o7m-bit-filter output)))))
    (unwind-protect
        (progn
          (accept-process-output process 3)
          (should (equal (symbol-value 'romacs-o7m-bit-filter) "o7m-output\n"))
          (should (equal (list before (romacs-o7m-bit-tests--state 'romacs-o7m-bit-filter))
                         '(nil t))))
      (delete-process process))))

;;; romacs-o7m-written-bit-tests.el ends here
