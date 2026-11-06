;;; dracula-theme.el --- Dracula  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Dracula, MIT, https://github.com/dracula/visual-studio-code
;; (b50e62e32090, src/dracula.yml).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme dracula "Dracula: vivid colours on dark."
  :mode dark :family dracula :style flat
  ((bg "#282a36") (bg-dark "#21222c") (bg-darker "#191a21") (bg-alt "#343746")
   (fg "#f8f8f2") (fg-dim "#6272a4") (muted "#6272a4") (accent "#bd93f9")
   (selection "#44475a") (red "#ff5555") (green "#50fa7b") (yellow "#f1fa8c")
   (blue "#bd93f9") (magenta "#ff79c6") (cyan "#8be9fd") (orange "#ffb86c")
   (bright-red "#ff6e6e") (bright-green "#69ff94") (bright-yellow "#ffffa5")
   (bright-blue "#d6acff") (bright-magenta "#ff92df") (bright-cyan "#a4ffff")
   (keyword "#ff79c6") (function "#50fa7b") (type "#8be9fd") (constant "#bd93f9")
   (string "#f1fa8c") (variable "#f8f8f2") (builtin "#8be9fd") (comment "#6272a4")
   (ansi-0 "#21222c") (ansi-8 "#6272a4") (ansi-15 "#ffffff")))

;;; dracula-theme.el ends here
