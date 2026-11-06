;;; rum-theme.el --- Rum, exactly as the reference session shows it  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; Rum exactly: every face as GNU shows it with modus-themes 5.3.0 and
;; Rum's theme file under the reference configuration
;; (crates/romacs-vm-gui/tests/romacs-theme-rum-test.el).  The stock
;; theme `default' starts from the same definition.

;;; Code:

(require 'romacs-theme)
;; Rum's definition is data beside the theme files, not on `load-path'
;; (the workspace, which evaluates this file's text, provides it first).
(unless (featurep 'romacs-theme-rum)
  (load (expand-file-name "romacs-theme-rum" (file-name-directory load-file-name)) nil t))

(apply #'romacs-theme-define
       'rum "Rum: warm parchment and restrained accents on black, as GNU shows it on Modus 5.3.0."
       :family 'rum :style 'flat romacs-theme-rum)

;;; rum-theme.el ends here
