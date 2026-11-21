;;; romacs-gui-command-line.el --- The interactive session's command line, after its init  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; What startup.el's `command-line-1' and `normal-top-level' do once the
;; user's init has run and the file arguments are visited: the buffers
;; shown, the startup screen and hooks, the user's frame settings.  The
;; GUI calls these (crates/romacs-vm-gui/src/startup.rs);
;; romacs-gui-loadup.el loads this library before the init, as
;; `normal-top-level' keeps `face-font-rescale-alist' across `command-line'.
;; A library of its own so a batch session can load and test it, where
;; romacs-gui-loadup.el takes a session's startup steps as it loads.

;;; Code:

;; The buffers the file arguments visited, which lisp/session/romacs-startup.el
;; defines and the GUI fills before it calls `romacs-gui-loadup-command-line-1'.
(defvar romacs-gui-displayable-buffers)

(defvar romacs-gui-loadup--old-face-font-rescale-alist face-font-rescale-alist
  "`face-font-rescale-alist' before the user's init, as `normal-top-level'
keeps it across `command-line' (startup.el:773).")

(defun romacs-gui-loadup--fill-scratch ()
  "Make *scratch* for an `initial-buffer-choice' of t, and give an empty
*scratch* `initial-scratch-message'."
  (when (eq initial-buffer-choice t)
    (get-scratch-buffer-create))
  (and initial-scratch-message
       (get-buffer "*scratch*")
       (with-current-buffer "*scratch*"
         (when (zerop (buffer-size))
           (insert (substitute-command-keys initial-scratch-message))
           (set-buffer-modified-p nil)))))

(defun romacs-gui-loadup--initial-choice ()
  "The live buffer `initial-buffer-choice' names, visited or made, or an error."
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
    buf))

(defun romacs-gui-loadup--show-buffers (buffers count)
  "Show BUFFERS, COUNT of them, as `command-line-1' does: the first in the
selected window, the second beside it, the rest visited in that window
before the second, last first."
  (when (> count 0)
    (switch-to-buffer (car buffers)))
  (cond
   ((= count 2)
    (switch-to-buffer-other-window (cadr buffers))
    (other-window -1))
   ((> count 2)
    (let ((bufs (reverse (cdr buffers))))
      (switch-to-buffer-other-window (pop bufs))
      (dolist (buf bufs)
        (switch-to-buffer buf nil t))
      (other-window -1)))))

(defun romacs-gui-loadup--startup-screen-p ()
  "Whether this session shows a startup screen: nothing inhibits it."
  (not (or inhibit-startup-screen
           initial-buffer-choice
           noninteractive
           (daemonp)
           inhibit-x-resources)))

(defun romacs-gui-loadup--startup-hooks (old-rescale)
  "Run the startup hooks as a startup screen's session does: the default
font made again when the hooks changed `face-font-rescale-alist' from
OLD-RESCALE to an entry for it, the user's frame settings, then
`window-setup-hook'; later ones are inhibited."
  (run-hooks 'emacs-startup-hook 'term-setup-hook)
  (when (and (display-multi-font-p)
             (not (eq face-font-rescale-alist old-rescale))
             (assoc (font-xlfd-name (face-attribute 'default :font))
                    face-font-rescale-alist #'string-match-p))
    (set-face-attribute 'default nil :font (font-spec)))
  (romacs-gui-loadup-notice-frame-settings)
  (run-hooks 'window-setup-hook)
  (setq inhibit-startup-hooks t))

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
  (romacs-gui-loadup--fill-scratch)
  (let ((buffers romacs-gui-displayable-buffers))
    (setq romacs-gui-displayable-buffers nil)
    (when initial-buffer-choice
      (let ((buf (romacs-gui-loadup--initial-choice)))
        (setq buffers (cons buf (delq buf buffers)))))
    (let ((count (length buffers)) (old-rescale face-font-rescale-alist))
      (romacs-gui-loadup--show-buffers buffers count)
      (if (not (romacs-gui-loadup--startup-screen-p))
          (progn
            (when (and (> count 2) (not noninteractive) (not inhibit-startup-buffer-menu))
              (list-buffers))
            nil)
        (romacs-gui-loadup--startup-hooks old-rescale)
        (if (> count 0) 'concise 'full)))))

(defun romacs-gui-loadup-notice-frame-settings ()
  "`frame-notice-user-settings' for Romacs's one frame.
On a window system GNU's `frame-initialize' (frame.el) makes the initial
frame from `initial-frame-alist' and `default-frame-alist' before the
init runs, and `frame-notice-user-settings' applies what the init
changed.  The host made Romacs's frame with no `frame-initial-frame', so
it first gets the alists as the init left them, the earlier alist
winning, as `frame-notice-user-settings' gives a frame without a window
system; then GNU's function runs."
  (when (fboundp 'frame-notice-user-settings)
    (when (and frame-notice-user-settings initial-window-system (null frame-initial-frame))
      (modify-frame-parameters
       nil (append initial-frame-alist
                   (cdr (assq initial-window-system
                              window-system-default-frame-alist))
                   default-frame-alist nil)))
    (frame-notice-user-settings)))

(defun romacs-gui-loadup-startup-buffer (name)
  "The startup screen, the buffer NAME, built if it is not live.
As `fancy-startup-screen' sets up *GNU Emacs* (startup.el:2269), it
starts in `command-line-default-directory' and takes GNU's
`splash-screen-keymap': typing does nothing, SPC and DEL scroll, and q
\(`exit-splash-screen') kills it.  It is the animated logo
\(lisp/ui/romacs-landing.el), whose keys (SPC, Home) come first, and
nothing else; without the logo, one title line."
  (or (get-buffer name)
      (with-current-buffer (get-buffer-create name)
        (setq default-directory command-line-default-directory)
        (use-local-map splash-screen-keymap)
        (if (fboundp 'romacs-landing)
            (progn (setq buffer-read-only t) (set-buffer-modified-p nil) (romacs-landing))
          (insert "Romacs\n")
          (setq buffer-read-only t)
          (set-buffer-modified-p nil))
        (goto-char (point-min))
        (current-buffer))))

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
which always exists, so the frame settings always apply, where GNU
skips them when it failed to make a window-system initial frame."
  (unless inhibit-startup-hooks
    (run-hooks 'emacs-startup-hook 'term-setup-hook))
  (when (and (display-multi-font-p)
             (not (eq face-font-rescale-alist
                      romacs-gui-loadup--old-face-font-rescale-alist))
             (assoc (face-attribute 'default :font)
                    face-font-rescale-alist
                    #'startup--rescale-elt-match-p))
    (set-face-attribute 'default nil :font (font-spec)))
  (romacs-gui-loadup-notice-frame-settings)
  (if (fboundp 'frame-set-background-mode)
      (frame-set-background-mode (selected-frame)))
  (if (fboundp 'font-menu-add-default)
      (font-menu-add-default))
  (unless inhibit-startup-hooks
    (run-hooks 'window-setup-hook)))

(provide 'romacs-gui-command-line)
;;; romacs-gui-command-line.el ends here
