;;; gruvbox-light-theme.el --- Gruvbox, light  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>
;; Palette: Gruvbox by Pavel Pertsev, MIT, https://github.com/morhetz/gruvbox
;; (ef8864bb42bf, colors/gruvbox.vim).  Colour values only: the face mapping is Romacs's.

;;; Code:

(require 'romacs-theme)

(def-romacs-theme gruvbox-light "Gruvbox: retro warmth on paper."
  :mode light :family gruvbox :style flat
  ((bg "#fbf1c7") (bg-dark "#f2e5bc") (bg-darker "#ebdbb2") (bg-alt "#ebdbb2")
   (fg "#3c3836") (fg-dim "#7c6f64") (fg-light "#504945") (fg-bright "#282828")
   (muted "#928374") (accent "#b57614") (selection "#d5c4a1") (red "#9d0006")
   (green "#79740e") (yellow "#b57614") (blue "#076678") (magenta "#8f3f71")
   (cyan "#427b58") (orange "#af3a03") (keyword "#9d0006") (function "#79740e")
   (type "#b57614") (constant "#8f3f71") (string "#79740e") (variable "#076678")
   (builtin "#af3a03") (comment "#928374") (ansi-1 "#cc241d") (ansi-2 "#98971a")
   (ansi-3 "#d79921") (ansi-4 "#458588") (ansi-5 "#b16286") (ansi-6 "#689d6a")
   (ansi-7 "#7c6f64") (ansi-8 "#928374") (ansi-9 "#9d0006") (ansi-10 "#79740e")
   (ansi-11 "#b57614") (ansi-12 "#076678") (ansi-13 "#8f3f71") (ansi-14 "#427b58")
   (ansi-15 "#3c3836")))

;;; gruvbox-light-theme.el ends here
