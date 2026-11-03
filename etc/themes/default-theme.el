;;; default-theme.el --- Romacs's stock theme: warm parchment on black  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The look a Romacs session starts in: the Rum palette, its base slots
;; and the choices Rum made beyond them pinned as slots, so its faces
;; realise as before (rlisp/romacs-theme.el).

;;; Code:

(require 'romacs-theme)

(romacs-theme-define
 'default "Romacs's stock look: warm parchment and restrained accents on black."
 :family 'default :mode 'dark
 :palette
 '((mode dark)
   (bg "#000000") (bg-dark "#0a0a0a") (bg-darker "#050403") (bg-alt "#111111")
   (fg "#d4c5a0") (fg-dim "#776d5c") (fg-light "#c8b890") (fg-bright "#f4e7c3")
   (accent "#d48a20") (selection "#3a2914") (muted "#756b59")
   (red "#ff5f59") (yellow "#d0bc00") (orange "#c76d52") (green "#9cbd6f")
   (cyan "#00d3d0") (blue "#5b9bd5") (magenta "#9b72cf") (brown "#8a5a32")
   (bright-red "#ff6b55") (bright-yellow "#e89e30") (bright-green "#44df44")
   (bright-cyan "#00eff0") (bright-blue "#9099d9") (bright-magenta "#b395db")
   ;; Rum's choices.
   (cursor "#9099d9")
   (bg-mode-line "#50269a") (bg-mode-line-inactive "#141414") (fg-mode-line-inactive "#756b59")
   (bg-region "#3e4451") (bg-highlight "#7030af") (bg-paren "#7030af")
   (bg-block "#2f0c3f") (bg-block-delimiter "#38325c")
   (prompt "#d2b580") (link "#4cc9f0")
   (keyword "#b395db") (function "#c28fea") (string "#aacb75") (comment "#756b59")
   (variable "#00a6a6") (constant "#72b6e4") (type "#00b8ad") (builtin "#d0a3e8")
   (fl-warning "#e89e30") (keybind "#72b6e4")
   ;; The chrome as it was mixed in sRGB: Oklab's mix toward black is darker.
   (border "#201e18") (glass "#11100d")
   (error "#c97054") (warning "#e89e30") (success "#00b8ad")))

;;; default-theme.el ends here
