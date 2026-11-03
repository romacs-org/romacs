;;; system-theme.el --- The palette file `romacs-theme-system-file' names  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The theme of a palette file kept outside Romacs, an Omarchy theme's
;; colors.toml for one: `romacs-theme-system-file' names it.  Base colours
;; the file leaves out take the stock theme's; the rest are derived.

;;; Code:

(require 'romacs-theme)

(let ((file romacs-theme-system-file))
  (unless (and file (file-readable-p file))
    (user-error "Set `romacs-theme-system-file' to a palette file first"))
  (let* ((own (romacs-theme-read-palette file))
         (stock (get 'default 'romacs-palette))
         ;; Only base colours the file leaves out come from the stock
         ;; theme; every other slot is derived from the file's.
         (palette (append own (let ((rest nil))
                                (dolist (entry stock (nreverse rest))
                                  (when (and (memq (car entry) romacs-theme-base-slots)
                                             (null (assq (car entry) own)))
                                    (push entry rest)))))))
    (romacs-theme-define 'system (format "The palette of %s." (abbreviate-file-name file))
                         :family 'system :mode (car (cdr (assq 'mode own))) :palette palette)))

;;; system-theme.el ends here
