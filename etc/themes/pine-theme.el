;;; pine-theme.el --- Pine  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-pine by Roman Hargrave, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-pine-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme pine "Pine, after doom-themes."
  :mode dark :family pine :style flat
  ((bg "#0c1400") (bg-dark "#121e00") (fg "#d5c9b0") (fg-dim "#928374")
   (accent "#d5c4a1") (selection "#222b14") (red "#ce4e2b") (green "#86914e")
   (yellow "#e5aa2b") (blue "#83a598") (magenta "#cc241d") (cyan "#8ec07c")
   (orange "#fe8019") (keyword "#ce4e2b") (function "#8ec07c") (type "#e5aa2b")
   (constant "#d3869b") (string "#86914e") (variable "#8ec07c") (builtin "#fe8019")
   (comment "#49503d") (doc "#dfd2b8") (number "#d3869b")))

;;; pine-theme.el ends here
