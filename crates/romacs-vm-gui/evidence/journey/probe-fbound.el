;;; probe-fbound.el --- report which journey symbols are unbound -*- lexical-binding: t -*-
;; Loaded on the census binary; the single form signals with the list of
;; symbols that are NOT fbound (or boundp for the variables), so the result
;; travels in the report's error text.
(let ((fns '(defgroup key-parse propertize gv-define-expander
             insert-file-contents write-region set-visited-file-name
             set-buffer-modified-p buffer-modified-p read-from-minibuffer
             completing-read buffer-list scroll-up scroll-down recenter
             primitive-undo undo-boundary generate-new-buffer-name
             current-global-map kill-region vertical-motion
             make-overlay overlay-put file-exists-p expand-file-name
             commandp interactive-form current-local-map use-local-map
             buffer-file-name find-file-noselect insert-file-contents-literally
             file-attributes verify-visited-file-modtime set-buffer-major-mode
             window-start set-window-start pos-visible-in-window-p
             invisible-p line-beginning-position read-key-sequence
             substitute-command-keys format-mode-line))
      (vars '(buffer-undo-list buffer-file-name mode-line-format
              buffer-invisibility-spec))
      (missing nil))
  (dolist (f fns) (unless (fboundp f) (setq missing (cons f missing))))
  (dolist (v vars) (unless (boundp v) (setq missing (cons (list 'var v) missing))))
  ;; `signal', not `error': warm subr.el's `error' needs `format-message'.
  (signal 'probe-missing (nreverse missing)))
