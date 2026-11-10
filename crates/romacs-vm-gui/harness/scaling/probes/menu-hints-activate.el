;;; menu-hints-activate.el --- scaling probe  -*- lexical-binding: t -*-

;; One `set-transient-map' activation and exit at depth 1 with N other
;; maps registered (retained by the setup): the hints' registration and
;; reconcile do the same work whatever N is.
(scaling-probe menu-hints-activate :gui t :n 100 :steps 10
  :setup (let ((maps (mapcar (lambda (_) (define-keymap "a" #'ignore)) (make-list n nil))))
           (dolist (map maps) (funcall (set-transient-map map)))
           (cons (switch-to-buffer (get-buffer-create "scaling-hints")) maps))
  :body (progn (funcall (set-transient-map (define-keymap "b" #'ignore)))
               (with-current-buffer (car it) (insert "x"))))
