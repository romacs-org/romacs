;;; leet-theme.el --- Leet  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-1337 by ccmywish, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-1337-theme.el; after https://github.com/microsoft/vscode-themes/tree/main/1337).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme leet "Leet, after doom-themes."
  :mode dark :family leet :style flat
  ((bg "#191919") (bg-dark "#252526") (fg "#d4d4d4") (fg-dim "#777778")
   (accent "#ffffff") (selection "#282828") (red "#ff5e5e") (green "#468800")
   (yellow "#e9fdac") (blue "#8cdaff") (magenta "#c586c0") (cyan "#85ddff")
   (orange "#fc9354") (keyword "#ff5e5e") (function "#8cdaff") (type "#e9fdac")
   (constant "#fc9354") (string "#fbe3bf") (variable "#e9fdac") (builtin "#6699cc")
   (comment "#6d6d6d") (doc "#6d6d6d") (number "#fc9354")))

;;; leet-theme.el ends here
