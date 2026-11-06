;;; rouge-theme.el --- Rouge  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-rouge by das-s, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-rouge-theme.el; after https://github.com/josefaidt/rouge-theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme rouge "Rouge, after doom-themes."
  :mode dark :family rouge :style flat
  ((bg "#172030") (bg-dark "#101828") (fg "#fafff6") (fg-dim "#64727d")
   (accent "#b16e75") (selection "#5d636e") (red "#c6797e") (green "#a3b09a")
   (yellow "#f7e3af") (blue "#6e94b9") (magenta "#b18bb1") (cyan "#88c0d0")
   (orange "#eabe9a") (keyword "#b18bb1") (function "#f9b5ac") (type "#c6797e")
   (constant "#c6797e") (string "#a3b09a") (variable "#c6797e") (builtin "#db6e8f")
   (comment "#64727d") (doc "#a3b09a") (number "#eabe9a")))

;;; rouge-theme.el ends here
