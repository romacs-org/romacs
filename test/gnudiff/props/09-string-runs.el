;;; 09-string-runs.el --- Props and markers GNU oracle -*- lexical-binding: t; -*-
(case props-m6-string-semantics eval
 '(with-temp-buffer
    (emacs-lisp-mode)
    (insert "(defun alpha () \"beta\")")
    (font-lock-ensure)
    (let ((strings nil))
      (dotimes (_ 20000)
        (goto-char 1) (re-search-forward "alpha") (push (match-string 0) strings))
      (garbage-collect)
      (let* ((sample (car strings)) (p (propertize "ab" 'face 'bold))
             (joined (concat p (propertize "cd" 'face 'italic)))
             (sliced (substring joined 1 3)))
        (list (length strings)
              (mapcar #'object-intervals (list sample p joined sliced))
              (equal (object-intervals (car strings)) (object-intervals (car (last strings))))
              (mapcar (lambda (s) (substring-no-properties s)) (list sample p joined sliced)))))))
