;;; -*- lexical-binding: t -*-
(princ (format "GNU %s\n" emacs-version))
(with-temp-buffer
  (insert "abcd")
  (let ((tick (buffer-modified-tick)))
    (with-silent-modifications (put-text-property 1 3 'face 'bold))
    (princ (format "silent-property-tick-delta=%S\n" (- (buffer-modified-tick) tick)))))
(with-temp-buffer
  (emacs-lisp-mode)
  (insert ";; first line\n\n(defun foo () 1)\n\n(defun bar () 2)\n")
  (let ((noninteractive nil)) (font-lock-mode 1))
  (font-lock-set-defaults)
  (jit-lock-register #'font-lock-fontify-region (not font-lock-keywords-only))
  (jit-lock-fontify-now (point-min) (point-max))
  (let ((at (- (point-max) 12)))
    (princ (format "before-context-edit=%S\n" (list (get-text-property at 'fontified) (get-text-property at 'face))))
    (goto-char 1) (insert "\"")
    (princ (format "immediate-dependent-state=%S\n" (list (get-text-property (1+ at) 'fontified) (get-text-property (1+ at) 'face))))))
(with-temp-buffer
  (insert "abcd")
  (let ((a (make-overlay 1 3)) (b (make-overlay 1 3)))
    (overlay-put a 'face 'A) (overlay-put b 'face 'B)
    (princ (format "identical-overlays-newer-winner=%S\n" (get-char-property 1 'face)))))
(with-temp-buffer
  (insert "abcd")
  (set-text-properties 1 2 '(p 1 q 2))
  (set-text-properties 2 3 '(q 2 p 1))
  (princ (format "same-set-next-boundary=%S intervals=%S\n" (next-property-change 1) (object-intervals (current-buffer)))))
