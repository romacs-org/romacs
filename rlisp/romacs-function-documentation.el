;;; romacs-function-documentation.el --- Private closure documentation bridge
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; GNU's generic function remains responsible for non-closure documentation.
;; Ordinary closures retain their documentation without exposing array slots.

;;; Code:

(require 'simple)

(cl-defmethod function-documentation ((function interpreted-function))
  (let ((doc (romacs--closure-document function)))
    (when (or (stringp doc) (fixnump doc) (fixnump (cdr-safe doc)))
      doc)))

(provide 'romacs-function-documentation)
;;; romacs-function-documentation.el ends here
