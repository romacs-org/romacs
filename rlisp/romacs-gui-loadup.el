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

;; The per-process steps of startup.el's `command-line' an interactive
;; session takes on its display.  They supersede the batch steps the chain
;; took (`romacs-batch-startup'): a VM restored from a world takes these
;; instead (`romacs-startup-function', `romacs-world-restart').
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
        ;; emacs.c main (emacs.c:2213) logs an empty message, so every session
        ;; starts with *Messages*, which `kill-buffer' and `other-buffer' may
        ;; fall back on; normal-top-level (startup.el:736) puts it in
        ;; `messages-buffer-mode' with *scratch*'s directory.  Romacs's log
        ;; (rlisp/vm/message.el) otherwise creates it at the first message.
        (let ((dir default-directory))
          (with-current-buffer (get-buffer-create messages-buffer-name)
            (messages-buffer-mode)
            (setq default-directory (or dir (expand-file-name "~/")))))
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
        (setq romacs-display
              `(:graphic t :mouse t :selections t :symbol-keys t
                :screens 1 :pixel-width ,(frame-pixel-width) :pixel-height ,(frame-pixel-height)
                :mm-width nil :mm-height nil
                :backing-store not-useful :save-under not-useful
                :planes 24 :color-cells 16777216 :visual-class true-color
                :color t :grayscale nil :display-type color :background-mode dark
                :palette nil))
        ;; startup.el command-line (startup.el:1479): the delayed settings take
        ;; their standard values in this session's context, as GNU computes them
        ;; at startup: `global-font-lock-mode', `abbrev-file-name',
        ;; `auto-save-list-file-prefix' and the rest of the list the chain
        ;; computed for batch.  GNU computes them on its initial terminal, which
        ;; in a UTF-8 locale shows what this display shows
        ;; (`mode-line-collapse-minor-modes-to'); the headless record does not,
        ;; so they follow the display record.
        (let ((user-emacs-directory-warning nil))
          (mapc #'custom-reevaluate-setting romacs-loadup-delayed-init-variables)))
    (romacs--startup-phase nil)))
