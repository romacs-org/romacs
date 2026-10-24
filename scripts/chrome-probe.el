;;; chrome-probe.el --- Deterministic GUI chrome probe -*- lexical-binding: t; -*-

(defvar romacs-chrome-probe-ready-file
  (getenv "ROMACS_CHROME_PROBE_READY_FILE")
  "Optional file path written after the chrome probe has been applied.")

(defvar romacs-chrome-probe-header "Header sample"
  "Header-line text used by the GUI chrome probe.")

(defvar romacs-chrome-probe--armed nil
  "Non-nil once the GUI chrome probe has been armed.")

(defun romacs-chrome-probe--write-ready ()
  "Write a small readiness report when `romacs-chrome-probe-ready-file' is set."
  (when romacs-chrome-probe-ready-file
    (let ((frame (selected-frame)))
      (with-temp-file romacs-chrome-probe-ready-file
        (prin1
         (list :frame-width (frame-pixel-width frame)
               :frame-height (frame-pixel-height frame)
               :frame-char-height (frame-char-height frame)
               :frame-font (frame-parameter frame 'font)
               :buffer (buffer-name (current-buffer))
               :default-face-font (face-attribute 'default :font frame t)
               :default-face-family (face-attribute 'default :family frame t)
               :default-face-height (face-attribute 'default :height frame t)
               :default-face-weight (face-attribute 'default :weight frame t)
               :default-face-slant (face-attribute 'default :slant frame t)
               :header-line header-line-format
               :mode-line-height (window-mode-line-height)
               :header-line-height (window-header-line-height)
               :tab-line-height (window-tab-line-height)
               :tab-bar-lines (frame-parameter frame 'tab-bar-lines)
               :tab-bar-height (when (fboundp 'tab-bar-height)
                                 (tab-bar-height))
               :default-text-height (when (fboundp 'default-text-height)
                                      (default-text-height))
               :line-spacing line-spacing)
         (current-buffer))
        (insert "\n")))))

(defun romacs-chrome-probe-apply ()
  "Apply a consistent tab-bar/tab-line/header-line setup to the selected window."
  (interactive)
  (when (display-graphic-p)
    (tab-bar-mode 1)
    (global-tab-line-mode 1)
    (setq-default header-line-format romacs-chrome-probe-header)
    (switch-to-buffer "*scratch*")
    (force-mode-line-update t)
    (redisplay)
    (run-with-timer 0 nil #'romacs-chrome-probe--write-ready)))

(unless romacs-chrome-probe--armed
  (setq romacs-chrome-probe--armed t)
  (add-hook 'window-setup-hook #'romacs-chrome-probe-apply)
  (when after-init-time
    (run-with-timer 0 nil #'romacs-chrome-probe-apply)))

(provide 'chrome-probe)

;;; chrome-probe.el ends here
