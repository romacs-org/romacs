;;; romacs-cl-generic.el --- GNU generic functions for Romacs -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Load after the GNU core prerequisites.  The representation bridge must be
;; installed before GNU builds dispatchers that inspect ordinary closures.

;;; Code:

(require 'romacs-oclosure)
(require 'cl-generic)

(provide 'romacs-cl-generic)
;;; romacs-cl-generic.el ends here
