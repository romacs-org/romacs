;;; vibrant-theme.el --- Vibrant  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-vibrant by Henrik Lissner, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-vibrant-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme vibrant "Vibrant, after doom-themes."
  :mode dark :family vibrant :style flat
  ((bg "#242730") (bg-dark "#2a2e38") (fg "#bbc2cf") (accent "#51afef")
   (selection "#3d4451") (red "#ff665c") (green "#7bc275") (yellow "#fcce7b")
   (blue "#51afef") (magenta "#c57bdb") (cyan "#5cefff") (orange "#e69055")
   (keyword "#51afef") (function "#5cefff") (type "#fcce7b") (constant "#a991f1")
   (string "#7bc275") (variable "#dcafe9") (builtin "#c57bdb") (comment "#62686e")
   (doc "#7e7e87") (number "#e69055")))

;;; vibrant-theme.el ends here
