;;; nord-theme.el --- Nord  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Nord by Sven Greb, MIT, https://github.com/nordtheme/nord
;; (1cef71605416, src/nord.css).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme nord "Nord: arctic blue-grey."
  :mode dark :family nord :style flat
  ((bg "#2e3440") (bg-alt "#3b4252") (fg "#d8dee9") (fg-light "#e5e9f0")
   (fg-bright "#eceff4") (muted "#4c566a") (accent "#88c0d0") (selection "#434c5e")
   (red "#bf616a") (green "#a3be8c") (yellow "#ebcb8b") (blue "#81a1c1")
   (magenta "#b48ead") (cyan "#88c0d0") (orange "#d08770") (keyword "#81a1c1")
   (function "#88c0d0") (type "#8fbcbb") (constant "#b48ead") (string "#a3be8c")
   (variable "#d8dee9") (builtin "#81a1c1") (comment "#4c566a") (ansi-0 "#3b4252")
   (ansi-7 "#e5e9f0") (ansi-8 "#4c566a") (ansi-14 "#8fbcbb") (ansi-15 "#eceff4")))

;;; nord-theme.el ends here
