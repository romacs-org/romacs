;;; syntax-list-spans-edit.el --- scaling probe -*- lexical-binding: t -*-

(scaling-probe syntax-list-spans-edit :n 4000 :budget 0.5
  :setup (let ((buffer (scaling--syntax-buffer " scaling-list-spans" n)))
           (with-current-buffer buffer
             (setq-local parse-sexp-ignore-comments t)
             (scan-sexps (point-min) (point-max)))
           buffer)
  :body (with-current-buffer it
          (dotimes (_ 20)
            (goto-char 100)
            (insert "x")
            (scan-sexps (point-min) (point-max)))))
