;;; romacs-defaults.el --- Romacs's interactive defaults  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The built-in modes and options an interactive Romacs session turns on
;; where GNU leaves them off, so that a new user needs no configuration
;; and no package for them: paired delimiters, minibuffer history, recent
;; files and places kept across sessions, key hints, buffers that follow their files, relative line numbers in
;; programming buffers, tree-sitter modes for the languages whose
;; grammars Romacs ships, and backups and auto-saves kept out of the
;; directories being edited.  `romacs-defaults-settings' lists
;; them.  DIVERGENCES.md VM2-OOB-DEFAULTS.
;;
;; One switch, `romacs-defaults-mode', turns them all on or off.  An
;; interactive --loadup session turns it on before the user's init
;; (`romacs-defaults-start'), unless -Q, which keeps GNU's values; batch and
;; noninteractive runs never load this file.  While the mode is on, each
;; setting is its option's standard value, set through the option's :set
;; as `setopt' does, so the init overrides any one of them as it would a
;; GNU default and Custom shows it as the default.  Modes the first frame
;; does not need (minibuffer history kept across sessions,
;; auto-revert) turn on at the first idle moment or before
;; the first command, unless the init turned them on or off itself, so a
;; launch paints and answers first.  `(romacs-defaults-mode -1)' in the
;; init gives every option its GNU standard value back, unless the init
;; already gave the option a value of its own.
;;
;; This file only defines.  The GUI's world holds it with the libraries
;; the settings need (startup.rs `install_session_definitions'), so a
;; launch loads none of them; the options those libraries compute from
;; the per-user directory are computed again in each session, -Q's too
;; (`romacs-defaults-start').

;;; Code:

(require 'elec-pair)
(require 'savehist)
(require 'saveplace)
(require 'recentf)
(require 'autorevert)
(require 'which-key)
(require 'imenu)
(require 'use-package-core)
(require 'display-line-numbers)
(require 'treesit)

(defconst romacs-defaults--session-options '(savehist-file save-place-file recentf-save-file)
  "Options of the libraries above whose standard value names a per-user file.")

(defgroup romacs-defaults nil
  "Romacs's interactive defaults."
  :group 'environment)

(defconst romacs-defaults-settings
  `((electric-pair-mode t)
    (savehist-mode t idle)
    ;; Before the init: a file the command line visits gets its place.
    (save-place-mode t)
    ;; Recent files are cleaned up when idle, not at startup, which would
    ;; check every listed file and say so in a message.
    (recentf-auto-cleanup 300)
    (recentf-mode t)
    (global-auto-revert-mode t idle)
    (column-number-mode t)
    ;; Programming buffers number their lines (`romacs-defaults--line-numbers'),
    ;; counted from point, the current line by its own number.
    (display-line-numbers-type 'relative)
    ;; Key hints in a panel over the frame, which resizes no window
    ;; (rlisp/romacs-popup.el), where the window system has layers.
    ,@(and (fboundp 'romacs-popup-which-key-show)
           '((which-key-popup-type 'custom)
             (which-key-custom-show-popup-function #'romacs-popup-which-key-show)
             (which-key-custom-hide-popup-function #'romacs-popup-which-key-hide)
             (which-key-custom-popup-max-dimensions-function #'romacs-popup-which-key-max-dimensions)))
    (which-key-mode t)
    (backup-directory-alist
     (list (cons "." (expand-file-name "backups/" user-emacs-directory))))
    ;; GNU's transforms first: a remote file's auto-save stays local, in
    ;; `temporary-file-directory'.
    (auto-save-file-name-transforms
     (append (eval (car (car (get 'auto-save-file-name-transforms 'romacs-defaults-gnu))) t)
             (list (list ".*" (expand-file-name "auto-save/" user-emacs-directory) t))))
    (save-interprogram-paste-before-kill t)
    (kill-do-not-save-duplicates t)
    (read-extended-command-predicate #'command-completion-default-include-p)
    ;; Tree-sitter modes in place of the regex modes, for the languages
    ;; whose grammars Romacs ships (`romacs-treesit-bundled-grammars'):
    ;; GNU's :set remaps each mode to its ts mode.  markdown-ts-mode is
    ;; left out until its cost per key is measured within budget.
    (treesit-enabled-modes
     '(bash-ts-mode c-ts-mode c++-ts-mode c-or-c++-ts-mode css-ts-mode go-ts-mode
       js-ts-mode json-ts-mode mhtml-ts-mode python-ts-mode rust-ts-mode
       toml-ts-mode tsx-ts-mode typescript-ts-mode yaml-ts-mode))
    (imenu-auto-rescan t)
    (imenu-flatten 'prefix)
    ;; `use-package' forms in a Lisp buffer's imenu, under "Packages".
    (use-package-enable-imenu-support t))
  "Romacs's defaults, each (OPTION FORM [idle]): FORM is OPTION's standard value.
They are set in this order while `romacs-defaults-mode' is on.  An `idle'
setting, a mode the first frame does not need, is set at the session's
first idle moment or before its first command, whichever comes first,
unless the user's init turned that mode on or off itself
\(`romacs-defaults--set-pending').  Only a mode whose turning off is
harmless before it was ever on can wait: turning `recentf-mode' off saves
its list, and `which-key-mode' restores what it saved when turned on.")

(defvar romacs-defaults--pending nil
  "The `idle' settings not set yet, each (OPTION FORM NOTE), last first.
NOTE, on OPTION's mode hook meanwhile, records that the mode was toggled.")

(defun romacs-defaults--set (option form)
  "Set OPTION to the value of FORM through OPTION's :set, as `setopt' does."
  (funcall (or (get option 'custom-set) #'set-default) option (eval form t)))

(defun romacs-defaults--set-ours (option form)
  "Set OPTION to FORM's value as Romacs's default, which the switch undoes."
  (romacs-defaults--set option form)
  (put option 'romacs-defaults-set t))

(defun romacs-defaults--defer (option form)
  "Set OPTION to FORM's value later, unless its mode is toggled meanwhile."
  (let ((note (lambda () (put option 'romacs-defaults-toggled t))))
    (add-hook (intern (format "%s-hook" option)) note)
    (push (list option form note) romacs-defaults--pending)))

(defun romacs-defaults--apply (settings)
  "Make each (OPTION FORM [idle]) of SETTINGS OPTION's standard value, and set it,
an `idle' one later.  OPTION's own standard value is kept, as its
`romacs-defaults-gnu' property."
  (dolist (setting settings)
    (let ((option (car setting)) (form (cadr setting)))
      (unless (get option 'romacs-defaults-gnu)
        (put option 'romacs-defaults-gnu
             (list (or (get option 'standard-value)
                       (list (custom-quote (default-value option)))))))
      (put option 'standard-value (list form))
      (if (eq (nth 2 setting) 'idle)
          (romacs-defaults--defer option form)
        (romacs-defaults--set-ours option form))))
  (when romacs-defaults--pending
    (add-hook 'pre-command-hook #'romacs-defaults--set-pending)
    (run-with-idle-timer 0 nil #'romacs-defaults--set-pending)))

(defun romacs-defaults--forget-pending ()
  "Set no pending setting: leave their modes' hooks and the triggers."
  (remove-hook 'pre-command-hook #'romacs-defaults--set-pending)
  (dolist (entry romacs-defaults--pending)
    (remove-hook (intern (format "%s-hook" (car entry))) (nth 2 entry))
    (put (car entry) 'romacs-defaults-toggled nil))
  (setq romacs-defaults--pending nil))

(defun romacs-defaults--set-pending ()
  "Set the pending `idle' settings whose modes the user left alone, once.
Run at the first idle moment or before the first command."
  (let ((pending (mapcar (lambda (entry) (cons (get (car entry) 'romacs-defaults-toggled) entry))
                         (reverse romacs-defaults--pending))))
    (romacs-defaults--forget-pending)
    (dolist (toggled-entry pending)
      (unless (or (car toggled-entry) (not romacs-defaults-mode))
        (romacs-defaults--set-ours (nth 1 toggled-entry) (nth 2 toggled-entry))))))

(defun romacs-defaults--restore (settings)
  "Give each option of SETTINGS its own standard value back, in reverse order.
An option this mode did not set, or whose value is no longer the
setting's, keeps its value."
  (romacs-defaults--forget-pending)
  (dolist (setting (reverse settings))
    (let* ((option (car setting))
           (gnu (car (get option 'romacs-defaults-gnu)))
           (ours (and gnu (get option 'romacs-defaults-set)
                      (equal (default-value option) (eval (cadr setting) t)))))
      (when gnu
        (put option 'standard-value gnu)
        (put option 'romacs-defaults-gnu nil)
        (put option 'romacs-defaults-set nil)
        (when ours (romacs-defaults--set option (car gnu)))))))

(defvar-local romacs-defaults--numbered nil
  "Whether `romacs-defaults--line-numbers' turned this buffer's numbers on.")

(defun romacs-defaults--line-numbers ()
  "Number a programming buffer's lines, on `prog-mode-hook' while the mode is on."
  (unless display-line-numbers-mode
    (display-line-numbers-mode 1)
    (setq romacs-defaults--numbered t)))

(defun romacs-defaults--line-numbers-off ()
  "Leave `prog-mode-hook', and turn off the numbers it turned on."
  (remove-hook 'prog-mode-hook #'romacs-defaults--line-numbers)
  (dolist (buffer (buffer-list))
    (with-current-buffer buffer
      (when romacs-defaults--numbered
        (setq romacs-defaults--numbered nil)
        (display-line-numbers-mode -1)))))

(defun romacs-defaults--ensure-directories ()
  "Make the auto-save directory the setting names; GNU makes backups' itself."
  (ignore-error file-error
    (make-directory (expand-file-name "auto-save/" user-emacs-directory) t)))

(define-minor-mode romacs-defaults-mode
  "Toggle Romacs's interactive defaults, `romacs-defaults-settings'.
On, each setting is its option's standard value and the option has it.
Settings for modes the first frame does not need wait for the first idle
moment or command.  Off, each option has its GNU standard value again,
unless it was given another value while the mode was on.  An interactive session turns this
on before the user's init, unless started with -Q."
  :global t
  :group 'romacs-defaults
  (if romacs-defaults-mode
      (progn (romacs-defaults--apply romacs-defaults-settings)
             (add-hook 'prog-mode-hook #'romacs-defaults--line-numbers)
             (romacs-defaults--ensure-directories))
    (romacs-defaults--line-numbers-off)
    (romacs-defaults--restore romacs-defaults-settings)))

(defun romacs-defaults-start (quick)
  "Start this session's defaults; the GUI calls this before the user's init.
The per-user files of the libraries above follow this session's
`user-emacs-directory', as GNU computes them when it loads those libraries
in the session.  Unless QUICK (-Q), `romacs-defaults-mode' is on, as the
session's standard state."
  (mapc #'custom-reevaluate-setting romacs-defaults--session-options)
  (unless quick
    (put 'romacs-defaults-mode 'standard-value '(t))
    (romacs-defaults-mode 1)))

(provide 'romacs-defaults)
;;; romacs-defaults.el ends here
