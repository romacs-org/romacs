;;; plain-theme.el --- Plain  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-plain by das-s, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-plain-theme.el; after https://github.com/gko/plain/).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme plain "Plain, after doom-themes."
  :mode light :family plain :style flat
  ((bg "#ffffff") (bg-dark "#f3f3f3") (fg "#282a2e") (fg-dim "#282a2e")
   (accent "#444444") (selection "#e7e7e7") (red "#282a2e") (green "#282a2e")
   (yellow "#282a2e") (blue "#282a2e") (magenta "#282a2e") (cyan "#282a2e")
   (orange "#282a2e") (keyword "#282a2e") (function "#282a2e") (type "#282a2e")
   (constant "#969896") (string "#969896") (variable "#969896") (builtin "#969896")
   (comment "#c5c8c6") (doc "#c5c8c6") (number "#969896")))

;;; plain-theme.el ends here
