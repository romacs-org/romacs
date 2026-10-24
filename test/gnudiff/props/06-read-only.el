;;; 06-read-only.el --- Props and markers GNU oracle -*- lexical-binding: t; -*-
(case props-read-only-matrix eval
 '(let (rows)
    (dolist (sticky '((nil nil) (t nil) ((read-only) nil) (nil t) (nil (read-only))))
      (dolist (inhibit '(nil t (locked) (other)))
        (dolist (pos '(1 2 3 4 5))
          (dolist (fn '(insert insert-and-inherit delete-region))
            (with-temp-buffer
              (insert "abcd")
              (add-text-properties 2 5 (list 'read-only 'locked 'front-sticky (car sticky)
                                           'rear-nonsticky (cadr sticky)))
              (goto-char pos)
              (let* ((inhibit-read-only inhibit)
                     (result (condition-case e
                               (progn (if (eq fn 'delete-region)
                                          (delete-region (max 1 (1- pos)) pos)
                                        (funcall fn "X")) 'ok)
                               (error e))))
                (push (list sticky inhibit pos fn result
                        (buffer-substring-no-properties 1 (point-max))) rows)))))))
    (vconcat (nreverse rows))))
