;;; catppuccin-latte-theme.el --- Catppuccin Latte  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Catppuccin, MIT, https://github.com/catppuccin/palette
;; (07d02aa110ef, palette.json (1.8.0)).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme catppuccin-latte "Catppuccin Latte: soft pastels on light."
  :mode light :family catppuccin :style flat
  ((bg "#eff1f5") (bg-dark "#e6e9ef") (bg-darker "#dce0e8") (bg-alt "#ccd0da")
   (fg "#4c4f69") (fg-dim "#6c6f85") (fg-light "#5c5f77") (muted "#8c8fa1")
   (accent "#8839ef") (selection "#bcc0cc") (red "#d20f39") (green "#40a02b")
   (yellow "#df8e1d") (blue "#1e66f5") (magenta "#ea76cb") (cyan "#179299")
   (orange "#fe640b") (keyword "#8839ef") (function "#1e66f5") (type "#df8e1d")
   (constant "#fe640b") (string "#40a02b") (variable "#4c4f69") (builtin "#d20f39")
   (comment "#7c7f93") (cursor "#dc8a78")))

;;; catppuccin-latte-theme.el ends here
