;;; 02-string-insert.el --- Props and markers GNU oracle -*- lexical-binding: t; -*-
(case props-string-insert eval
 '(let* ((value (list 'shared)) (other (list 'other))
         (s (propertize "aé中d" 'p value)))
    (put-text-property 2 4 'q other s)
    (with-temp-buffer
      (insert "LR") (put-text-property 1 3 'p 'old) (goto-char 2) (insert s)
      (list (buffer-substring-no-properties 1 (point-max))
            (object-intervals (current-buffer))
            (mapcar (lambda (pos) (list (eq (get-text-property pos 'p) value)
                                       (eq (get-text-property pos 'q) other))) '(2 3 4 5))
            (eq (get-text-property 0 'p s) (get-text-property 2 'p))))))
