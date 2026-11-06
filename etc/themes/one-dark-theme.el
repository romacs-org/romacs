;;; one-dark-theme.el --- One Dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: One by GitHub, MIT, https://github.com/atom/atom
;; (1c3bd35ce238, packages/one-dark-syntax/styles/colors.less).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme one-dark "One Dark: the balanced classic."
  :mode dark :family one :style flat
  ((bg "#282c34") (fg "#abb2bf") (fg-dim "#828997") (muted "#5c6370")
   (accent "#528bff") (red "#e06c75") (green "#98c379") (yellow "#e5c07b")
   (blue "#61afef") (magenta "#c678dd") (cyan "#56b6c2") (orange "#d19a66")
   (keyword "#c678dd") (function "#61afef") (type "#e5c07b") (constant "#d19a66")
   (string "#98c379") (variable "#e06c75") (builtin "#56b6c2") (comment "#5c6370")))

;;; one-dark-theme.el ends here
