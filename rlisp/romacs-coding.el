;;; romacs-coding.el --- Workspace coding state -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; The workspace has no loadup, so startup evaluates GNU's own mule.el and
;; mule-conf.el definitions of the UTF-8 and Latin-1 coding systems, and
;; GNU's `set-auto-coding' and `select-safe-coding-system' as the visit and
;; save hooks (see `initialize_coding' in crates/romacs-vm-gui/src/startup.rs).
;; This file sets the state GNU reaches after mule-conf.el and
;; `set-locale-environment' for a UTF-8 locale on GNU/Linux, as pinned GNU
;; reports it: `coding-system-priority-list' restricted to the systems defined
;; here, and the locale's `utf-8-unix' defaults.  The workspace assumes a UTF-8
;; locale.

;;; Code:
(set-coding-system-priority 'utf-8 'iso-latin-1 'raw-text 'utf-8-auto
                            'utf-8-with-signature)
(set-default 'buffer-file-coding-system 'utf-8-unix)
(defvar default-file-name-coding-system nil)
(setq default-file-name-coding-system 'utf-8-unix)

;; Workspace adaptation: GNU asks when no default coding system can encode
;; the text, e.g. a `coding: latin-1' cookie over a buffer holding λ.  Its
;; question explains the choice in a *Warning* window with help buttons, which
;; the workspace cannot display, so the workspace refuses instead.  The signal
;; leaves `write-region' before it opens the file, so nothing is written and
;; the buffer stays modified.  The data are the unsafe defaults and the safe
;; coding systems GNU would offer, utf-8 first.
(define-error 'romacs-unsafe-coding
  "Write refused: no default coding system encodes the text (unsafe, safe)")

(defun select-safe-coding-system-interactively (_from _to codings unsafe
                                                &optional _rejected _default)
  "Refuse the write `select-safe-coding-system' would ask about.
Signal `romacs-unsafe-coding' with UNSAFE and the safe CODINGS."
  (signal 'romacs-unsafe-coding (list unsafe codings)))
