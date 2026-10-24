;;; face-test-jump-sample.el --- Jump to the mixed-width face sample -*- lexical-binding: t; -*-

(defun romacs-face-test-jump-sample ()
  "Jump the face test window to the line containing the \"a好好b\" sample."
  (interactive)
  (let ((buf (get-buffer "*Romacs Face Test*"))
        (win (get-buffer-window "*Romacs Face Test*" 0)))
    (unless (and buf win)
      (user-error "Face test buffer/window is not ready"))
    (with-current-buffer buf
      (goto-char (point-min))
      (unless (search-forward "a好好b" nil t)
        (user-error "Could not find mixed-width sample"))
      (beginning-of-line)
      (set-window-point win (point)))
    (redisplay t)))

(defvar romacs-face-test-jump-sample--armed nil
  "Non-nil once the jump helper has been armed.")

(defun romacs-face-test-jump-sample-arm ()
  "Schedule one jump to the mixed-width sample after window setup."
  (unless romacs-face-test-jump-sample--armed
    (setq romacs-face-test-jump-sample--armed t)
    (add-hook 'window-setup-hook #'romacs-face-test-jump-sample)
    (add-hook 'emacs-startup-hook #'romacs-face-test-jump-sample)
    (when after-init-time
      (run-with-timer 0.25 nil #'romacs-face-test-jump-sample))))

(provide 'face-test-jump-sample)

;;; face-test-jump-sample.el ends here
