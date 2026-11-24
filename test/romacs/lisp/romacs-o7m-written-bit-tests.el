;;; romacs-o7m-written-bit-tests.el --- O7m C4 written bits and registrations -*- lexical-binding: t; -*-

(require 'ert)
(require 'custom)

(defvar romacs--after-load-registrations)

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

;; S7. Keep one record when GNU keeps one file-key registration.
(ert-deftest romacs-o7m-bit-tests-duplicate-registration-is-kept-once ()
  (let ((after-load-alist nil)
        (romacs--after-load-registrations nil)
        (callback (lambda () (error "Unloaded fixture unexpectedly ran")))
        (key "o7m-written-bit-unloaded-never.el"))
    (eval-after-load key callback)
    (eval-after-load key callback)
    (should (equal (cdar after-load-alist) (list callback)))
    (should (= (length romacs--after-load-registrations) 1))))

;; S7. Retain an unrelated successful registration when the immediate
;; callback signals, and leave no record for that failed callback.
(ert-deftest romacs-o7m-bit-tests-immediate-signal-leaves-no-registration ()
  (let ((after-load-alist nil)
        (romacs--after-load-registrations nil)
        (features (copy-sequence features))
        (ran nil))
    (provide 'romacs-o7m-bit-already-loaded)
    (eval-after-load "o7m-written-bit-control.el" #'ignore)
    (let ((before (copy-tree romacs--after-load-registrations)))
      (should-error
       (eval-after-load 'romacs-o7m-bit-already-loaded
         (lambda ()
           (setq ran t)
           (error "O7m immediate callback")))
       :type 'error)
      (should ran)
      (should (equal romacs--after-load-registrations before))
      (should (= (length before) 1)))))

;;; romacs-o7m-written-bit-tests.el ends here
