;;; 08-plist-identity.el --- Props and markers GNU oracle -*- lexical-binding: t; -*-
(case props-d1-gnu-observation eval
 '(with-temp-buffer
    (insert "abcde")
    (set-text-properties 1 3 (list 'p 'a))
    (set-text-properties 4 6 (list 'p 'a))
    (let ((a (text-properties-at 1)) (b (text-properties-at 2)) (c (text-properties-at 4)))
      (let ((same (eq a b)) (separate (eq a c)) (tick (buffer-modified-tick)))
        (setcar (cdr a) 'mutated)
        (list same separate (equal a c)
              (mapcar (lambda (p) (get-text-property p 'p)) '(1 2 3 4 5))
              (- (buffer-modified-tick) tick))))))
