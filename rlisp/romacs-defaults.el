;;; romacs-defaults.el --- Romacs's interactive defaults  -*- lexical-binding: t -*-
;; SPDX-License-Identifier: GPL-3.0-or-later
;; Maintainer: Thanos Apollo <public@thanosapollo.org>

;;; Commentary:
;; The built-in modes and options an interactive Romacs session turns on
;; where GNU leaves them off, so that a new user needs no configuration
;; and no package for them: paired delimiters, minibuffer history, recent
;; files and places kept across sessions, repeatable key sequences, key
;; hints, buffers that follow their files, and backups and auto-saves kept
;; out of the directories being edited.  `romacs-defaults-settings' lists
;; them.  DIVERGENCES.md VM2-OOB-DEFAULTS.
;;
;; One switch, `romacs-defaults-mode', turns them all on or off.  An
;; interactive --loadup session turns it on before the user's init
;; (`romacs-defaults-start'), unless -Q, which keeps GNU's values; batch and
;; noninteractive runs never load this file.  While the mode is on, each
;; setting is its option's standard value, set through the option's :set
;; as `setopt' does, so the init overrides any one of them as it would a
;; GNU default and Custom shows it as the default.  `(romacs-defaults-mode
;; -1)' in the init gives every option its GNU standard value back, unless
;; the init already gave the option a value of its own.
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
(require 'repeat)
(require 'autorevert)
(require 'which-key)
(require 'imenu)

(defconst romacs-defaults--session-options '(savehist-file save-place-file recentf-save-file)
  "Options of the libraries above whose standard value names a per-user file.")

(defgroup romacs-defaults nil
  "Romacs's interactive defaults."
  :group 'environment)

(defconst romacs-defaults-settings
  `((electric-pair-mode t)
    (savehist-mode t)
    (save-place-mode t)
    ;; Recent files are cleaned up when idle, not at startup, which would
    ;; check every listed file and say so in a message.
    (recentf-auto-cleanup 300)
    (recentf-mode t)
    (repeat-mode t)
    (global-auto-revert-mode t)
    (column-number-mode t)
    ;; Key hints in a layer over the frame, which resizes no window
    ;; (rlisp/romacs-layer.el), where the window system has layers.
    ,@(and (fboundp 'romacs-layer-which-key-show)
           '((which-key-popup-type 'custom)
             (which-key-custom-show-popup-function #'romacs-layer-which-key-show)
             (which-key-custom-hide-popup-function #'romacs-layer-which-key-hide)
             (which-key-custom-popup-max-dimensions-function #'romacs-layer-which-key-max-dimensions)))
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
    (imenu-auto-rescan t)
    (imenu-flatten 'prefix))
  "Romacs's defaults, each (OPTION FORM): FORM is OPTION's standard value.
They are set in this order while `romacs-defaults-mode' is on.")

(defun romacs-defaults--set (option form)
  "Set OPTION to the value of FORM through OPTION's :set, as `setopt' does."
  (funcall (or (get option 'custom-set) #'set-default) option (eval form t)))

(defun romacs-defaults--apply (settings)
  "Make each (OPTION FORM) of SETTINGS OPTION's standard value, and set it.
OPTION's own standard value is kept, as its `romacs-defaults-gnu' property."
  (dolist (setting settings)
    (let ((option (car setting)) (form (cadr setting)))
      (unless (get option 'romacs-defaults-gnu)
        (put option 'romacs-defaults-gnu
             (list (or (get option 'standard-value)
                       (list (custom-quote (default-value option)))))))
      (put option 'standard-value (list form))
      (romacs-defaults--set option form))))

(defun romacs-defaults--restore (settings)
  "Give each option of SETTINGS its own standard value back, in reverse order.
An option whose value is no longer the setting's keeps that value."
  (dolist (setting (reverse settings))
    (let* ((option (car setting))
           (gnu (car (get option 'romacs-defaults-gnu)))
           (ours (and gnu (equal (default-value option) (eval (cadr setting) t)))))
      (when gnu
        (put option 'standard-value gnu)
        (put option 'romacs-defaults-gnu nil)
        (when ours (romacs-defaults--set option (car gnu)))))))

(defun romacs-defaults--ensure-directories ()
  "Make the auto-save directory the setting names; GNU makes backups' itself."
  (ignore-error file-error
    (make-directory (expand-file-name "auto-save/" user-emacs-directory) t)))

(define-minor-mode romacs-defaults-mode
  "Toggle Romacs's interactive defaults, `romacs-defaults-settings'.
On, each setting is its option's standard value and the option has it.
Off, each option has its GNU standard value again, unless it was given
another value while the mode was on.  An interactive session turns this
on before the user's init, unless started with -Q."
  :global t
  :group 'romacs-defaults
  (if romacs-defaults-mode
      (progn (romacs-defaults--apply romacs-defaults-settings)
             (romacs-defaults--ensure-directories))
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
