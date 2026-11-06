;;; tomorrow-night-theme.el --- Tomorrow Night  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-tomorrow-night by Henrik Lissner, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-tomorrow-night-theme.el; after https://github.com/ChrisKempson/Tomorrow-Theme).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme tomorrow-night "Tomorrow Night, after doom-themes."
  :mode dark :family tomorrow :style flat
  ((bg "#1d1f21") (bg-dark "#161719") (fg "#c5c8c6") (accent "#81a2be")
   (selection "#333537") (red "#cc6666") (green "#b5bd68") (yellow "#f0c674")
   (blue "#81a2be") (magenta "#c9b4cf") (cyan "#8abeb7") (orange "#de935f")
   (keyword "#b294bb") (function "#81a2be") (type "#f0c674") (constant "#de935f")
   (string "#b5bd68") (variable "#cc6666") (builtin "#81a2be") (comment "#5a5b5a")
   (doc "#717171") (number "#de935f")))

;;; tomorrow-night-theme.el ends here
