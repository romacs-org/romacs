;;; theme-preview-stack.el --- scaling probe  -*- lexical-binding: t -*-

;; Candidates over N enabled themes: the stack under them is left alone.
(scaling-probe theme-preview-stack :gui t :n 16 :steps 1
  :setup (let ((themes (mapcar #'intern (scaling--names "scaling-under" n))))
           (dolist (theme themes)
             (custom-declare-theme theme nil)
             (custom-theme-set-faces theme '(region ((t :underline t))))
             (enable-theme theme))
           (deftheme scaling-candidate-a "One face.")
           (custom-theme-set-faces 'scaling-candidate-a '(highlight ((t :foreground "#102030"))))
           (deftheme scaling-candidate-b "One face.")
           (custom-theme-set-faces 'scaling-candidate-b '(highlight ((t :foreground "#203040"))))
           (switch-to-buffer (get-buffer-create "scaling-under")))
  :body (progn
          (romacs-theme-preview--session
            (dotimes (_ 8)
              (romacs-theme-preview--show 'scaling-candidate-a)
              (romacs-theme-preview--show 'scaling-candidate-b)))
          (with-current-buffer it (insert "x"))))
