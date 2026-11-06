;;; kanagawa-wave-theme.el --- Kanagawa Wave  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Kanagawa by Tommaso Laurenzi, MIT, https://github.com/rebelot/kanagawa.nvim
;; (bb85e4bfc8d8, lua/kanagawa/colors.lua, themes.lua).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme kanagawa-wave "Kanagawa Wave: muted ink and woodblock colours."
  :mode dark :family kanagawa :style flat
  ((bg "#1f1f28") (bg-dark "#1a1a22") (bg-darker "#16161d") (bg-alt "#2a2a37")
   (fg "#dcd7ba") (fg-dim "#727169") (fg-light "#c8c093") (muted "#54546d")
   (accent "#7e9cd8") (selection "#223249") (red "#e46876") (green "#98bb6c")
   (yellow "#e6c384") (blue "#7e9cd8") (magenta "#957fb8") (cyan "#7aa89f")
   (orange "#ffa066") (keyword "#957fb8") (function "#7e9cd8") (type "#7aa89f")
   (constant "#ffa066") (string "#98bb6c") (variable "#dcd7ba") (builtin "#e46876")
   (comment "#727169")))

;;; kanagawa-wave-theme.el ends here
