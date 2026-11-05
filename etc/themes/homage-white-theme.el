;;; homage-white-theme.el --- Homage White  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-homage-white by mskorzhinskiy, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-homage-white-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme homage-white "Homage White, after doom-themes."
  :mode light :family homage :style flat
  ((bg "#fafafa") (bg-dark "#f0f0f0") (fg "#383a42") (fg-dim "#383a42")
   (accent "#014980") (selection "#d8d8d8") (red "#e45649") (green "#556b2f")
   (yellow "#986801") (blue "#014980") (magenta "#a626a4") (cyan "#0184bc")
   (orange "#8a3b3c") (keyword "#383a42") (function "#014980") (type "#383a42")
   (constant "#383a42") (string "#8a3b3c") (variable "#383a42") (builtin "#383a42")
   (comment "#556b2f") (doc "#485a27") (number "#8a3b3c")))

;;; homage-white-theme.el ends here
