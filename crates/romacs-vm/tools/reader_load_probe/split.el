;;; split.el --- emit top-level form spans  -*- lexical-binding: t -*-
;; emacs -Q --batch -l split.el FILE OUT  -> JSON lines {"s":START,"e":END,"head":"defun"}
(let* ((file (nth 0 command-line-args-left))
       (out (nth 1 command-line-args-left))
       (forms nil))
  (with-temp-buffer
    (insert-file-contents file)
    (emacs-lisp-mode)
    (goto-char (point-min))
    (condition-case nil
        (while t
          (forward-comment (buffer-size))
          (let* ((s (point))
                 (f (read (current-buffer)))
                 (e (point)))
            (push (list s e (if (consp f) (format "%s" (car f)) "atom")) forms)))
      (end-of-file nil)))
  (with-temp-file out
    (dolist (f (nreverse forms))
      (insert (format "{\"s\":%d,\"e\":%d,\"head\":%S}\n" (nth 0 f) (nth 1 f) (nth 2 f))))))
