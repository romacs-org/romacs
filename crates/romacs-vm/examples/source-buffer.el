;;; source-buffer.el --- Source evaluation fixture -*- lexical-binding: t; -*-
;;; A complete source file, not a hand-constructed Form.
(defun count-action-items (&optional needle)
  "Count task markers in the current buffer."
  (goto-char (point-min))
  (let ((count 0))
    (while (search-forward (if needle needle "TODO") nil t)
      (setq count (1+ count)))
    count))
(with-temp-buffer
  (insert "TODO reply to review\nDONE publish\nTODO pay invoice\n")
  (count-action-items))
