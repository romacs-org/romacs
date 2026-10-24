;; -*- lexical-binding: t -*-
(setq x 9)
(prin1 (let ((x 2)) (list (set 'x (list 'changed)) x (symbol-value 'x))))
(terpri)
(defvar s124-special 8)
(prin1 (list (let ((s124-special 3)) (makunbound 's124-special) (boundp 's124-special)) s124-special))
(terpri)
(with-temp-buffer (setq truncate-lines t) (with-temp-buffer (prin1 truncate-lines)))
(terpri)
