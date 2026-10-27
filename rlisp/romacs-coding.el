;;; romacs-coding.el --- Workspace coding state -*- lexical-binding: t -*-

;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:

;; The workspace has no loadup, so startup evaluates GNU's own mule.el and
;; mule-conf.el definitions of the UTF-8 and Latin-1 coding systems (see
;; `initialize_coding' in crates/romacs-vm-gui/src/startup.rs).  This file
;; sets the state GNU reaches after mule-conf.el and `set-locale-environment'
;; for a UTF-8 locale on GNU/Linux, as pinned GNU reports it:
;; `coding-system-priority-list' restricted to the systems defined here, and
;; the locale's `utf-8-unix' defaults.  The workspace assumes a UTF-8 locale.

;;; Code:
(set-coding-system-priority 'utf-8 'iso-latin-1 'raw-text 'utf-8-auto
                            'utf-8-with-signature)
(set-default 'buffer-file-coding-system 'utf-8-unix)
(defvar default-file-name-coding-system nil)
(setq default-file-name-coding-system 'utf-8-unix)
