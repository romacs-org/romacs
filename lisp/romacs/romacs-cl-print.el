;;; romacs-cl-print.el --- Private closure printing for GNU cl-print
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; GNU's interpreted-function method reads its array representation.  Romacs
;; closures use the native opaque printer instead of exposing code or captures.

;;; Code:

(require 'romacs-cl-generic)
(require 'cl-print)

(cl-defmethod cl-print-object ((object interpreted-function) stream)
  (prin1 object stream))

(provide 'romacs-cl-print)
;;; romacs-cl-print.el ends here
