;;; romacs-truncate.el --- Truncate text within its width  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; `truncate-string-to-width' with an ellipsis wider than END-COLUMN
;; returns all of its string and the ellipsis.  A caller whose width can
;; be that narrow uses `romacs-truncate', which leaves the ellipsis out.

;;; Code:

(require 'mule-util)

;;;###autoload
(defun romacs-truncate (string width &optional padding)
  "STRING in at most WIDTH columns, a cut one ending in an ellipsis when
WIDTH leaves room for more than that.  PADDING, a character, fills a
shorter STRING to WIDTH."
  (truncate-string-to-width string (max 0 width) nil padding
                            (> width (string-width (truncate-string-ellipsis)))))

;;;###autoload
(defun romacs-truncate-middle (name width)
  "NAME in at most WIDTH columns, its middle elided and its extension kept."
  (if (<= (string-width name) width) name
    (let* ((extension (file-name-extension name t))
           (stem (file-name-sans-extension name))
           (dots (truncate-string-ellipsis))
           (room (- width (string-width extension) (string-width dots))))
      (if (< room 2) (romacs-truncate name width)
        (concat (truncate-string-to-width stem (ceiling room 2)) dots
                (romacs-truncate--tail stem (/ room 2)) extension)))))

(defun romacs-truncate--tail (string width)
  "The end of STRING in at most WIDTH columns."
  (let ((start (length string)))
    (while (and (> start 0) (<= (string-width string (1- start)) width))
      (setq start (1- start)))
    (substring string start)))

(provide 'romacs-truncate)
;;; romacs-truncate.el ends here
