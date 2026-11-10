;;; undo.el --- scaling probe  -*- lexical-binding: t -*-

(scaling-probe undo :n 2000
  :setup (with-current-buffer (get-buffer-create " scaling-undo")
           (buffer-enable-undo)
           (current-buffer))
  :body (with-current-buffer it
          (dotimes (_ n) (insert "x") (undo-boundary))
          (undo-start)
          (undo-more n)))
