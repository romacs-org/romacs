;;; scratch-probe.el --- Dump scratch buffer font-lock state -*- lexical-binding: t; -*-

(defvar romacs-scratch-probe-output
  (or (getenv "ROMACS_SCRATCH_PROBE_OUTPUT")
      "/tmp/romacs-scratch-probe.sexp")
  "Output file for the scratch-buffer startup probe.")

(defvar romacs-scratch-probe-delay
  (string-to-number
   (or (getenv "ROMACS_SCRATCH_PROBE_DELAY")
       "0.5"))
  "Seconds to wait before writing the scratch-buffer probe.")

(defvar romacs-scratch-probe--armed nil
  "Non-nil once the probe has been armed.")

(defun romacs-scratch-probe--comment-pos ()
  "Return the first comment position in `*scratch*', or nil."
  (with-current-buffer "*scratch*"
    (save-excursion
      (goto-char (point-min))
      (when (search-forward ";;" nil t)
        (match-beginning 0)))))

(defun romacs-scratch-probe--char-state (pos)
  "Return a plist describing POS in `*scratch*'."
  (when pos
    (with-current-buffer "*scratch*"
      (list :pos pos
            :char (char-after pos)
            :face (get-text-property pos 'face)
            :font-lock-face (get-text-property pos 'font-lock-face)
            :char-face (get-char-property pos 'face)
            :props (text-properties-at pos)))))

(defun romacs-scratch-probe--snapshot ()
  "Return a plist describing the live `*scratch*' state."
  (with-current-buffer "*scratch*"
    (let* ((comment-pos (romacs-scratch-probe--comment-pos))
           (sample-end (min (point-max) (+ (point-min) 120)))
           (raw-comment (romacs-scratch-probe--char-state comment-pos))
           (raw-next (romacs-scratch-probe--char-state (and comment-pos (1+ comment-pos)))))
      (when (fboundp 'font-lock-ensure)
        (font-lock-ensure (point-min) (point-max)))
      (list
       :major-mode major-mode
       :mode-name mode-name
       :global-font-lock (bound-and-true-p global-font-lock-mode)
       :font-lock font-lock-mode
       :jit (bound-and-true-p jit-lock-mode)
       :defaults font-lock-defaults
       :char-syntax-comment (and comment-pos (char-syntax (char-after comment-pos)))
       :syntax-entry-comment (and comment-pos (syntax-after comment-pos))
       :syntax-ppss-comment (and comment-pos (syntax-ppss comment-pos))
       :syntax-ppss-after-comment (and comment-pos (syntax-ppss (1+ comment-pos)))
       :buffer-sample (buffer-substring-no-properties (point-min) sample-end)
       :raw-comment raw-comment
       :raw-next raw-next
       :ensured-comment (romacs-scratch-probe--char-state comment-pos)
       :ensured-next (romacs-scratch-probe--char-state (and comment-pos (1+ comment-pos)))))))

(defun romacs-scratch-probe--write ()
  "Write the current `*scratch*' snapshot to disk."
  (when (get-buffer "*scratch*")
    (with-temp-file romacs-scratch-probe-output
      (let ((print-length nil)
            (print-level nil))
        (prin1 (romacs-scratch-probe--snapshot) (current-buffer))
        (insert "\n")))))

(defun romacs-scratch-probe-arm ()
  "Arm the one-shot scratch-buffer probe."
  (unless romacs-scratch-probe--armed
    (setq romacs-scratch-probe--armed t)
    (run-at-time romacs-scratch-probe-delay nil
                 #'romacs-scratch-probe--write)))

(add-hook 'emacs-startup-hook #'romacs-scratch-probe-arm)
(add-hook 'window-setup-hook #'romacs-scratch-probe-arm)

(when after-init-time
  (romacs-scratch-probe-arm))

(provide 'scratch-probe)

;;; scratch-probe.el ends here
