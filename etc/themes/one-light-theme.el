;;; one-light-theme.el --- One Light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: One by GitHub, MIT, https://github.com/atom/atom
;; (1c3bd35ce238, packages/one-light-syntax/styles/colors.less).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme one-light "One Light: the balanced classic on white."
  :mode light :family one :style flat
  ((bg "#fafafa") (fg "#383a42") (fg-dim "#696c77") (muted "#a0a1a7")
   (accent "#526fff") (red "#e45649") (green "#50a14f") (yellow "#cb7701")
   (blue "#4078f2") (magenta "#a626a4") (cyan "#0184bc") (orange "#b76b01")
   (keyword "#a626a4") (function "#4078f2") (type "#cb7701") (constant "#b76b01")
   (string "#50a14f") (variable "#e45649") (builtin "#0184bc") (comment "#a0a1a7")))

;;; one-light-theme.el ends here
