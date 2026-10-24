;;; 10-overlay-properties.el --- Props and markers GNU oracle -*- lexical-binding: t; -*-
(case props-overlay-combination eval
 '(with-temp-buffer
    (insert "abcdef") (put-text-property 1 7 'p 'text)
    (let ((low (make-overlay 2 5)) (high (make-overlay 3 4)))
      (overlay-put low 'p 'low) (overlay-put low 'priority 1)
      (overlay-put high 'p 'high) (overlay-put high 'priority 2)
      (list
        (mapcar (lambda (p) (list p (get-text-property p 'p)
                                 (get-char-property p 'p) (get-pos-property p 'p))) '(1 2 3 4 5 6 7))
        (mapcar (lambda (p) (list p (next-single-char-property-change p 'p nil 7)
                                 (previous-single-char-property-change p 'p nil 1))) '(1 2 3 4 5 6 7))))))
