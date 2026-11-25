;;; romacs-gui-loadup.el --- The interactive session after the shared loadup  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; --loadup runs romacs-loadup-bootstrap.el, the GNU-loadup chain the batch
;; package runner preloads, as that runner does: in batch, as GNU's dump
;; preloads.  The bundled packages' autoloads load next, then this file:
;; the steps of startup.el's `command-line' an interactive session on a
;; hosted display takes and batch does not.  Each is commented where it
;; happens.  The GUI's own Romacs layers and the user's init follow.

;;; Code:

;; The per-process steps of startup.el's `command-line' an interactive
;; session takes on its display.  They supersede the batch steps the chain
;; took (`romacs-batch-startup'): a VM restored from a world takes these
;; instead (`romacs-startup-function', `romacs-world-restart').
(defun romacs-gui--startup-ellipsis ()
  "Truncate with three dots: Iosevka paints `…' two cells wide, where
`string-width' counts one, so every column after it would shift."
  (setq truncate-string-ellipsis "..."))

(require 'romacs-init-load)

(defun romacs-gui-startup ()
  "Take the per-process startup steps of an interactive session on this display.
The session leaves batch and has *Messages*, finds its per-user directory,
its locale and its display, initialization begins, and the delayed
variables take their standard values in that context.  The user's init
follows; `romacs-gui-loadup-after-init' ends initialization."
  (unwind-protect
      (progn
        (romacs--startup-phase t)
        ;; The session is interactive from here: GNU's C sets `noninteractive'
        ;; from the command line, and only batch keeps it.
        (setq noninteractive nil)
        (romacs-gui--startup-ellipsis)
        ;; frame.el frame-initialize: a window session has no terminal frame.
        (setq terminal-frame nil)
        ;; emacs.c main (emacs.c:2213) logs an empty message, so every session
        ;; starts with *Messages*, which `kill-buffer' and `other-buffer' may
        ;; fall back on; normal-top-level (startup.el:736) puts it in
        ;; `messages-buffer-mode' with *scratch*'s directory.  Romacs's log
        ;; (lisp/vm/message.el) otherwise creates it at the first message.
        (let ((dir default-directory))
          (with-current-buffer (get-buffer-create messages-buffer-name)
            (messages-buffer-mode)
            (setq default-directory (or dir (expand-file-name "~/")))))
        ;; startup.el normal-top-level (startup.el:584): the command line is
        ;; processed once, so a later return to top level only says so.
        (setq command-line-processed t)
        ;; startup.el normal-top-level (startup.el:591): the per-user directory.
        ;; Romacs's is $XDG_CONFIG_HOME/romacs/, else ~/.config/romacs/, the
        ;; directory of the init file the GUI loads (user_init.rs); loading that
        ;; init sets the same value.
        (setq user-emacs-directory (romacs-init-directory))
        (romacs--startup-environment)
        ;; startup.el command-line (startup.el:1600, 1620): window-system
        ;; initialization and `frame-initialize' put the session on its display.
        ;; The host opened its display before this file loaded, so the frame is
        ;; a `pgtk' frame there (VM2-WIN5) and `initial-window-system' and
        ;; `window-system' name it, as GNU's C sets them; without a display
        ;; host they stay nil.  Romacs's display is the record romacs-frame.el
        ;; reads (VM2-DISPLAY1); the chain ran on the headless one, batch's.
        ;; The host's window is graphical, mouse-driven, with selections, and
        ;; shows any 24-bit color.
        (setq initial-window-system (window-system)
              window-system initial-window-system)
        ;; startup.el:1602: the display is initialized, so `make-frame'
        ;; does not initialize it again.
        (when initial-window-system
          (put initial-window-system 'window-system-initialized t))
        (setq romacs-display
              `(:graphic t :mouse t :selections t :symbol-keys t
                :screens 1 :pixel-width ,(frame-pixel-width) :pixel-height ,(frame-pixel-height)
                :mm-width nil :mm-height nil
                :backing-store not-useful :save-under not-useful
                :planes 24 :color-cells 16777216 :visual-class true-color
                :color t :grayscale nil :display-type color :background-mode dark
                :palette nil))
        ;; pgtk_term_init (pgtkterm.c:7082): a window-system terminal runs at
        ;; 19200 baud, so isearch shows no one-line slow-terminal window.
        (setq baud-rate 19200)
        ;; startup.el command-line (startup.el:1479): the delayed settings take
        ;; their standard values in this session's context, as GNU computes them
        ;; at startup: `global-font-lock-mode', `abbrev-file-name',
        ;; `auto-save-list-file-prefix' and the rest of the list the chain
        ;; computed for batch.  GNU computes them on its initial terminal, which
        ;; in a UTF-8 locale shows what this display shows
        ;; (`mode-line-collapse-minor-modes-to'); the headless record does not,
        ;; so they follow the display record.
        (let ((user-emacs-directory-warning nil))
          (mapc #'custom-reevaluate-setting romacs-loadup-delayed-init-variables))
        ;; A list an earlier Romacs kept in the home directory joins the
        ;; per-user directory that now holds it.
        (romacs--migrate-project-list (expand-file-name "projects.eld" "~") project-list-file)
        ;; faces.el x-create-frame-with-faces (faces.el:2243), by way of
        ;; `frame-initialize': the frame's display parameters, then every
        ;; face computed for this display, each new-frame default kept.
        ;; The world computed the faces on the headless display, where a
        ;; spec for a colour or graphic display (region's :extend, a
        ;; button's :box) never matches.
        (when initial-window-system
          ;; The keys' alternative meanings on a window system (common-win.el):
          ;; M-tab is M-TAB, M-return M-RET, as the Tab and Return keys are
          ;; function keys here.
          (x-setup-function-keys (selected-frame))
          (frame-set-background-mode (selected-frame) t)
          (face-set-after-frame-default (selected-frame))
          ;; x-win.el and pgtk-win.el's window-system initialization, and
          ;; fontset_from_font for the frame's font.
          (romacs--setup-fontsets))
        ;; The root frame's windows are drawn as panes, with a status bar,
        ;; on a window system, in the chrome colours of the faces this
        ;; display shows (romacs-chrome.el, ledger/divergences VM2-PANE-CHROME
        ;; and VM2-STATUS-BAR).
        (when (fboundp 'romacs-chrome-refresh) (romacs-chrome-refresh)))
    (romacs--startup-phase nil)))
(setq romacs-startup-function #'romacs-gui-startup)
(romacs-gui-startup)
;; faces.el face-set-after-frame-default, by way of `frame-initialize':
;; the frame's faces for this display.  A face declared before the chain
;; (the Rum palette, the launch font) keeps its own spec.
(frame-set-background-mode (selected-frame))

(declare-function romacs-startup-progress "animate.rs" (stage &optional label state))

(defvar romacs-progress-state nil
  "Each startup stage's latest state, as (STAGE :label LABEL . STATE), kept
by `romacs-startup-progress': STATE a plist of :status (`running', `done'
or `failed'), :done, :of, :name, :error and :seconds.  A restart's
supervisor reads it over the server's socket.")

;; A start's long steps (a restore's buffers, the agent sessions) show on
;; the launch screen, or after it in a card (romacs-progress-view.el), which
;; loads when the first step with a state reports.
(autoload 'romacs-progress-view-update "romacs-progress-view")

(defun romacs-gui-loadup--progress-view (on-screen)
  "Show the start's steps (`romacs-progress-view-update', ON-SCREEN as
there); `romacs-startup-progress' calls this.  A failure to show them is
said, and the step that reported goes on."
  (with-demoted-errors "Startup progress view: %S"
    (romacs-progress-view-update on-screen)))

(defun romacs-gui-loadup--init-loaded (file)
  "Tell the launch screen the init loaded FILE (`after-load-functions')."
  (romacs-startup-progress 'init (concat "Loading your init: " (file-name-base file))))

(defun romacs-gui-loadup-before-init ()
  "The GUI calls this just before the user's init: each library it loads
is a step of the launch screen's progress, until
`romacs-gui-loadup-after-init'."
  (add-hook 'after-load-functions #'romacs-gui-loadup--init-loaded))

;; The user's init is loaded, and an error that stops it said, by
;; lisp/session/romacs-init-load.el (required above).

(defun romacs-gui-loadup-after-init ()
  "What GNU's startup does once the init file has loaded, up to the files.
The GUI calls this after the user's init: the end of startup.el
`command-line' (startup.el:1717-1724), `after-init-hook',
`delayed-warnings-hook' and *scratch*'s `initial-major-mode', then the
start of `command-line-1' (startup.el:2736).  The GUI visits the file
arguments next with `romacs-gui-visit-file-noselect', then calls
`romacs-gui-loadup-command-line-1'."
  (remove-hook 'after-load-functions #'romacs-gui-loadup--init-loaded)
  (romacs--after-init)
  (display-startup-echo-area-message))

;; The command line's rest after the init (startup.el `command-line-1',
;; `normal-top-level'), which the GUI calls.
(require 'romacs-gui-command-line)

(provide 'romacs-gui-loadup)
;;; romacs-gui-loadup.el ends here
