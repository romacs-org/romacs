;;; romacs-oclosure.el --- OClosure representation bridge -*- lexical-binding: t; -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; Ordinary Romacs closures retain documentation without public array slots.

;;; Code:

(require 'oclosure)

(defun oclosure-type (object)
  "Return OBJECT's symbolic closure type, or nil."
  (romacs--oclosure-type object))

(provide 'romacs-oclosure)
;;; romacs-oclosure.el ends here