(setq romacs-startup-function #'romacs-gui-startup)
(romacs-gui-startup)
;; faces.el face-set-after-frame-default, by way of `frame-initialize':
;; the frame's faces for this display.  A face declared before the chain
;; (the Rum palette, the launch font) keeps its own spec.
(frame-set-background-mode (selected-frame))

(defun romacs-gui-loadup-after-init ()
  "What GNU's startup does once the init file has loaded, up to the files.
The GUI calls this after the user's init: the end of startup.el
`command-line' (startup.el:1717-1724), `after-init-hook',
`delayed-warnings-hook' and *scratch*'s `initial-major-mode', then the
start of `command-line-1' (startup.el:2736).  The GUI visits the file
arguments next with `romacs-gui-visit-file-noselect', then calls
`romacs-gui-loadup-command-line-1'."
  (setq after-init-time (current-time))
  (run-hooks 'after-init-hook 'delayed-warnings-hook)
  (when (get-buffer "*scratch*")
    (with-current-buffer "*scratch*"
      (when (eq major-mode 'fundamental-mode)
        (funcall initial-major-mode))))
  (display-startup-echo-area-message))

(defvar romacs-gui-loadup--old-face-font-rescale-alist face-font-rescale-alist
  "`face-font-rescale-alist' before the user's init, as `normal-top-level'
keeps it across `command-line' (startup.el:773).")

(defun romacs-gui-loadup-command-line-1 ()
  "The rest of `command-line-1' once the file arguments are visited
\(startup.el:3008-3125): *scratch* for an `initial-buffer-choice' of t,
the initial scratch message, the choice's buffer ahead of
`romacs-gui-displayable-buffers', and the first two of those shown.
Unless `inhibit-startup-screen', the choice, `noninteractive',
`daemonp' or `inhibit-x-resources' prevents it, the startup hooks, the
user's frame settings and `window-setup-hook' run, and the value is the
startup screen to show: `concise' beside the shown buffers, else `full'.
The GUI shows *Romacs* there with `romacs-gui-loadup-startup-screen'.
`romacs-gui-loadup-normal-top-level' follows."
  (when (eq initial-buffer-choice t)
    (get-scratch-buffer-create))
  (and initial-scratch-message
       (get-buffer "*scratch*")
       (with-current-buffer "*scratch*"
         (when (zerop (buffer-size))
           (insert (substitute-command-keys initial-scratch-message))
           (set-buffer-modified-p nil))))
  (let ((displayable-buffers romacs-gui-displayable-buffers))
    (setq romacs-gui-displayable-buffers nil)
    (when initial-buffer-choice
      (let ((buf
             (cond ((stringp initial-buffer-choice)
                    (find-file-noselect initial-buffer-choice))
                   ((functionp initial-buffer-choice)
                    (funcall initial-buffer-choice))
                   ((eq initial-buffer-choice t)
                    (get-scratch-buffer-create))
                   (t
                    (error "`initial-buffer-choice' must be a string, a function, or t")))))
        (unless (buffer-live-p buf)
          (error "Value returned by `initial-buffer-choice' is not a live buffer: %S" buf))
        (setq displayable-buffers (cons buf (delq buf displayable-buffers)))))
    (let ((displayable-buffers-len (length displayable-buffers))
          (old-face-font-rescale-alist face-font-rescale-alist))
      (when (> displayable-buffers-len 0)
        (switch-to-buffer (car displayable-buffers)))
      (cond
       ((= displayable-buffers-len 2)
        (switch-to-buffer-other-window (cadr displayable-buffers))
        (other-window -1))
       ((> displayable-buffers-len 2)
        (let ((bufs (reverse (cdr displayable-buffers))))
          (switch-to-buffer-other-window (pop bufs))
          (dolist (buf bufs)
            (switch-to-buffer buf nil t))
          (other-window -1))))
      (if (or inhibit-startup-screen
              initial-buffer-choice
              noninteractive
              (daemonp)
              inhibit-x-resources)
          (progn
            (when (and (> displayable-buffers-len 2)
                       (not noninteractive)
                       (not inhibit-startup-buffer-menu))
              (list-buffers))
            nil)
        (run-hooks 'emacs-startup-hook 'term-setup-hook)
        (when (and (display-multi-font-p)
                   (not (eq face-font-rescale-alist old-face-font-rescale-alist))
                   (assoc (font-xlfd-name (face-attribute 'default :font))
                          face-font-rescale-alist #'string-match-p))
          (set-face-attribute 'default nil :font (font-spec)))
        (when (fboundp 'frame-notice-user-settings)
          (frame-notice-user-settings))
        (run-hooks 'window-setup-hook)
        (setq inhibit-startup-hooks t)
        (if (> displayable-buffers-len 0) 'concise 'full)))))

(defun romacs-gui-loadup-startup-screen (buffer concise)
  "Show BUFFER, the filled *Romacs* startup screen, as startup.el
`fancy-startup-screen' shows *GNU Emacs* (startup.el:2269): if
CONCISE, in another window, fitted when that window is a split; else
in the selected one."
  (if concise
      (progn
        (display-buffer buffer)
        (let ((window (get-buffer-window buffer t)))
          (or (null window)
              (eq window (selected-window))
              (eq window (next-window window))
              (fit-window-to-buffer window))))
    (switch-to-buffer buffer)))

(defun romacs-gui-loadup-normal-top-level ()
  "What `normal-top-level' does after `command-line' (startup.el:808-849):
the startup hooks unless the startup screen ran them, the user's frame
settings on a frame that already exists, and `window-setup-hook' unless
an earlier hook inhibited it.  Romacs's one frame is the hosted window,
not a window-system initial frame, so the frame settings apply here as
on GNU's terminal frame."
  (unless inhibit-startup-hooks
    (run-hooks 'emacs-startup-hook 'term-setup-hook))
  (when (or frame-initial-frame
            (not (and initial-window-system
                      (not noninteractive)
                      (not (eq initial-window-system 'pc)))))
    (when (and (display-multi-font-p)
               (not (eq face-font-rescale-alist
                        romacs-gui-loadup--old-face-font-rescale-alist))
               (assoc (face-attribute 'default :font)
                      face-font-rescale-alist
                      #'startup--rescale-elt-match-p))
      (set-face-attribute 'default nil :font (font-spec)))
    (if (fboundp 'frame-notice-user-settings)
        (frame-notice-user-settings))
    (if (fboundp 'frame-set-background-mode)
        (frame-set-background-mode (selected-frame))))
  (if (fboundp 'font-menu-add-default)
      (font-menu-add-default))
  (unless inhibit-startup-hooks
    (run-hooks 'window-setup-hook)))

(provide 'romacs-gui-loadup)
;;; romacs-gui-loadup.el ends here
