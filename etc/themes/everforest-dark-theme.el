;;; everforest-dark-theme.el --- Everforest, dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Everforest by sainnhe, MIT, https://github.com/sainnhe/everforest
;; (85a86eb62409, autoload/everforest.vim, medium contrast).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme everforest-dark "Everforest: quiet greens on dark."
  :mode dark :family everforest :style flat
  ((bg "#2d353b") (bg-dark "#232a2e") (bg-alt "#343f44") (fg "#d3c6aa")
   (fg-dim "#859289") (fg-light "#9da9a0") (muted "#7a8478") (accent "#a7c080")
   (selection "#543a48") (red "#e67e80") (green "#a7c080") (yellow "#dbbc7f")
   (blue "#7fbbb3") (magenta "#d699b6") (cyan "#83c092") (orange "#e69875")
   (keyword "#e67e80") (function "#a7c080") (type "#dbbc7f") (constant "#d699b6")
   (string "#83c092") (variable "#7fbbb3") (builtin "#e69875") (comment "#859289")))

;;; everforest-dark-theme.el ends here
