;;; romacs-buffer-switch-tests.el --- Dedicated window switching -*- lexical-binding: t; -*-

(require 'ert)

(when (fboundp 'romacs--subr-effect)
  (load "romacs-buffer-switch" nil t))

(defmacro romacs-buffer-switch-test--with-buffers (&rest body)
  "Run BODY with SOURCE displayed in WINDOW and a disposable TARGET."
  (declare (indent 0) (debug t))
  `(with-temp-buffer
     (let ((source (current-buffer)))
       (with-temp-buffer
         (let* ((target (current-buffer))
                (window (selected-window))
                (switch-to-buffer-obey-display-actions nil)
                (read-buffer-function (lambda (&rest _) (buffer-name target))))
           (save-window-excursion
             (set-window-buffer window source)
             (set-buffer source)
             ,@body))))))

(defun romacs-buffer-switch-test--interactive ()
  "Switch interactively, returning any unexpected error for the assertion."
  (condition-case err
      (call-interactively #'switch-to-buffer)
    (error err)))

(ert-deftest romacs-buffer-switch-dedicated-nil ()
  (romacs-buffer-switch-test--with-buffers
    (let ((switch-to-buffer-in-dedicated-window nil))
      (set-window-dedicated-p window t)
      (should-error (call-interactively #'switch-to-buffer) :type 'user-error)
      (should (eq (selected-window) window))
      (should (eq (window-buffer window) source))
      (should (eq (window-dedicated-p window) t)))))

(ert-deftest romacs-buffer-switch-dedicated-pop ()
  (romacs-buffer-switch-test--with-buffers
    (let ((alternate (split-window window nil 'below))
          (switch-to-buffer-in-dedicated-window 'pop)
          (display-buffer-overriding-action '((display-buffer-reuse-window))))
      (set-window-buffer alternate target)
      (set-window-dedicated-p window t)
      (should (eq (romacs-buffer-switch-test--interactive) target))
      (should (eq (selected-window) alternate))
      (should (eq (window-buffer window) source))
      (should (eq (window-dedicated-p window) t)))))

(ert-deftest romacs-buffer-switch-dedicated-t ()
  (romacs-buffer-switch-test--with-buffers
    (let ((switch-to-buffer-in-dedicated-window t))
      (set-window-dedicated-p window t)
      (should (eq (romacs-buffer-switch-test--interactive) target))
      (should (eq (selected-window) window))
      (should (eq (window-buffer window) target))
      (should-not (window-dedicated-p window)))))

(ert-deftest romacs-buffer-switch-nondedicated ()
  (romacs-buffer-switch-test--with-buffers
    (let ((switch-to-buffer-in-dedicated-window nil))
      (should-not (window-dedicated-p window))
      (should (eq (romacs-buffer-switch-test--interactive) target))
      (should (eq (selected-window) window))
      (should (eq (window-buffer window) target)))))

(ert-deftest romacs-buffer-switch-weakly-dedicated ()
  (romacs-buffer-switch-test--with-buffers
    (let ((switch-to-buffer-in-dedicated-window nil))
      (set-window-dedicated-p window 'weak)
      (should (eq (romacs-buffer-switch-test--interactive) target))
      (should (eq (selected-window) window))
      (should (eq (window-buffer window) target))
      (should-not (window-dedicated-p window)))))

(ert-deftest romacs-buffer-switch-dedicated-force-same-window ()
  (romacs-buffer-switch-test--with-buffers
    (let ((switch-to-buffer-in-dedicated-window t))
      (set-window-dedicated-p window t)
      (should-error (switch-to-buffer target nil t) :type 'user-error)
      (should (eq (selected-window) window))
      (should (eq (window-buffer window) source))
      (should (eq (window-dedicated-p window) t)))))

;;; romacs-buffer-switch-tests.el ends here
