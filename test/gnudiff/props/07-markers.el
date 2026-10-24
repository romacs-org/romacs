;;; 07-markers.el --- Props and markers GNU oracle -*- lexical-binding: t; -*-
(case props-marker-edits eval
 '(let (rows)
    (dolist (op '(insert insert-before-markers delete replace transpose))
      (with-temp-buffer
        (insert "abcdefghij")
        (let ((markers (apply #'append
                         (mapcar (lambda (p) (list (copy-marker p) (copy-marker p t))) '(1 3 4 5 7 8 11)))))
          (goto-char 3)
          (pcase op
            ('insert (insert "XY")) ('insert-before-markers (insert-before-markers "XY"))
            ('delete (delete-region 3 8))
            ('replace (looking-at "cdefg") (replace-match "XYZ" t t))
            ('transpose (transpose-regions 2 4 7 9)))
          (push (list op (buffer-substring-no-properties 1 (point-max))
                  (mapcar #'marker-position markers)
                  (mapcar #'marker-insertion-type markers)) rows))))
    (vconcat (nreverse rows))))
(case props-marker-lifetimes eval
 '(let ((base (generate-new-buffer " *props-base*")) indirect m)
    (unwind-protect
        (progn
          (with-current-buffer base
            (insert "abcde") (setq m (copy-marker 3 t))
            (setq indirect (make-indirect-buffer base " *props-indirect*")))
          (let ((cross (with-temp-buffer
                         (insert "XY") (set-marker m 2 (current-buffer))
                         (list (marker-position m) (eq (marker-buffer m) (current-buffer))))))
            (set-marker m 3 base)
            (with-current-buffer indirect (goto-char 3) (insert "Q"))
            (let ((shared (marker-position m))
                  (copy (copy-marker m)))
              (kill-buffer base)
              (list cross shared (marker-insertion-type copy)
                    (marker-position m) (marker-buffer m) (marker-last-position m)
                    (buffer-live-p indirect)))))
      (when (buffer-live-p indirect) (kill-buffer indirect))
      (when (buffer-live-p base) (kill-buffer base)))))
(case props-marker-copy-detach eval
 '(with-temp-buffer
    (insert "abc")
    (let* ((original (copy-marker 2 t)) (before (copy-marker original))
           (after (copy-marker original t)))
      (goto-char 2) (insert "X")
      (let ((positions (mapcar #'marker-position (list original before after))))
        (set-marker before nil)
        (list positions (mapcar #'marker-insertion-type (list original before after))
              (marker-position before) (marker-buffer before) (marker-last-position before))))))
(case props-marker-save-excursion eval
 '(with-temp-buffer
    (insert "abcde") (goto-char 3)
    (let ((inside (save-excursion (goto-char 1) (insert "XY") (point))))
      (list inside (point)))))
(case props-marker-undo-redo eval
 '(with-temp-buffer
    (buffer-enable-undo) (insert "abcdef") (setq buffer-undo-list nil)
    (let* ((markers (list (copy-marker 2) (copy-marker 3 t) (copy-marker 5)))
           (positions (lambda () (mapcar #'marker-position markers))))
      (delete-region 2 5)
      (let* ((deleted (funcall positions))
             (record (mapcar (lambda (entry)
                              (if (and (consp entry) (markerp (car entry)))
                                  (list 'marker (marker-position (car entry)) (cdr entry)) entry))
                            buffer-undo-list))
             (undo-list buffer-undo-list))
        (setq buffer-undo-list nil)
        (primitive-undo (length undo-list) undo-list)
        (let ((restored (funcall positions)) (redo-list buffer-undo-list))
          (setq buffer-undo-list nil)
          (primitive-undo (length redo-list) redo-list)
          (list record deleted restored (funcall positions)
                (buffer-substring-no-properties 1 (point-max))))))))
