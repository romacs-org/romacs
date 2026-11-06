;;; catppuccin-mocha-theme.el --- Catppuccin Mocha  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Catppuccin, MIT, https://github.com/catppuccin/palette
;; (07d02aa110ef, palette.json (1.8.0)).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme catppuccin-mocha "Catppuccin Mocha: soft pastels on dark."
  :mode dark :family catppuccin :style flat
  ((bg "#1e1e2e") (bg-dark "#181825") (bg-darker "#11111b") (bg-alt "#313244")
   (fg "#cdd6f4") (fg-dim "#a6adc8") (fg-light "#bac2de") (muted "#7f849c")
   (accent "#cba6f7") (selection "#45475a") (red "#f38ba8") (green "#a6e3a1")
   (yellow "#f9e2af") (blue "#89b4fa") (magenta "#f5c2e7") (cyan "#94e2d5")
   (orange "#fab387") (keyword "#cba6f7") (function "#89b4fa") (type "#f9e2af")
   (constant "#fab387") (string "#a6e3a1") (variable "#cdd6f4") (builtin "#f38ba8")
   (comment "#9399b2") (cursor "#f5e0dc")))

;;; catppuccin-mocha-theme.el ends here
