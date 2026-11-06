;;; horizon-theme.el --- Horizon  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: doom-themes' doom-horizon by karetsu, MIT, https://github.com/doomemacs/themes
;; (d114523c4c43, themes/doom-horizon-theme.el; after https://github.com/aodhneine/horizon-theme.el).
;; Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme horizon "Horizon, after doom-themes."
  :mode dark :family horizon :style flat
  ((bg "#232530") (bg-dark "#1c1e26") (fg "#c7c9cb") (fg-dim "#6a6a6a")
   (accent "#e95678") (selection "#2e2f3a") (red "#e95678") (green "#09f7a0")
   (yellow "#fab795") (blue "#21bfc2") (magenta "#6c6f93") (cyan "#59e3e3")
   (orange "#f09383") (keyword "#b877db") (function "#87ceeb") (type "#87ceeb")
   (constant "#f09383") (string "#fab795") (variable "#e95678") (builtin "#b877db")
   (comment "#4e5059") (doc "#fab795") (number "#f09383")))

;;; horizon-theme.el ends here
