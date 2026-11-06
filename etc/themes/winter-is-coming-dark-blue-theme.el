;;; winter-is-coming-dark-blue-theme.el --- Winter Is Coming Dark Blue  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-winter-is-coming-dark-blue by Philip Arvidsson, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-winter-is-coming-dark-blue-theme.el; after https://github.com/johnpapa/vscode-winteriscoming).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme winter-is-coming-dark-blue "Winter Is Coming Dark Blue, after doom-themes."
  :mode dark :family winter-is-coming :style flat
  ((bg "#011627") (bg-dark "#062e5a") (fg "#a7dbf7") (fg-dim "#5c7485")
   (accent "#4373c2") (selection "#2b4d72") (red "#ef5350") (green "#6bff81")
   (yellow "#f7ecb5") (blue "#4373c2") (magenta "#d29ffc") (cyan "#00bff9")
   (orange "#ffca28") (keyword "#00bff9") (function "#82aaff") (type "#d29ffc")
   (constant "#82aaff") (string "#8dec95") (variable "#a7dbf7") (builtin "#00bff9")
   (comment "#0c4994") (doc "#4876ae") (number "#f7ecb5")))

;;; winter-is-coming-dark-blue-theme.el ends here
