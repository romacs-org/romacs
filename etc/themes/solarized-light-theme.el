;;; solarized-light-theme.el --- Solarized, light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Solarized by Ethan Schoonover, MIT, https://github.com/altercation/solarized
;; (62f656a02f93, README.md).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme solarized-light "Solarized: precision colours on paper."
  :mode light :family solarized :style flat
  ((bg "#fdf6e3") (bg-alt "#eee8d5") (fg "#657b83") (fg-dim "#93a1a1")
   (fg-light "#586e75") (fg-bright "#073642") (muted "#93a1a1") (accent "#268bd2")
   (selection "#eee8d5") (red "#dc322f") (green "#859900") (yellow "#b58900")
   (blue "#268bd2") (magenta "#d33682") (cyan "#2aa198") (orange "#cb4b16")
   (keyword "#859900") (function "#268bd2") (type "#b58900") (constant "#2aa198")
   (string "#2aa198") (variable "#268bd2") (builtin "#cb4b16") (comment "#93a1a1")
   (ansi-0 "#073642") (ansi-7 "#eee8d5") (ansi-8 "#002b36") (ansi-9 "#cb4b16")
   (ansi-10 "#586e75") (ansi-11 "#657b83") (ansi-12 "#839496") (ansi-13 "#6c71c4")
   (ansi-14 "#93a1a1") (ansi-15 "#fdf6e3")))

;;; solarized-light-theme.el ends here
