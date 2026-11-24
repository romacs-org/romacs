;;; romacs-buffer-switch-parity-tests.el --- Switch and display actions -*- lexical-binding: t; -*-

(require 'ert)
(load "romacs-buffer-switch-tests" nil t)

(ert-deftest romacs-buffer-switch-pop-updates-default ()
  (romacs-buffer-switch-test--with-buffers
    (let ((prior (generate-new-buffer "switch-prior"))
          (alternate (split-window window nil 'below))
          (display-buffer-overriding-action '((display-buffer-reuse-window))))
      (unwind-protect
          (progn
            (set-window-buffer window prior)
            (switch-to-buffer source)
            (set-window-buffer alternate target)
            (set-window-dedicated-p window t)
            (switch-to-buffer target)
            (should (eq (selected-window) alternate))
            (let ((default (if (fboundp 'romacs-buffer-default) source (other-buffer))))
              (should (eq (switch-to-buffer nil) default))))
        (kill-buffer prior)))))

(ert-deftest romacs-buffer-switch-obeys-display-actions ()
  (romacs-buffer-switch-test--with-buffers
    (let ((switch-to-buffer-in-dedicated-window nil)
          (switch-to-buffer-obey-display-actions t)
          (display-buffer-overriding-action '((display-buffer-pop-up-window))))
      (delete-other-windows)
      (set-window-dedicated-p window t)
      (should (eq (call-interactively #'switch-to-buffer) target))
      (should (eq (window-buffer (selected-window)) target))
      (should-not (eq (selected-window) window))
      (should (eq (window-buffer window) source))
      (should (eq (window-dedicated-p window) t)))))

(ert-deftest romacs-buffer-switch-pop-records-like-gnu ()
  (romacs-buffer-switch-test--with-buffers
    (let ((alternate (split-window window nil 'below))
          (display-buffer-overriding-action '((display-buffer-reuse-window))))
      (set-window-buffer alternate target)
      (select-window window)
      (should-not (eq (car (buffer-list)) target))
      (set-window-dedicated-p window t)
      (switch-to-buffer target t)
      (should (eq (selected-window) alternate))
      (should (eq (car (buffer-list)) target)))))

(ert-deftest romacs-buffer-switch-pop-preserves-other-dedicated-windows ()
  (romacs-buffer-switch-test--with-buffers
    (let ((weak-buffer (generate-new-buffer "switch-weak"))
          (alternate (split-window window nil 'below))
          (split-height-threshold 0)
          (split-width-threshold nil))
      (unwind-protect
          (progn
            (set-window-buffer alternate weak-buffer)
            (set-window-dedicated-p alternate 'weak)
            (set-window-dedicated-p window t)
            (if (fboundp 'romacs-buffer--workspace-display)
                (cl-letf (((symbol-function 'romacs-gui-split)
                           (lambda (_horizontal anchor) (split-window anchor))))
                  (romacs-buffer--workspace-display target nil))
              (switch-to-buffer target))
            (should (eq (window-buffer (selected-window)) target))
            (should (eq (window-buffer alternate) weak-buffer))
            (should (eq (window-dedicated-p alternate) 'weak))
            (should (eq (window-buffer window) source))
            (should (eq (window-dedicated-p window) t)))
        (kill-buffer weak-buffer)))))

(ert-deftest romacs-buffer-switch-keeps-point-when-selection-edits-buffer ()
  (romacs-buffer-switch-test--with-buffers
    (let* ((target (generate-new-buffer "switch-hook-target"))
           (edited nil)
           (buffer-list-update-hook
            (list (lambda ()
                    (when (and (not edited) (eq (window-buffer window) target))
                      (setq edited t)
                      (with-current-buffer target
                        (save-excursion
                          (goto-char (point-min))
                          (insert "prefix"))))))))
      (unwind-protect
          (progn
            (with-current-buffer target
              (insert "abcd")
              (goto-char 3))
            (switch-to-buffer target)
            (should edited)
            (should (eq (char-after) ?c))
            (should (= (window-point window) 9)))
        (kill-buffer target)))))

;;; romacs-buffer-switch-parity-tests.el ends here
