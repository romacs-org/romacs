;;; default-theme.el --- Romacs's stock theme: warm parchment on black  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The look a Romacs session starts in.  It starts from Rum
;; (romacs-theme-rum.el, beside this file); `rum' keeps that look exactly, while
;; this theme is where Romacs's own look grows.

;;; Code:

(require 'romacs-theme)
;; Rum's definition is data beside the theme files, not on `load-path'
;; (the workspace, which evaluates this file's text, provides it first).
(unless (featurep 'romacs-theme-rum)
  (load (expand-file-name "romacs-theme-rum" (file-name-directory load-file-name)) nil t))

(apply #'romacs-theme-define
       'default "Romacs's stock look: warm parchment and restrained accents on black."
       :family 'default romacs-theme-rum)

;;; default-theme.el ends here
