;;; treesit-font-lock-settings.el --- scaling probe  -*- lexical-binding: t -*-

;; A Rust buffer fontified through N font-lock settings of one language,
;; each capturing every identifier with an override: one combined query
;; walks the text once, and each capture is applied once per setting that
;; names it, so the work is linear in N (romacs-treesit-font-lock.el).
(scaling-probe treesit-font-lock-settings :n 40 :budget 1.25
  :setup (with-current-buffer (progn (require (quote treesit)) (get-buffer-create " scaling-treesit-settings"))
           (dotimes (i 200)
             (insert (format "fn f%d(x: u32) -> u32 { let y = x + %d; y * 2 }\n" i i)))
           (setq-local treesit-primary-parser (treesit-parser-create 'rust))
           (setq-local treesit-font-lock-settings
                       (apply #'treesit-font-lock-rules
                              (apply #'append
                                     (mapcar (lambda (i)
                                               (list :language 'rust :feature (intern (format "f%d" i))
                                                     :override t '((identifier) @font-lock-variable-name-face)))
                                             (number-sequence 1 n)))))
           (current-buffer))
  :body (with-current-buffer it
          (treesit-font-lock-fontify-region (point-min) (point-max))))
