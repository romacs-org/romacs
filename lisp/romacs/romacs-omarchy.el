;;; romacs-omarchy.el --- Romacs on the Omarchy desktop  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The home of Omarchy integration.  `omarchy-mode' follows Omarchy's theme:
;; Omarchy keeps its current theme's name, a counter that changes on every
;; switch and the theme itself under one directory, which the mode watches.
;; No hook is installed and nothing is written outside Emacs.
;;
;; Omarchy swaps the theme directory first and writes the counter and then
;; the name, so a state whose name is at least as new as its counter is a
;; finished theme; anything earlier waits for the name's own event.  The
;; follow itself waits for a boundary between commands.  An Omarchy theme maps to a Romacs theme of the same name
;; (`omarchy-NAME', or `omarchy-theme-map''s entry); one Romacs does not ship
;; is made from its colors.toml, so every Omarchy theme looks right.
;;
;; A theme the user loads by hand pauses following until the next Omarchy
;; theme change, which is an explicit act on the desktop.

;;; Code:

(require 'romacs-theme)

(defgroup omarchy nil
  "Romacs on the Omarchy desktop."
  :group 'environment)

(defcustom omarchy-state-directory "~/.local/state/omarchy/current/"
  "The directory holding Omarchy's current theme."
  :type 'directory :group 'omarchy)

(defcustom omarchy-theme-map '(("rum" . rum))
  "Omarchy theme names with a Romacs theme not called `omarchy-NAME',
as (NAME . THEME)."
  :type '(alist :key-type string :value-type symbol) :group 'omarchy)

(defcustom omarchy-follow-delay 0.2
  "Seconds after Omarchy's last write before following it."
  :type 'number :group 'omarchy)

(defvar omarchy-mode-paused nil
  "Non-nil while `omarchy-mode' is not following: a theme was loaded by hand.
The next Omarchy theme change clears it.")

(defvar omarchy--watch nil "The file notification descriptor.")
(defvar omarchy--timer nil "The pending follow.")
(defvar omarchy--seen nil "The (GENERATION . NAME) last followed.")
(defvar omarchy--following nil "Non-nil while the mode itself enables a theme.")
(defvar omarchy--pause-told nil "Non-nil once a pause was announced.")
(defvar omarchy--pending nil "Non-nil while a change waits for a boundary between commands.")

(defun omarchy-detected-p ()
  "Whether this system keeps an Omarchy theme state."
  (file-directory-p omarchy-state-directory))

(defun omarchy--read (file)
  "The text of FILE in `omarchy-state-directory', or nil."
  (let ((path (expand-file-name file omarchy-state-directory)))
    (when (file-readable-p path)
      (with-temp-buffer
        (insert-file-contents path)
        (string-trim (buffer-string))))))

(defun omarchy--written ()
  "When Omarchy last wrote its counter and name, as a list of two times."
  (mapcar (lambda (file)
            (file-attribute-modification-time (file-attributes (expand-file-name file omarchy-state-directory))))
          '("theme.generation" "theme.name")))

(defun omarchy-current ()
  "Omarchy's current theme as (GENERATION . NAME), or nil while a switch is
writing it: Omarchy writes the name last, so a name older than the
counter, or one just emptied, is the previous theme's, and files written
while they are read belong to no one state."
  (let* ((written (omarchy--written))
         (generation (omarchy--read "theme.generation"))
         (name (omarchy--read "theme.name")))
    (and name (not (equal name ""))
         (equal written (omarchy--written))
         (not (and (car written) (nth 1 written) (time-less-p (nth 1 written) (car written))))
         (cons generation name))))

(defun omarchy-romacs-theme (name)
  "The Romacs theme for Omarchy's theme NAME."
  (or (cdr (assoc name omarchy-theme-map)) (intern (concat "omarchy-" name))))

(defun omarchy--palette ()
  "The palette of Omarchy's current theme."
  (romacs-theme-read-palette (expand-file-name "theme/colors.toml" omarchy-state-directory)))

(defun omarchy--define (theme name palette)
  "Define THEME from PALETTE, Omarchy's theme NAME's, anew: a theme's
palette can change under one name."
  (put theme 'theme-settings nil)
  (romacs-theme-define theme (format "Omarchy's %s, from its palette." name)
                       :family theme :mode (romacs-theme--second (assq 'mode palette)) :palette palette))

(defun omarchy--disable-schemes ()
  "Disable every enabled colour scheme."
  (dolist (theme (copy-sequence custom-enabled-themes))
    (when (and (not (eq theme 'user)) (romacs-theme--colour-scheme-p theme))
      (disable-theme theme))))

(defun omarchy-follow ()
  "Show Omarchy's current theme, and record that it was followed.
What can fail comes first, so a failure leaves the theme showing, and a
palette read while Omarchy switched again is left to that switch."
  (interactive)
  (let ((current (omarchy-current)) (omarchy--following t))
    (when current
      (let* ((name (cdr current))
             (theme (omarchy-romacs-theme name))
             (shipped (assq theme romacs-theme-bundled))
             (palette (unless shipped (omarchy--palette))))
        (when palette (romacs-theme-resolve palette))
        (when (equal current (omarchy-current))
          (when (and shipped (not (custom-theme-p theme))) (load-theme theme t t))
          (omarchy--disable-schemes)
          (when palette (omarchy--define theme name palette))
          (enable-theme theme)
          (setq omarchy--seen current omarchy-mode-paused nil))))))

(defun omarchy--changed ()
  "Follow Omarchy if a finished theme differs from the one last followed."
  (setq omarchy--pending nil)
  (remove-hook 'post-command-hook #'omarchy--boundary)
  (let ((current (omarchy-current)))
    (when (and current (not (equal current omarchy--seen)))
      (condition-case err
          (omarchy-follow)
        (error (message "omarchy-mode: %s" (error-message-string err)))))))

(defun omarchy--boundary ()
  "From `post-command-hook': follow at a command's end, unless that command
ran inside another, as a key in the minibuffer does."
  (when (zerop (recursion-depth)) (omarchy--changed)))

(defun omarchy--at-rest-p ()
  "Whether no command is running: the command loop itself is waiting.
`sit-for' and process output inside a command run timers too, but only
the loop's own wait counts as idle."
  (and (current-idle-time) (zerop (recursion-depth))))

(defun omarchy--idle ()
  "Follow now if Emacs is at rest; a command's end follows otherwise."
  (when (and omarchy--pending (omarchy--at-rest-p)) (omarchy--changed)))

(defun omarchy--wait-for-boundary ()
  "The debounce's end: follow at the next boundary between commands."
  (setq omarchy--timer nil omarchy--pending t)
  (add-hook 'post-command-hook #'omarchy--boundary)
  (run-with-idle-timer 0 nil #'omarchy--idle))

(defun omarchy--event (event)
  "File notification EVENT: a write to the theme's name or counter restarts the wait."
  (when (member (file-name-nondirectory (format "%s" (nth 2 event))) '("theme.name" "theme.generation"))
    (when omarchy--timer (cancel-timer omarchy--timer))
    (setq omarchy--timer (run-with-timer omarchy-follow-delay nil #'omarchy--wait-for-boundary))))

(defun omarchy--note-theme (theme)
  "From `enable-theme-functions': a colour scheme THEME loaded by hand pauses following."
  (when (and (not omarchy--following) (not (eq theme 'user)) (romacs-theme--colour-scheme-p theme))
    (setq omarchy-mode-paused t)
    (unless omarchy--pause-told
      (setq omarchy--pause-told t)
      (message "omarchy-mode: following paused until the next Omarchy theme change"))))

(define-minor-mode omarchy-mode
  "Follow Omarchy's theme.
A theme loaded by hand pauses following until the next Omarchy theme
change, as `omarchy-mode-paused' shows; turning the mode off keeps Romacs
apart from the desktop."
  :global t :group 'omarchy
  :lighter (:eval (and omarchy-mode-paused " Omarchy-paused"))
  (when omarchy--watch (file-notify-rm-watch omarchy--watch))
  (when omarchy--timer (cancel-timer omarchy--timer))
  (setq omarchy--watch nil omarchy--timer nil omarchy--pause-told nil omarchy--pending nil)
  (remove-hook 'enable-theme-functions #'omarchy--note-theme)
  (remove-hook 'post-command-hook #'omarchy--boundary)
  (when omarchy-mode
    (require 'filenotify)
    (setq omarchy--watch (file-notify-add-watch (expand-file-name omarchy-state-directory) '(change)
                                                #'omarchy--event))
    (add-hook 'enable-theme-functions #'omarchy--note-theme)
    ;; An init that chose a theme keeps it, until the desktop's next change.
    (if (or romacs-theme--stock (called-interactively-p 'any))
        (omarchy--changed)
      (setq omarchy--seen (omarchy-current) omarchy-mode-paused t))))

(provide 'romacs-omarchy)
;;; romacs-omarchy.el ends here
