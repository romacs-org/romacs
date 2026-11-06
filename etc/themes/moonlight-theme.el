;;; moonlight-theme.el --- Moonlight  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-moonlight by Brettm12345, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-moonlight-theme.el; after https://github.com/atomiks/moonlight-vscode-theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme moonlight "Moonlight, after doom-themes."
  :mode dark :family moonlight :style flat
  ((bg "#212337") (bg-dark "#191a2a") (fg "#c8d3f5") (accent "#82aaff")
   (selection "#383e5c") (red "#ff757f") (green "#c3e88d") (yellow "#ffc777")
   (blue "#82aaff") (magenta "#c099ff") (cyan "#b4f9f8") (orange "#ff995e")
   (keyword "#c099ff") (function "#82aaff") (type "#ffc777") (constant "#ff995e")
   (string "#c3e88d") (variable "#ff98a4") (builtin "#c099ff") (comment "#7a88cf")
   (doc "#9ba5db") (number "#ff995e")))

;;; moonlight-theme.el ends here
