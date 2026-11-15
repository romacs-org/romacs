;;; romacs-key-hint.el --- Key hints for lines redisplay asks for  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Every GUI regime loads this before the terminal; a library that can load
;; in batch, as the scaling probes load the agents' pages, requires it.

;;; Code:

(defvar romacs-key-hint--cache (make-hash-table :test 'eq)
  "Each literal's hint, made since `romacs-key-hint--generation'.")

(defvar romacs-key-hint--generation nil
  "The keymap generation of the hints in `romacs-key-hint--cache'.")

(defun romacs-key-hint (text &optional face)
  "TEXT, a literal, through `substitute-command-keys', in FACE where no key
is: for a mode line or a header line, which redisplay asks for far more
often than a key is rebound, so keys are looked up once per keymap change."
  (let ((generation (romacs--keymap-generation)))
    (unless (eql generation romacs-key-hint--generation)
      (clrhash romacs-key-hint--cache)
      (setq romacs-key-hint--generation generation)))
  (or (gethash text romacs-key-hint--cache)
      (let ((hint (substitute-command-keys text)))
        (when face (add-face-text-property 0 (length hint) face t hint))
        (puthash text hint romacs-key-hint--cache))))

(provide 'romacs-key-hint)
;;; romacs-key-hint.el ends here
