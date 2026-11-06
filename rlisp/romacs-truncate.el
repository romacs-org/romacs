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

(provide 'romacs-truncate)
;;; romacs-truncate.el ends here
