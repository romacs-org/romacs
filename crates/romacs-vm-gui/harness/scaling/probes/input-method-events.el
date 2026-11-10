;;; input-method-events.el --- scaling probe  -*- lexical-binding: t -*-

;; One key whose method returns N events: they queue on
;; `unread-post-input-method-events' and drain one command each, to the
;; end of a keyboard macro.
(scaling-probe input-method-events :n 2000
  :setup (let ((buffer (get-buffer-create " *scaling-input*")))
           (with-current-buffer buffer (buffer-disable-undo))
           (set-window-buffer (selected-window) buffer)
           buffer)
  :body (let ((input-method-function (lambda (_) (make-list n ?x))))
          (execute-kbd-macro "a")
          (cl-assert (= (buffer-size it) n))))
