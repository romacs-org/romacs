;;; xcode-theme.el --- Xcode  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-xcode by kadenbarlow, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-xcode-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme xcode "Xcode, after doom-themes."
  :mode dark :family xcode :style flat
  ((bg "#292a30") (bg-dark "#252629") (fg "#ffffff") (fg-dim "#6c7986")
   (accent "#5dd8ff") (selection "#3e3f44") (red "#fc6a5d") (green "#67b7a4")
   (yellow "#d0bf68") (blue "#5dd8ff") (magenta "#d0a8ff") (cyan "#8abeb7")
   (orange "#fd8f3f") (keyword "#fc5fa3") (function "#d0a8ff") (type "#5dd8ff")
   (constant "#a167e6") (string "#fc6a5d") (variable "#41a1c0") (builtin "#9ef1dd")
   (comment "#6c7986") (doc "#808b96") (number "#d0bf68")))

;;; xcode-theme.el ends here
