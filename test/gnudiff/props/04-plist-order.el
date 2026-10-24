;;; 04-plist-order.el --- Props and markers GNU oracle -*- lexical-binding: t; -*-
(case props-plist-order eval
 '(let ((s (copy-sequence "abcd")) rows)
    (dolist (op '((add-text-properties 0 4 (a 1 b 2)) (put-text-property 0 4 a 3)
                  (add-text-properties 0 4 (c 4 b 5 d 6))
                  (remove-text-properties 0 4 (b nil))
                  (add-text-properties 0 4 (b 7 a 8))
                  (set-text-properties 1 3 (z 9 a 8))))
      (apply (car op) (append (cdr op) (list s)))
      (push (list op (copy-tree (object-intervals s)) (copy-sequence (text-properties-at 0 s))) rows))
    (vconcat (nreverse rows))))
