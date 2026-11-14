;;; romacs-project-enumerations.el --- scaling probe  -*- lexical-binding: t -*-

;; The project commands' record of enumeration times (lisp/romacs/romacs-project.el):
;; one entry per project root, each decision one lookup and one stat of the
;; root's Git index.
(scaling-probe romacs-project-enumerations :n 2000
  :setup (progn (require 'romacs-project) (scaling--names "/nonexistent/scaling-project" n))
  :body (progn
          (dolist (root it) (romacs-project--record root 0.01))
          (dolist (root it) (romacs-project--enumeration-estimate root))
          (dolist (root it) (remhash root romacs-project--enumerations))))
