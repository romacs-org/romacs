;;; iosvkem-theme.el --- Iosvkem  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-Iosvkem by neutaaaaan, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-Iosvkem-theme.el; after https://github.com/neutaaaaan/iosvkem).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme iosvkem "Iosvkem, after doom-themes."
  :mode dark :family iosvkem :style flat
  ((bg "#1b1d1e") (bg-dark "#262829") (fg "#dddddd") (accent "#fc20bb")
   (selection "#464849") (red "#d02b61") (green "#60aa00") (yellow "#d08928")
   (blue "#6c9ef8") (magenta "#b77fdb") (cyan "#00aa80") (orange "#da8548")
   (keyword "#6c9ef8") (function "#b77fdb") (type "#00aa80") (constant "#60aa00")
   (string "#d08928") (variable "#5699af") (builtin "#b77fdb") (comment "#808080")
   (doc "#9f9f9f") (number "#60aa00")))

;;; iosvkem-theme.el ends here
