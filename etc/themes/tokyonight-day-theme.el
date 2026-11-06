;;; tokyonight-day-theme.el --- Tokyo Night, the day variant  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Tokyo Night by Folke Lemaitre, Apache-2.0, https://github.com/folke/tokyonight.nvim
;; (cdc07ac78467, extras/lua/tokyonight_day.lua).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme tokyonight-day "Tokyo Night by day: cool paper with deep accents."
  :mode light :family tokyonight :style flat
  ((bg "#e1e2e7") (bg-dark "#d0d5e3") (bg-darker "#c4c8da") (bg-alt "#c4c8da")
   (fg "#3760bf") (fg-dim "#68709a") (fg-light "#6172b0") (muted "#8990b3")
   (accent "#2e7de9") (selection "#b7c1e3") (red "#f52a65") (green "#587539")
   (yellow "#8c6c3e") (blue "#2e7de9") (magenta "#9854f1") (cyan "#007197")
   (orange "#b15c00") (bright-red "#ff4774") (bright-green "#5c8524")
   (bright-yellow "#a27629") (bright-blue "#358aff") (bright-magenta "#a463ff")
   (bright-cyan "#007ea8") (keyword "#7847bd") (function "#2e7de9") (type "#118c74")
   (constant "#b15c00") (string "#587539") (variable "#3760bf") (builtin "#118c74")
   (comment "#848cb5") (ansi-0 "#b4b5b9") (ansi-7 "#6172b0") (ansi-8 "#a1a6c5")
   (ansi-15 "#3760bf")))

;;; tokyonight-day-theme.el ends here
