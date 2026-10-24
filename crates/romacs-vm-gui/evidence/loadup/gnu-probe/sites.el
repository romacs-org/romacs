;;; -*- lexical-binding: t -*-
(let ((n 0) (names nil))
  (with-temp-buffer
    (insert-file-contents (car command-line-args-left))
    (goto-char (point-min))
    (condition-case nil
        (while t
          (let ((form (read (current-buffer))))
            (named-let walk ((f form))
              (when (consp f)
                (when (and (eq (car f) 'load) (proper-list-p f)) (setq n (1+ n)) (push (cadr f) names))
                (when (proper-list-p f) (mapc #'walk f))
                (unless (proper-list-p f) (walk (car f)) (walk (cdr f)))))))
      (end-of-file nil)))
  (message "GNU reader: %d (load ...) calls in loadup.el; first %S; 68th %S; last %S" n (car (last names)) (nth (- n 68) names) (car names)))
