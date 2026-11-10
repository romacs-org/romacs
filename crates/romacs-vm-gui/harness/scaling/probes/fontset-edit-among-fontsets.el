;;; fontset-edit-among-fontsets.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe fontset-edit-among-fontsets :n 1000
  :setup (let ((name nil)) (dotimes (_ n) (setq name (new-fontset (scaling--fontset-name) nil))) name)
  :body (dotimes (_ 200) (set-fontset-font it ?Ж "Y") (fontset-font it ?Ж)))
