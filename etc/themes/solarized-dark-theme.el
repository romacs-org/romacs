;;; solarized-dark-theme.el --- Solarized, dark  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Solarized by Ethan Schoonover, MIT, https://github.com/altercation/solarized
;; (62f656a02f93, README.md).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme solarized-dark "Solarized: precision colours on dark."
  :mode dark :family solarized :style flat
  ((bg "#002b36") (bg-alt "#073642") (fg "#839496") (fg-dim "#586e75")
   (fg-light "#93a1a1") (fg-bright "#eee8d5") (muted "#586e75") (accent "#268bd2")
   (selection "#073642") (red "#dc322f") (green "#859900") (yellow "#b58900")
   (blue "#268bd2") (magenta "#d33682") (cyan "#2aa198") (orange "#cb4b16")
   (keyword "#859900") (function "#268bd2") (type "#b58900") (constant "#2aa198")
   (string "#2aa198") (variable "#268bd2") (builtin "#cb4b16") (comment "#586e75")
   (ansi-0 "#073642") (ansi-7 "#eee8d5") (ansi-8 "#002b36") (ansi-9 "#cb4b16")
   (ansi-10 "#586e75") (ansi-11 "#657b83") (ansi-12 "#839496") (ansi-13 "#6c71c4")
   (ansi-14 "#93a1a1") (ansi-15 "#fdf6e3")))

;;; solarized-dark-theme.el ends here
