;;; theme-preview-names.el --- scaling probe  -*- lexical-binding: t -*-

;; `load-theme''s prompt with N themes offered: a command that leaves the
;; selection alone, and one that moves it to no theme.
(scaling-probe theme-preview-names :n 2000
  :setup (let ((names (mapcar #'symbol-name (mapcar #'intern (scaling--names "scaling-offered" n)))))
           (list (romacs-theme-preview--table names) (car (last names))))
  :body (romacs-theme-preview--session
          (let ((romacs-theme-preview--names (car it))
                (romacs-theme-preview--selected "")
                (name (cadr it)))
            (with-temp-buffer
              (dotimes (_ 200)
                (romacs-theme-preview--follow)
                (erase-buffer)
                (insert name)
                (romacs-theme-preview--follow)
                (erase-buffer))))))
