;;; romacs-gui-loadup.el --- The interactive session after the shared loadup  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later

;;; Commentary:
;; --loadup runs romacs-loadup-bootstrap.el, the GNU-loadup chain the batch
;; package runner preloads, as that runner does: in batch, as GNU's dump
;; preloads.  The bundled packages' autoloads load next, then this file:
;; the steps of startup.el's `command-line' an interactive session on a
;; hosted display takes and batch does not.  Each is commented where it
;; happens.  The GUI's own Romacs layers and the user's init follow.

;;; Code:

;; The session is interactive from here: GNU's C sets `noninteractive'
;; from the command line, and only batch keeps it.
(setq noninteractive nil)

;; startup.el normal-top-level (startup.el:591): the per-user directory.
;; Romacs's is $XDG_CONFIG_HOME/romacs/, else ~/.config/romacs/, the
;; directory of the init file the GUI loads (user_init.rs); loading that
;; init sets the same value.
(setq user-emacs-directory
      (let ((config (getenv-internal "XDG_CONFIG_HOME")))
        (file-name-as-directory
         (expand-file-name "romacs" (if (and config (file-name-absolute-p config))
                                        config
                                      "~/.config")))))

;; startup.el command-line (startup.el:1600, 1620): window-system
;; initialization and `frame-initialize' put the session on its display.
;; Romacs's display is the record romacs-frame.el reads (VM2-DISPLAY1);
;; the chain ran on the headless one, batch's.  The host's window is
;; graphical, mouse-driven, with selections, and shows any 24-bit color.
(setq romacs-display
      `(:graphic t :mouse t :selections t :symbol-keys t
        :screens 1 :pixel-width ,(frame-pixel-width) :pixel-height ,(frame-pixel-height)
        :mm-width nil :mm-height nil
        :backing-store not-useful :save-under not-useful
        :planes 24 :color-cells 16777216 :visual-class true-color
        :color t :grayscale nil :display-type color :background-mode dark
        :palette nil))
;; startup.el command-line (startup.el:1479): the delayed settings take
;; their standard values in this session's context, as GNU computes them at
;; startup: `global-font-lock-mode', `abbrev-file-name',
;; `auto-save-list-file-prefix' and the rest of the list the chain computed
;; for batch.  GNU computes them on its initial terminal, which in a UTF-8
;; locale shows what this display shows (`mode-line-collapse-minor-modes-to');
;; the headless record does not, so they follow the display record.
(let ((user-emacs-directory-warning nil))
  (mapc #'custom-reevaluate-setting romacs-loadup-delayed-init-variables))
;; faces.el face-set-after-frame-default, by way of `frame-initialize':
;; the frame's faces for this display.  A face declared before the chain
;; (the Rum palette, the launch font) keeps its own spec.
(frame-set-background-mode (selected-frame))

(provide 'romacs-gui-loadup)
;;; romacs-gui-loadup.el ends here
