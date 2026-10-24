;;; 11-fontlock.el --- Font-lock and escape regression oracles -*- lexical-binding: t; -*-
(case props-fontlock-backslash-reader eval
 '(mapcar (lambda (body)
            (condition-case err
                (string-to-list (car (read-from-string (concat "\"" "\\" body "\""))))
              (error err)))
          '("C" "M" "C-m" "M-x" "M-\\r" "M-g\\M-c")))
(case props-fontlock-backslash eval
 '(with-temp-buffer
    (insert "(list \"\\C-m\" \"\\M-x\" \"\\M-\\r\" \"\\M-g\\M-c\")\n")
    (delay-mode-hooks (emacs-lisp-mode))
    (setq-local font-lock-support-mode nil)
    (font-lock-mode 1) (font-lock-ensure)
    (object-intervals (current-buffer))))
(case props-fontlock-org eval
 '(with-temp-buffer
    (insert "* TODO Heading :tag:\nSome *bold* and /italic/ and =code=.\n[[https://example.com][link]]\n#+begin_src emacs-lisp\n(+ 1 2)\n#+end_src\n")
    (delay-mode-hooks (org-mode))
    (setq-local font-lock-support-mode nil)
    (font-lock-mode 1) (font-lock-ensure)
    (let ((p (point-min)) runs)
      (while (< p (point-max))
        (let ((end (next-property-change p nil (point-max))))
          (push (list p end (text-properties-at p)) runs)
          (setq p end)))
      (secure-hash 'sha1 (prin1-to-string (nreverse runs))))))
(case props-fontlock-c-xdisp eval
 '(with-temp-buffer
    (insert-file-contents (or (getenv "CORE_BENCH_XDISP")
                              (error "Set CORE_BENCH_XDISP to pinned GNU xdisp.c")))
    (delay-mode-hooks (c-mode))
    (setq-local font-lock-support-mode nil)
    (font-lock-mode 1) (font-lock-ensure)
    (let ((p (point-min)) runs)
      (while (< p (point-max))
        (let ((end (next-property-change p nil (point-max))))
          (push (list p end (text-properties-at p)) runs)
          (setq p end)))
      (list (buffer-size) (length runs)
            (secure-hash 'sha1 (prin1-to-string (nreverse runs)))))))
